//! CC Switch-owned storage. External applications keep their own home/configuration rules.
use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
static LEGACY_WEBVIEW_DIR: OnceLock<PathBuf> = OnceLock::new();
#[cfg(windows)]
static STORAGE_ROOTS: OnceLock<StorageRoots> = OnceLock::new();

#[cfg(windows)]
#[derive(Debug, Clone, Default)]
struct StorageOptions {
    data: Option<PathBuf>,
    cache: Option<PathBuf>,
}

#[cfg(windows)]
#[derive(Debug, Clone)]
struct StorageRoots {
    data: PathBuf,
    cache: PathBuf,
    explicit_data: bool,
    explicit_cache: bool,
    layered: bool,
}

#[cfg(windows)]
impl StorageRoots {
    fn app(&self) -> PathBuf {
        if self.layered {
            self.data.join("app")
        } else {
            self.data.clone()
        }
    }

    fn device(&self) -> PathBuf {
        if self.layered {
            self.data.join("device")
        } else {
            self.data.clone()
        }
    }
}

#[cfg(windows)]
fn has_existing_cc_data(dir: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    let mut found = false;
    for name in ["cc-switch.db", "config.json", "settings.json"] {
        match std::fs::symlink_metadata(dir.join(name)) {
            Ok(metadata) if metadata.is_file() && metadata.file_attributes() & 0x400 == 0 => {
                found = true;
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "已有数据文件不是普通文件",
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(found)
}

#[cfg(windows)]
fn has_direct_data_layout(data: &Path) -> io::Result<bool> {
    let direct = has_existing_cc_data(data)?;
    let marker = marker_layered_layout(data)?;
    let app_entries = directory_has_entries(&data.join("app"))?;
    let device_entries = directory_has_entries(&data.join("device"))?;
    let nested_entries = app_entries || device_entries || marker == Some(true);
    // A completed direct-layout marker identifies app/device-looking folders
    // as ordinary contents unless a second database/config actually appears.
    let nested = if marker == Some(false) {
        app_entries && has_existing_cc_data(&data.join("app"))?
    } else {
        nested_entries
    };
    let direct = direct || marker == Some(false);
    if direct && nested {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "指定目录同时含直存和分层的 CC Switch 数据，请明确选择其中一个目录",
        ));
    }
    if direct && is_marked_layered_app(data)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "指定的是已有分层数据的 app 子目录，请将数据目录参数指向其父数据根",
        ));
    }
    // A new root follows CC Switch's native layout; only evidence of a
    // previous local split selects the app/device subdirectories.
    Ok(!nested)
}

#[cfg(windows)]
fn directory_has_entries(path: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && metadata.file_attributes() & 0x400 == 0 => {
            Ok(std::fs::read_dir(path)?.next().transpose()?.is_some())
        }
        // A regular file named app/device can be unrelated direct-layout
        // content; it does not establish a separate data tree.
        Ok(metadata) if metadata.is_file() && metadata.file_attributes() & 0x400 == 0 => Ok(false),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "已有数据目录不是普通目录",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn marker_layered_layout(data: &Path) -> io::Result<Option<bool>> {
    use std::os::windows::fs::MetadataExt;
    let marker = data.join("tauri").join(".cc-switch-data-migration-v1.json");
    let metadata = match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() > 1024 * 1024
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "已有数据迁移标记无效",
        ));
    }
    let marker: serde_json::Value = serde_json::from_slice(&std::fs::read(marker)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "已有数据迁移标记无效"))?;
    let read_path = |key: &str| {
        marker
            .pointer(key)
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "已有数据迁移标记无效"))
    };
    let app = read_path("/targets/app")?;
    let device = read_path("/targets/device")?;
    if equivalent_windows_path(&app, &device) {
        return Ok(Some(false));
    }
    let names_match = app
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("app"))
        && device
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("device"));
    if names_match
        && app
            .parent()
            .zip(device.parent())
            .is_some_and(|(left, right)| equivalent_windows_path(left, right))
    {
        return Ok(Some(true));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "已有数据迁移标记布局不一致",
    ))
}

#[cfg(windows)]
fn is_marked_layered_app(data: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    let Some(parent) = data.parent() else {
        return Ok(false);
    };
    let marker = parent
        .join("tauri")
        .join(".cc-switch-data-migration-v1.json");
    let metadata = match std::fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || metadata.len() > 1024 * 1024
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "已有数据迁移标记无效",
        ));
    }
    let marker: serde_json::Value = serde_json::from_slice(&std::fs::read(marker)?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "已有数据迁移标记无效"))?;
    let marked = marker
        .pointer("/targets/app")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "已有数据迁移标记无效"))?;
    Ok(equivalent_windows_path(Path::new(marked), data))
}

#[cfg(windows)]
fn parse_storage_options(args: &[OsString]) -> io::Result<StorageOptions> {
    let mut options = StorageOptions::default();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (slot, value) = if arg == OsStr::new("--data-dir") {
            (&mut options.data, args.next().cloned())
        } else if arg == OsStr::new("--cache-dir") {
            (&mut options.cache, args.next().cloned())
        } else if let Some(value) = arg.to_str().and_then(|s| s.strip_prefix("--data-dir=")) {
            (&mut options.data, Some(OsString::from(value)))
        } else if let Some(value) = arg.to_str().and_then(|s| s.strip_prefix("--cache-dir=")) {
            (&mut options.cache, Some(OsString::from(value)))
        } else if arg.to_string_lossy().starts_with("--data-dir=")
            || arg.to_string_lossy().starts_with("--cache-dir=")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "无法解析数据或缓存目录参数",
            ));
        } else {
            continue;
        };
        if slot.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "数据或缓存目录参数重复",
            ));
        }
        let value = value.filter(|value| !value.is_empty()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "数据或缓存目录参数缺少路径")
        })?;
        let path = PathBuf::from(value);
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "数据和缓存目录参数必须是绝对路径，且不能包含上级目录",
            ));
        }
        *slot = Some(path);
    }
    Ok(options)
}

#[cfg(windows)]
fn roots_from(exe: &Path, options: StorageOptions) -> io::Result<StorageRoots> {
    let directory = exe
        .parent()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法确定 EXE 所在目录"))?;
    let explicit_data = options.data.is_some();
    let explicit_cache = options.cache.is_some();
    let data = options.data.unwrap_or_else(|| directory.join("data"));
    let layered = !has_direct_data_layout(&data)?;
    let cache = options.cache.unwrap_or_else(|| {
        if explicit_data {
            data.join("cache")
        } else {
            directory.join("cache")
        }
    });
    Ok(StorageRoots {
        data,
        cache,
        explicit_data,
        explicit_cache,
        layered,
    })
}

#[cfg(windows)]
fn storage_roots() -> io::Result<&'static StorageRoots> {
    if let Some(roots) = STORAGE_ROOTS.get() {
        return Ok(roots);
    }
    let exe = std::env::current_exe()?;
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let roots = roots_from(&exe, parse_storage_options(&args)?)?;
    let _ = STORAGE_ROOTS.set(roots);
    STORAGE_ROOTS
        .get()
        .ok_or_else(|| io::Error::other("无法初始化存储目录"))
}

#[cfg(windows)]
fn selected_storage_roots() -> &'static StorageRoots {
    storage_roots().expect("storage paths must be validated during startup")
}

#[cfg(windows)]
pub(crate) fn has_explicit_data_dir() -> bool {
    selected_storage_roots().explicit_data
}

#[cfg(not(windows))]
pub(crate) fn has_explicit_data_dir() -> bool {
    false
}

#[cfg(windows)]
fn storage_arguments_for(roots: &StorageRoots) -> Vec<OsString> {
    let mut args = Vec::new();
    if roots.explicit_data {
        args.push(OsString::from("--data-dir"));
        args.push(roots.data.as_os_str().to_os_string());
    }
    if roots.explicit_cache {
        args.push(OsString::from("--cache-dir"));
        args.push(roots.cache.as_os_str().to_os_string());
    }
    args
}

#[cfg(windows)]
pub(crate) fn storage_launch_arguments() -> io::Result<Vec<OsString>> {
    Ok(storage_arguments_for(storage_roots()?))
}

#[cfg(windows)]
fn equivalent_windows_path(left: &Path, right: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        return left == right;
    }
    fn key(path: &Path) -> String {
        let text = path.to_string_lossy().replace('/', "\\");
        let text = text.strip_prefix(r"\\?\UNC\").map_or_else(
            || text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned(),
            |suffix| format!(r"\\{suffix}"),
        );
        text.trim_end_matches('\\').to_lowercase()
    }
    key(left) == key(right)
}

#[cfg(windows)]
fn storage_options_conflict(
    requested: &StorageOptions,
    current: &StorageRoots,
    active_app: &Path,
) -> io::Result<bool> {
    if requested
        .data
        .as_deref()
        .is_some_and(|path| !equivalent_windows_path(path, &current.data))
        || requested
            .cache
            .as_deref()
            .is_some_and(|path| !equivalent_windows_path(path, &current.cache))
    {
        return Ok(true);
    }
    if let Some(data) = requested.data.as_deref() {
        let requested_app = if has_direct_data_layout(data)? {
            data.to_path_buf()
        } else {
            data.join("app")
        };
        return Ok(!equivalent_windows_path(&requested_app, active_app));
    }
    Ok(false)
}

/// A second invocation without path options is only a window/deep-link wakeup.
#[cfg(windows)]
pub(crate) fn second_instance_storage_conflict(args: &[String]) -> io::Result<bool> {
    let args: Vec<OsString> = args
        .iter()
        .skip(1)
        .map(|value| OsString::from(value.as_str()))
        .collect();
    let requested = parse_storage_options(&args)?;
    let active_app = if requested.data.is_some() {
        crate::config::get_app_config_dir()
    } else {
        PathBuf::new()
    };
    storage_options_conflict(&requested, storage_roots()?, &active_app)
}

pub(crate) fn test_root() -> Option<PathBuf> {
    #[cfg(windows)]
    if storage_roots()
        .ok()
        .is_some_and(|roots| roots.explicit_data || roots.explicit_cache)
    {
        return None;
    }
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
    return selected_storage_roots().app();
    #[cfg(not(windows))]
    crate::config::get_home_dir().join(".cc-switch")
}

pub fn device_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root;
    }
    #[cfg(windows)]
    return selected_storage_roots().device();
    #[cfg(not(windows))]
    crate::config::get_home_dir().join(".cc-switch")
}

pub fn tauri_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root.join("tauri");
    }
    #[cfg(windows)]
    return selected_storage_roots().data.join("tauri");
    #[cfg(not(windows))]
    dirs::config_dir()
        .unwrap_or_else(device_dir)
        .join("com.ccswitch.desktop")
}

#[cfg(windows)]
pub fn webview_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root.join("webview2");
    }
    selected_storage_roots().data.join("webview2")
}

pub fn cache_dir() -> PathBuf {
    if let Some(root) = test_root() {
        return root.join("cache");
    }
    #[cfg(windows)]
    return selected_storage_roots().cache.clone();
    #[cfg(not(windows))]
    dirs::cache_dir()
        .unwrap_or_else(|| device_dir().join("cache"))
        .join("com.ccswitch.desktop")
}

pub fn temp_dir() -> PathBuf {
    cache_dir().join("tmp")
}

#[cfg(windows)]
fn windows_volume(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let Component::Prefix(prefix) = path.components().next()? else {
        return None;
    };
    match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            Some(format!("drive:{}", (drive as char).to_ascii_uppercase()))
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(format!(
            "unc:{}:{}",
            server.to_string_lossy().to_lowercase(),
            share.to_string_lossy().to_lowercase()
        )),
        _ => None,
    }
}

/// Validate selected CC storage without changing any external application's paths.
pub fn validate_data_dir(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use std::path::Component;
        let volume = windows_volume(path);
        if !path.is_absolute()
            || volume.is_none()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CC Switch 数据和缓存目录必须是有效绝对路径，且不能包含上级目录",
            ));
        }
        // Unlike the old fixed-D layout, each explicitly selected volume must
        // exist; missing drives must never be replaced by another location.
        let root = path
            .ancestors()
            .last()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法确定存储目录所在卷"))?;
        if !std::fs::metadata(root)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "存储目录所在卷不可用",
            ));
        }
        for ancestor in path.ancestors() {
            match std::fs::symlink_metadata(ancestor) {
                Ok(metadata) => {
                    if metadata.file_attributes() & 0x400 != 0 || !metadata.is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "CC Switch 目录不能经过链接、重解析点或普通文件",
                        ));
                    }
                    if windows_volume(&std::fs::canonicalize(ancestor)?) != volume {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "CC Switch 目录解析后离开所选卷",
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
    if path.is_dir() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无法创建存储目录"))?;
    std::fs::create_dir_all(parent)?;
    validate_data_dir(path)?;
    match std::fs::create_dir(path) {
        Ok(()) => {
            validate_data_dir(path)?;
            crate::data_permissions::secure_directory(path)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => validate_data_dir(path),
        Err(error) => Err(error),
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
        let roots = storage_roots()?;
        // Capture the old profile location before setting this process's profile.
        let _ = legacy_webview_dir();
        let data_root = test_root().unwrap_or_else(|| roots.data.clone());
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
        // The single-instance lock is not held yet. Inspect paths only here;
        // directory creation and write probes happen in ensure_storage_ready.
        for path in [
            data_root,
            app_dir(),
            device_dir(),
            tauri_dir(),
            webview_dir(),
            cache_dir(),
            temp_dir(),
        ] {
            validate_data_dir(&path)?;
        }
        std::env::set_var("TEMP", temp_dir());
        std::env::set_var("TMP", temp_dir());
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", webview_dir());
    }
    Ok(())
}

/// Called after single-instance ownership, before migration or persistence.
#[cfg(windows)]
pub fn ensure_storage_ready() -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::io::Write;
        let roots = storage_roots()?;
        let data_root = test_root().unwrap_or_else(|| roots.data.clone());
        for root in [&data_root, &cache_dir()] {
            ensure_private_directory(root)?;
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
    }
    Ok(())
}

pub fn report_startup_error(error: impl std::fmt::Display) {
    let message = format!("CC Switch 无法初始化数据或缓存目录。\n\n{error}");
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
    fn accepts_only_safe_absolute_storage_paths() {
        for path in [r"D:relative", r"..\data", r"D:\Data\..\outside"] {
            assert!(validate_data_dir(Path::new(path)).is_err(), "{path}");
        }
        let temp = tempfile::tempdir().unwrap();
        let absent = temp.path().join("new").join("data");
        assert!(validate_data_dir(&absent).is_ok());
        assert!(
            !absent.exists(),
            "read-only validation must not create directories"
        );
        if let Some(missing) = (b'A'..=b'Z')
            .map(|drive| format!("{}:\\", drive as char))
            .find(|drive| !Path::new(drive).exists())
        {
            assert!(validate_data_dir(&PathBuf::from(missing).join("cc-switch-data")).is_err());
        }
    }

    #[test]
    fn canonical_and_plain_windows_volume_prefixes_match() {
        assert_eq!(
            windows_volume(Path::new(r"C:\example")),
            windows_volume(Path::new(r"\\?\c:\example"))
        );
        assert_ne!(
            windows_volume(Path::new(r"C:\example")),
            windows_volume(Path::new(r"D:\example"))
        );
        assert_eq!(
            windows_volume(Path::new(r"\\server\share\example")),
            windows_volume(Path::new(r"\\?\UNC\server\share\example"))
        );
    }

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(|value| OsString::from(*value)).collect()
    }

    #[test]
    fn defaults_to_exe_directory_and_supports_explicit_independent_roots() {
        let fixture = tempfile::tempdir().unwrap();
        let exe = fixture.path().join("program").join("cc-switch.exe");
        let defaults = roots_from(&exe, parse_storage_options(&[]).unwrap()).unwrap();
        assert_eq!(defaults.data, fixture.path().join("program").join("data"));
        assert_eq!(defaults.cache, fixture.path().join("program").join("cache"));
        assert!(!defaults.explicit_data && !defaults.explicit_cache);
        assert_eq!(defaults.app(), defaults.data);
        assert_eq!(defaults.device(), defaults.data);
        assert!(storage_arguments_for(&defaults).is_empty());
        let data = fixture.path().join("selected data");
        let cache = fixture.path().join("separate cache");
        let selected = roots_from(
            &exe,
            parse_storage_options(&[
                OsString::from("--data-dir"),
                data.as_os_str().to_os_string(),
                OsString::from("--cache-dir"),
                cache.as_os_str().to_os_string(),
                OsString::from("ccswitch://example"),
            ])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(selected.data, data);
        assert_eq!(selected.cache, cache);
        assert!(selected.explicit_data && selected.explicit_cache);
        assert_eq!(
            storage_arguments_for(&selected),
            vec![
                OsString::from("--data-dir"),
                data.as_os_str().to_os_string(),
                OsString::from("--cache-dir"),
                cache.as_os_str().to_os_string(),
            ]
        );
        assert_eq!(selected.app(), data);
        assert_eq!(selected.device(), data);
        assert_eq!(selected.data.join("tauri"), data.join("tauri"));
        assert_eq!(selected.data.join("webview2"), data.join("webview2"));
        assert_eq!(selected.cache.join("tmp"), cache.join("tmp"));

        let only_data = roots_from(
            &exe,
            StorageOptions {
                data: Some(data.clone()),
                cache: None,
            },
        )
        .unwrap();
        assert_eq!(only_data.cache, data.join("cache"));
        assert!(!only_data.explicit_cache);
    }

    #[test]
    fn rejects_missing_duplicate_relative_and_parent_paths() {
        let equals =
            parse_storage_options(&args(&[r"--data-dir=D:\example", r"--cache-dir=E:\cache"]))
                .unwrap();
        assert_eq!(equals.data, Some(PathBuf::from(r"D:\example")));
        assert_eq!(equals.cache, Some(PathBuf::from(r"E:\cache")));
        for values in [
            vec!["--data-dir"],
            vec!["--cache-dir="],
            vec!["--data-dir", r"D:relative"],
            vec!["--cache-dir", r"D:\Data\..\elsewhere"],
            vec!["--data-dir", r"D:\Data", r"--data-dir=D:\Other"],
            vec!["--data-dir", "--cache-dir", r"D:\Cache"],
        ] {
            assert!(parse_storage_options(&args(&values)).is_err(), "{values:?}");
        }
    }

    #[test]
    fn second_instance_only_compares_explicit_options() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path().join("data-root");
        let cache = fixture.path().join("cache-root");
        let current = roots_from(
            &fixture.path().join("cc-switch.exe"),
            StorageOptions {
                data: Some(data.clone()),
                cache: Some(cache.clone()),
            },
        )
        .unwrap();
        for requested in [
            parse_storage_options(&args(&["ccswitch://example"])).unwrap(),
            StorageOptions {
                data: Some(data.clone()),
                cache: None,
            },
            StorageOptions {
                data: None,
                cache: Some(cache.clone()),
            },
        ] {
            assert!(!storage_options_conflict(&requested, &current, &current.app()).unwrap());
        }
        for requested in [
            StorageOptions {
                data: Some(fixture.path().join("different-data")),
                cache: None,
            },
            StorageOptions {
                data: None,
                cache: Some(fixture.path().join("different-cache")),
            },
        ] {
            assert!(storage_options_conflict(&requested, &current, &current.app()).unwrap());
        }
        let store_override = fixture.path().join("official-store-override");
        let explicit_same_root = StorageOptions {
            data: Some(data),
            cache: None,
        };
        assert!(storage_options_conflict(&explicit_same_root, &current, &store_override).unwrap());
        let ordinary_wakeup = StorageOptions::default();
        assert!(!storage_options_conflict(&ordinary_wakeup, &current, &store_override).unwrap());
    }

    #[test]
    fn existing_direct_and_layered_data_are_detected_without_copying() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("existing");
        std::fs::create_dir_all(&root).unwrap();
        let exe = fixture.path().join("cc-switch.exe");
        let options = || StorageOptions {
            data: Some(root.clone()),
            cache: None,
        };
        std::fs::write(root.join("cc-switch.db"), b"synthetic direct database").unwrap();
        let direct = roots_from(&exe, options()).unwrap();
        assert!(!direct.layered);
        assert_eq!(direct.app(), root);
        assert_eq!(direct.device(), root);
        let nested = root.join("app");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(
            nested.join("config.json"),
            b"synthetic nested configuration",
        )
        .unwrap();
        assert!(roots_from(&exe, options()).is_err());
        assert_eq!(
            std::fs::read(root.join("cc-switch.db")).unwrap(),
            b"synthetic direct database"
        );
        assert_eq!(
            std::fs::read(nested.join("config.json")).unwrap(),
            b"synthetic nested configuration"
        );
        let selected_nested = roots_from(
            &exe,
            StorageOptions {
                data: Some(nested.clone()),
                cache: None,
            },
        )
        .unwrap();
        assert!(!selected_nested.layered);
        assert_eq!(selected_nested.app(), nested);
    }

    #[test]
    fn default_exe_data_detects_existing_direct_layout() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("data");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("cc-switch.db"), b"synthetic portable database").unwrap();
        let exe = fixture.path().join("cc-switch.exe");
        let selected = roots_from(&exe, StorageOptions::default()).unwrap();
        assert!(!selected.layered);
        assert!(!selected.explicit_data);
        assert_eq!(selected.app(), root);
        assert_eq!(selected.device(), root);
        assert_eq!(selected.cache, fixture.path().join("cache"));
        assert_eq!(
            std::fs::read(root.join("cc-switch.db")).unwrap(),
            b"synthetic portable database"
        );
    }

    #[test]
    fn direct_layout_preserves_regular_files_named_app_and_device() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path().join("existing-direct");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("cc-switch.db"), b"synthetic database").unwrap();
        std::fs::write(data.join("app"), b"unknown app content").unwrap();
        std::fs::write(data.join("device"), b"unknown device content").unwrap();
        let selected = roots_from(
            &fixture.path().join("cc-switch.exe"),
            StorageOptions {
                data: Some(data.clone()),
                cache: None,
            },
        )
        .unwrap();
        assert!(!selected.layered);
        assert_eq!(selected.app(), data);
        assert_eq!(
            std::fs::read(data.join("app")).unwrap(),
            b"unknown app content"
        );
        assert_eq!(
            std::fs::read(data.join("device")).unwrap(),
            b"unknown device content"
        );
    }

    #[test]
    fn layout_rejects_app_and_device_reparse_points() {
        let fixture = tempfile::tempdir().unwrap();
        let outside = fixture.path().join("external");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("unchanged"), b"synthetic external data").unwrap();
        for name in ["app", "device"] {
            let data = fixture.path().join(format!("direct-{name}"));
            std::fs::create_dir(&data).unwrap();
            std::fs::write(data.join("cc-switch.db"), b"synthetic database").unwrap();
            crate::agy_config::create_dir_link(&outside, &data.join(name)).unwrap();
            assert!(roots_from(
                &fixture.path().join("cc-switch.exe"),
                StorageOptions {
                    data: Some(data),
                    cache: None
                },
            )
            .is_err());
            assert_eq!(
                std::fs::read(outside.join("unchanged")).unwrap(),
                b"synthetic external data"
            );
        }
    }

    #[test]
    fn marked_layered_app_requires_its_parent_data_root() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("layered");
        let app = root.join("app");
        let tauri = root.join("tauri");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&tauri).unwrap();
        std::fs::write(app.join("cc-switch.db"), b"synthetic database").unwrap();
        let marker = serde_json::json!({"version": 1, "targets": {"app": app.clone(), "device": root.join("device")}});
        std::fs::write(
            tauri.join(".cc-switch-data-migration-v1.json"),
            marker.to_string(),
        )
        .unwrap();
        let exe = fixture.path().join("cc-switch.exe");
        assert!(roots_from(
            &exe,
            StorageOptions {
                data: Some(app.clone()),
                cache: None
            }
        )
        .is_err());
        let layered = roots_from(
            &exe,
            StorageOptions {
                data: Some(root),
                cache: None,
            },
        )
        .unwrap();
        assert!(layered.layered);
        assert_eq!(layered.app(), app);
        assert_eq!(
            std::fs::read(app.join("cc-switch.db")).unwrap(),
            b"synthetic database"
        );
    }

    #[test]
    fn layered_layout_survives_missing_database_and_tracks_existing_device_data() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("old-layered");
        let device = root.join("device");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::write(device.join("settings.json"), b"synthetic device settings").unwrap();
        let exe = fixture.path().join("cc-switch.exe");
        let options = || StorageOptions {
            data: Some(root.clone()),
            cache: None,
        };
        let selected = roots_from(&exe, options()).unwrap();
        assert!(selected.layered);
        assert_eq!(selected.app(), root.join("app"));
        assert_eq!(selected.device(), device);
        std::fs::write(root.join("config.json"), b"synthetic competing data").unwrap();
        assert!(roots_from(&exe, options()).is_err());
        assert_eq!(
            std::fs::read(device.join("settings.json")).unwrap(),
            b"synthetic device settings"
        );
    }

    #[test]
    fn existing_layered_marker_is_preserved_even_when_data_files_are_missing() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("moved-layered");
        let tauri = root.join("tauri");
        std::fs::create_dir_all(&tauri).unwrap();
        let previous = fixture.path().join("previous-location");
        let marker = serde_json::json!({"version": 1, "targets": {
            "app": previous.join("app"), "device": previous.join("device")
        }});
        std::fs::write(
            tauri.join(".cc-switch-data-migration-v1.json"),
            marker.to_string(),
        )
        .unwrap();
        let selected = roots_from(
            &fixture.path().join("cc-switch.exe"),
            StorageOptions {
                data: Some(root.clone()),
                cache: None,
            },
        )
        .unwrap();
        assert!(selected.layered);
        assert_eq!(selected.app(), root.join("app"));
        assert_eq!(selected.device(), root.join("device"));
    }

    #[test]
    fn existing_directory_acl_is_preserved_and_new_child_is_private() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::{GetFileSecurityW, DACL_SECURITY_INFORMATION};

        fn dacl(path: &Path) -> Vec<u8> {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut length = 0u32;
            unsafe {
                GetFileSecurityW(
                    wide.as_ptr(),
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    0,
                    &mut length,
                );
            }
            assert!(length > 0);
            let mut bytes = vec![0u8; length as usize];
            assert_ne!(
                unsafe {
                    GetFileSecurityW(
                        wide.as_ptr(),
                        DACL_SECURITY_INFORMATION,
                        bytes.as_mut_ptr().cast(),
                        length,
                        &mut length,
                    )
                },
                0
            );
            bytes.truncate(length as usize);
            bytes
        }

        let fixture = tempfile::tempdir().unwrap();
        let existing = fixture.path().join("existing-user-directory");
        std::fs::create_dir(&existing).unwrap();
        let before = dacl(&existing);
        ensure_private_directory(&existing).unwrap();
        assert_eq!(
            dacl(&existing),
            before,
            "existing ACL must not be rewritten"
        );
        let child = existing.join("new-cc-directory");
        ensure_private_directory(&child).unwrap();
        crate::data_permissions::validate_directory(&child).unwrap();
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
