//! CC Switch-owned storage. External applications keep their own home/configuration rules.
use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
const DATA_ROOT: &str = r"D:\Data\CCSwitch";
#[cfg(windows)]
const CACHE_ROOT: &str = r"D:\Cache\CCSwitch";
#[cfg(windows)]
static LEGACY_WEBVIEW_DIR: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn test_root() -> Option<PathBuf> {
    let value = std::env::var_os("CC_SWITCH_TEST_HOME")?;
    let value = value.to_string_lossy();
    let value = value.trim();
    (!value.is_empty()).then(|| PathBuf::from(value).join(".cc-switch"))
}

pub fn app_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root;
    }
    #[cfg(windows)]
    return PathBuf::from(DATA_ROOT).join("app");
    #[cfg(not(windows))]
    crate::config::get_home_dir().join(".cc-switch")
}

pub fn device_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root;
    }
    #[cfg(windows)]
    return PathBuf::from(DATA_ROOT).join("device");
    #[cfg(not(windows))]
    crate::config::get_home_dir().join(".cc-switch")
}

pub fn tauri_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root.join("tauri");
    }
    #[cfg(windows)]
    return PathBuf::from(DATA_ROOT).join("tauri");
    #[cfg(not(windows))]
    dirs::config_dir()
        .unwrap_or_else(device_dir)
        .join("com.ccswitch.desktop")
}

pub fn webview_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root.join("webview2");
    }
    #[cfg(windows)]
    return PathBuf::from(DATA_ROOT).join("webview2");
    #[cfg(not(windows))]
    dirs::data_local_dir()
        .unwrap_or_else(device_dir)
        .join("com.ccswitch.desktop")
}

pub fn cache_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root.join("cache");
    }
    #[cfg(windows)]
    return PathBuf::from(CACHE_ROOT);
    #[cfg(not(windows))]
    dirs::cache_dir()
        .unwrap_or_else(|| device_dir().join("cache"))
        .join("com.ccswitch.desktop")
}

pub fn temp_dir() -> PathBuf {
    cache_dir().join("tmp")
}

/// Validate selected CC storage without changing any external application's paths.
pub fn validate_data_dir(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use std::path::{Component, Prefix};

        let on_d = |value: &Path| {
            matches!(
                value.components().next(),
                Some(Component::Prefix(prefix))
                    if matches!(prefix.kind(), Prefix::Disk(b'D' | b'd') | Prefix::VerbatimDisk(b'D' | b'd'))
            )
        };
        if !path.is_absolute()
            || !on_d(path)
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "CC Switch 数据和缓存目录必须是 D 盘绝对路径：{}",
                    path.display()
                ),
            ));
        }
        for ancestor in path.ancestors() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) => {
                    if metadata.file_attributes() & 0x400 != 0 || !metadata.is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!(
                                "CC Switch 目录不能经过链接、重解析点或普通文件：{}",
                                ancestor.display()
                            ),
                        ));
                    }
                    if !on_d(&std::fs::canonicalize(ancestor)?) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("CC Switch 目录解析后离开 D 盘：{}", ancestor.display()),
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    #[cfg(not(windows))]
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "需要绝对数据目录",
        ));
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    validate_data_dir(path)?;
    std::fs::create_dir_all(path)?;
    validate_data_dir(path)
}

pub fn ensure_temp_dir() -> io::Result<PathBuf> {
    let path = temp_dir();
    ensure_directory(&path)?;
    Ok(path)
}

#[cfg(windows)]
fn ensure_private_directory(path: &Path) -> io::Result<()> {
    validate_data_dir(path)?;
    let existed = path.is_dir();
    ensure_directory(path)?;
    if existed {
        crate::data_permissions::validate_directory(path)
    } else {
        crate::data_permissions::secure_directory(path)
    }
}

#[cfg(windows)]
pub fn legacy_webview_dir() -> PathBuf {
    LEGACY_WEBVIEW_DIR
        .get_or_init(|| {
            std::env::var_os("WEBVIEW2_USER_DATA_FOLDER")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::data_local_dir()
                        .unwrap_or_else(crate::config::get_home_dir)
                        .join("com.ccswitch.desktop")
                })
        })
        .clone()
}

/// Runs before the runtime creates threads, plugins, windows or persistence.
pub fn prepare() -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::io::Write;
        if !Path::new(r"D:\").is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "D 盘不可用，CC Switch 无法启动",
            ));
        }
        // Capture the old profile location before enforcing this process's D profile.
        let _ = legacy_webview_dir();
        let data_root = test_root().unwrap_or_else(|| PathBuf::from(DATA_ROOT));
        for root in [data_root, cache_dir()] {
            ensure_private_directory(&root)?;
        }
        if let Ok(arguments) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            if [
                "--user-data-dir",
                "--disk-cache-dir",
                "--crash-dumps-dir",
                "--log-file",
                "--log-net-log",
                "--trace-startup-file",
            ]
            .iter()
            .any(|flag| arguments.contains(flag))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS 含目录覆盖，请移除该覆盖后重试",
                ));
            }
        }
        for path in [
            app_dir(),
            device_dir(),
            tauri_dir(),
            webview_dir(),
            cache_dir(),
            temp_dir(),
        ] {
            ensure_private_directory(&path)?;
            let probe = path.join(format!(".write-check-{}", std::process::id()));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)?;
            let result = file
                .write_all(b"CC Switch storage check")
                .and_then(|_| file.sync_all());
            drop(file);
            let cleanup = std::fs::remove_file(&probe);
            result?;
            cleanup?;
        }
        std::env::set_var("TEMP", temp_dir());
        std::env::set_var("TMP", temp_dir());
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", webview_dir());
    }
    Ok(())
}

pub fn report_startup_error(error: impl std::fmt::Display) {
    let message = format!("CC Switch 无法初始化 D 盘数据目录。\n\n{error}");
    eprintln!("{message}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "CC Switch".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_absolute_d_paths() {
        for path in [
            r"C:\Users\test\data",
            r"E:\data",
            r"D:relative",
            r"..\data",
            r"\\server\data",
        ] {
            assert!(validate_data_dir(Path::new(path)).is_err(), "{path}");
        }
        assert!(validate_data_dir(Path::new(r"D:\Data\CCSwitch\app")).is_ok());
        assert!(validate_data_dir(Path::new(r"D:\Data\..\outside")).is_err());
    }

    #[test]
    fn file_in_directory_chain_is_an_error() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("blocked");
        std::fs::write(&file, b"existing data").unwrap();
        assert!(ensure_directory(&file.join("child")).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"existing data");
    }
}
