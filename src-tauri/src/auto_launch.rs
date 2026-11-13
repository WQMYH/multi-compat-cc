use crate::error::AppError;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

// auto-launch 0.5 writes its Windows registry command by joining strings with
// spaces; encode each argument using the Windows CRT command-line rules.
#[cfg(windows)]
fn quote_windows_argument(value: &str) -> String {
    let mut result = String::from("\"");
    let mut backslashes = 0usize;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                result.extend(std::iter::repeat('\\').take(backslashes * 2 + 1));
                result.push('"');
                backslashes = 0;
            }
            _ => {
                result.extend(std::iter::repeat('\\').take(backslashes));
                result.push(character);
                backslashes = 0;
            }
        }
    }
    result.extend(std::iter::repeat('\\').take(backslashes * 2));
    result.push('"');
    result
}

/// 获取 macOS 上的 .app bundle 路径
/// 将 `/path/to/CC Switch.app/Contents/MacOS/CC Switch` 转换为 `/path/to/CC Switch.app`
#[cfg(target_os = "macos")]
fn get_macos_app_bundle_path(exe_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path_str = exe_path.to_string_lossy();
    // 查找 .app/Contents/MacOS/ 模式
    if let Some(app_pos) = path_str.find(".app/Contents/MacOS/") {
        let app_bundle_end = app_pos + 4; // ".app" 的结束位置
        Some(std::path::PathBuf::from(&path_str[..app_bundle_end]))
    } else {
        None
    }
}

/// 初始化 AutoLaunch 实例
fn get_auto_launch() -> Result<AutoLaunch, AppError> {
    let app_name = "CC Switch";
    let exe_path =
        std::env::current_exe().map_err(|e| AppError::Message(format!("无法获取应用路径: {e}")))?;

    // macOS 需要使用 .app bundle 路径，否则 AppleScript login item 会打开终端
    #[cfg(target_os = "macos")]
    let app_path = get_macos_app_bundle_path(&exe_path).unwrap_or(exe_path);

    #[cfg(not(target_os = "macos"))]
    let app_path = exe_path;

    // 使用 AutoLaunchBuilder 消除平台差异
    // macOS: 使用 AppleScript 方式（默认），需要 .app bundle 路径
    // Windows/Linux: 使用注册表/XDG autostart
    #[cfg(windows)]
    let app_path_string = quote_windows_argument(
        app_path
            .to_str()
            .ok_or_else(|| AppError::Message("无法表示自启动程序路径".to_string()))?,
    );
    #[cfg(not(windows))]
    let app_path_string = app_path.to_string_lossy().to_string();

    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(app_name)
        .set_app_path(&app_path_string);
    #[cfg(windows)]
    {
        let storage_args = crate::data_paths::storage_launch_arguments()
            .map_err(|e| AppError::Message(format!("无法读取自启动数据目录: {e}")))?;
        let quoted_args: Vec<String> = storage_args
            .iter()
            .map(|arg| {
                arg.to_str()
                    .map(quote_windows_argument)
                    .ok_or_else(|| AppError::Message("无法表示自启动目录参数".to_string()))
            })
            .collect::<Result<_, _>>()?;
        builder.set_args(&quoted_args);
    }
    let auto_launch = builder
        .build()
        .map_err(|e| AppError::Message(format!("创建 AutoLaunch 失败: {e}")))?;

    Ok(auto_launch)
}

/// 启用开机自启
pub fn enable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .enable()
        .map_err(|e| AppError::Message(format!("启用开机自启失败: {e}")))?;
    log::info!("已启用开机自启");
    Ok(())
}

/// 禁用开机自启
pub fn disable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .disable()
        .map_err(|e| AppError::Message(format!("禁用开机自启失败: {e}")))?;
    log::info!("已禁用开机自启");
    Ok(())
}

/// 检查是否已启用开机自启
pub fn is_auto_launch_enabled() -> Result<bool, AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .is_enabled()
        .map_err(|e| AppError::Message(format!("检查开机自启状态失败: {e}")))
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_startup_command_preserves_spaces_quotes_and_trailing_slashes() {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

        let arguments = [
            r"C:\Program Files\CC Switch\cc-switch.exe",
            "--data-dir",
            "D:\\Data with spaces\\",
            "--cache-dir",
            r"D:\缓存\CC Switch",
            "embedded\"quote",
            "",
        ];
        let command = arguments
            .iter()
            .map(|value| quote_windows_argument(value))
            .collect::<Vec<_>>()
            .join(" ");
        let command_wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let mut count = 0i32;
        let pointer = unsafe { CommandLineToArgvW(command_wide.as_ptr(), &mut count) };
        assert!(!pointer.is_null());
        let decoded: Vec<_> = (0..count as usize)
            .map(|index| {
                let value = unsafe { *pointer.add(index) };
                let mut length = 0usize;
                while unsafe { *value.add(length) } != 0 {
                    length += 1;
                }
                let wide = unsafe { std::slice::from_raw_parts(value, length) };
                std::ffi::OsString::from_wide(wide)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        unsafe {
            LocalFree(pointer.cast());
        }
        assert_eq!(
            decoded,
            arguments
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_valid() {
        let exe_path = std::path::Path::new("/Applications/CC Switch.app/Contents/MacOS/CC Switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(
            result,
            Some(std::path::PathBuf::from("/Applications/CC Switch.app"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_with_spaces() {
        let exe_path =
            std::path::Path::new("/Users/test/My Apps/CC Switch.app/Contents/MacOS/CC Switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(
            result,
            Some(std::path::PathBuf::from(
                "/Users/test/My Apps/CC Switch.app"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_not_in_bundle() {
        let exe_path = std::path::Path::new("/usr/local/bin/cc-switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_dev_build() {
        // 开发环境下的路径通常不在 .app bundle 内
        let exe_path = std::path::Path::new("/Users/dev/project/target/debug/cc-switch");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(result, None);
    }
}
