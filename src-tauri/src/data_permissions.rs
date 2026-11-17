//! Access control for CC Switch-owned directories before private data is copied.

#[cfg(not(windows))]
pub fn secure_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub fn validate_directory(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub use windows::secure_directory;
#[cfg(all(windows, test))]
pub use windows::validate_directory;

#[cfg(windows)]
mod windows {
    #[cfg(test)]
    use std::collections::BTreeSet;
    use std::ffi::c_void;
    use std::io;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE,
    };
    #[cfg(test)]
    use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    #[cfg(test)]
    use windows_sys::Win32::Security::{
        GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        CONTAINER_INHERIT_ACE, INHERIT_ONLY_ACE, OBJECT_INHERIT_ACE, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    #[cfg(test)]
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    struct TokenHandle(HANDLE);

    impl Drop for TokenHandle {
        fn drop(&mut self) {
            // This handle is owned after a successful OpenProcessToken call.
            unsafe { CloseHandle(self.0) };
        }
    }

    struct LocalAllocation(*mut c_void);

    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SID strings and SDDL descriptors are allocated by LocalAlloc.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    fn sid_string(sid: PSID) -> io::Result<String> {
        let mut pointer = null_mut();
        let result = unsafe { ConvertSidToStringSidW(sid, &mut pointer) };
        let _allocation = LocalAllocation(pointer.cast());
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        // The conversion returns an allocated, null-terminated UTF-16 string.
        let mut length = 0;
        unsafe {
            while *pointer.add(length) != 0 {
                length += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(pointer, length))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        }
    }

    fn current_user_sid() -> io::Result<String> {
        let mut handle = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = TokenHandle(handle);
        let mut required = 0;
        let result =
            unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required) };
        if result == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
                return Err(error);
            }
        }
        if (required as usize) < size_of::<TOKEN_USER>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned an incomplete token user",
            ));
        }
        // TOKEN_USER contains a pointer. Word storage supplies the required alignment.
        let mut storage = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
        sid_string(user.User.Sid)
    }

    fn directory_path(path: &Path) -> io::Result<Vec<u16>> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CC Switch data root must be a directory without a reparse point",
            ));
        }
        let mut path_wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if path_wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CC Switch data root contains a null character",
            ));
        }
        path_wide.push(0);
        Ok(path_wide)
    }

    /// Initialize a newly created CC-owned root. Existing roots use validation.
    pub fn secure_directory(path: &Path) -> io::Result<()> {
        let path_wide = directory_path(path)?;
        let user = current_user_sid()?;
        let sddl = format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        let sddl_wide = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut descriptor = null_mut();
        let result = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl_wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        let _descriptor = LocalAllocation(descriptor);
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if present == 0 || dacl.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CC Switch data root requires an explicit access-control list",
            ));
        }
        // Only the DACL is changed. Ownership, primary group and SACL are retained.
        // This setter propagates inheritable ACEs to existing unprotected children.
        let error = unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null(),
            )
        };
        if error != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        Ok(())
    }

    #[cfg(test)]
    fn directory_acl(path: &[u16]) -> io::Result<(LocalAllocation, *mut ACL, u16)> {
        let mut descriptor = null_mut();
        let mut dacl = null_mut();
        let error = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        let allocation = LocalAllocation(descriptor);
        if error != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        let mut control = 0;
        let mut revision = 0;
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if dacl.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "CC Switch data root has an unrestricted DACL",
            ));
        }
        Ok((allocation, dacl, control))
    }

    #[cfg(test)]
    fn allowed_entries(dacl: *mut ACL) -> io::Result<Vec<(String, u32, u8)>> {
        let mut entries = Vec::new();
        // GetNamedSecurityInfoW returns a valid ACL and owns all its ACE storage.
        for index in 0..unsafe { (*dacl).AceCount } {
            let mut entry = null_mut();
            if unsafe { GetAce(dacl, index as u32, &mut entry) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let header = unsafe { &*entry.cast::<ACE_HEADER>() };
            if header.AceType != 0 || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "CC Switch data root has an unsupported access-control entry",
                ));
            }
            let entry = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
            let sid = std::ptr::addr_of!(entry.SidStart).cast_mut().cast();
            entries.push((sid_string(sid)?, entry.Mask, entry.Header.AceFlags));
        }
        Ok(entries)
    }

    /// Check an existing root without rewriting its ACL or its children's ACLs.
    #[cfg(test)]
    pub fn validate_directory(path: &Path) -> io::Result<()> {
        let path_wide = directory_path(path)?;
        let (_descriptor, dacl, control) = directory_acl(&path_wide)?;
        if control & SE_DACL_PROTECTED == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Existing CC Switch data root inherits access from its parent",
            ));
        }
        let trusted = BTreeSet::from([
            current_user_sid()?,
            "S-1-5-18".into(),
            "S-1-5-32-544".into(),
        ]);
        let mut full_control = BTreeSet::new();
        for (sid, mask, flags) in allowed_entries(dacl)? {
            if !trusted.contains(&sid) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Existing CC Switch data root grants access to another account",
                ));
            }
            let inheritance = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
            if mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS
                && flags as u32 & inheritance == inheritance
                && flags as u32 & INHERIT_ONLY_ACE == 0
            {
                full_control.insert(sid);
            }
        }
        if full_control != trusted {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Existing CC Switch data root lacks inherited full control for the trusted accounts"));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::Security::INHERITED_ACE;

        fn read_dacl(path: &Path) -> (u16, Vec<(String, u32, u8)>) {
            let path = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let (_descriptor, dacl, control) = directory_acl(&path).unwrap();
            (control, allowed_entries(dacl).unwrap())
        }

        #[test]
        fn owned_directory_has_protected_acl_and_children_inherit_only_trusted_accounts() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("owned");
            std::fs::create_dir(&root).unwrap();
            secure_directory(&root).unwrap();
            validate_directory(&root).unwrap();
            let fresh = root.join("fresh.txt");
            std::fs::write(&fresh, "synthetic fixture").unwrap();
            let expected = BTreeSet::from([
                current_user_sid().unwrap(),
                "S-1-5-18".into(),
                "S-1-5-32-544".into(),
            ]);
            let (control, entries) = read_dacl(&root);
            assert_ne!(control & SE_DACL_PROTECTED, 0);
            assert_eq!(
                entries
                    .iter()
                    .map(|entry| entry.0.clone())
                    .collect::<BTreeSet<_>>(),
                expected
            );
            for (_, mask, flags) in &entries {
                assert_eq!(*mask, FILE_ALL_ACCESS);
                assert_eq!(*flags as u32, OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE);
            }
            let (_, inherited) = read_dacl(&fresh);
            assert_eq!(
                inherited
                    .iter()
                    .map(|entry| entry.0.clone())
                    .collect::<BTreeSet<_>>(),
                expected
            );
            for (_, mask, flags) in inherited {
                assert_eq!(mask, FILE_ALL_ACCESS);
                assert_ne!(flags as u32 & INHERITED_ACE, 0);
            }
            validate_directory(&root).unwrap();
            assert_eq!(read_dacl(&root).1, entries);
        }

        #[test]
        fn existing_unprotected_directory_is_rejected_without_changing_its_acl() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("existing");
            std::fs::create_dir(&root).unwrap();
            let before = read_dacl(&root);
            assert_eq!(before.0 & SE_DACL_PROTECTED, 0);
            assert_eq!(
                validate_directory(&root).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(read_dacl(&root), before);
        }
    }
}
