use serde_json::Value;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use tauri_plugin_store::StoreExt;

use crate::error::AppError;

/// Store 中的键名
const STORE_KEY_APP_CONFIG_DIR: &str = "app_config_dir_override";

/// 缓存当前的 app_config_dir 覆盖路径，避免存储 AppHandle
static APP_CONFIG_DIR_OVERRIDE: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

fn override_cache() -> &'static RwLock<Option<PathBuf>> {
    APP_CONFIG_DIR_OVERRIDE.get_or_init(|| RwLock::new(None))
}

fn update_cached_override(value: Option<PathBuf>) {
    if let Ok(mut guard) = override_cache().write() {
        *guard = value;
    }
}

/// 获取缓存中的 app_config_dir 覆盖路径
pub fn get_app_config_dir_override() -> Option<PathBuf> {
    override_cache().read().ok()?.clone()
}

fn store_file() -> PathBuf {
    #[cfg(windows)]
    return crate::data_paths::tauri_dir().join("app_paths.json");
    #[cfg(not(windows))]
    PathBuf::from("app_paths.json")
}

/// Read the saved next-start selection without changing the active database root.
pub fn read_app_config_dir_override(app: &tauri::AppHandle) -> Result<Option<PathBuf>, AppError> {
    // An explicit launch root takes precedence; an old, unavailable store
    // override must not prevent the selected root from starting.
    if crate::data_paths::has_explicit_data_dir() {
        return Ok(None);
    }
    let store = app
        .store_builder(store_file())
        .build()
        .map_err(|error| AppError::Message(format!("无法读取 CC Switch 目录设置：{error}")))?;

    match store.get(STORE_KEY_APP_CONFIG_DIR) {
        Some(Value::String(path_str)) => {
            let path_str = path_str.trim();
            if path_str.is_empty() {
                return Ok(None);
            }

            let path = validate_override_path(path_str)?;

            log::info!("使用 Store 中的 app_config_dir: {path:?}");
            Ok(Some(path))
        }
        Some(_) => Err(AppError::Message(format!(
            "Store 中的 {STORE_KEY_APP_CONFIG_DIR} 应为字符串"
        ))),
        None => Ok(None),
    }
}

/// 从 Store 刷新 app_config_dir 覆盖值并更新缓存
pub fn refresh_app_config_dir_override(
    app: &tauri::AppHandle,
) -> Result<Option<PathBuf>, AppError> {
    let value = read_app_config_dir_override(app)?;
    update_cached_override(value.clone());
    Ok(value)
}

/// 写入 app_config_dir 到 Tauri Store
pub fn set_app_config_dir_to_store(
    app: &tauri::AppHandle,
    path: Option<&str>,
) -> Result<(), AppError> {
    if crate::data_paths::has_explicit_data_dir() {
        return Err(AppError::Message(
            "当前数据目录由 --data-dir 启动参数指定，请修改启动参数后重启".into(),
        ));
    }
    let store = app
        .store_builder(store_file())
        .build()
        .map_err(|e| AppError::Message(format!("创建 Store 失败: {e}")))?;

    match path {
        Some(p) => {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                let _selected = validate_override_path(trimmed)?;
                #[cfg(windows)]
                crate::data_migration::validate_app_override_for_current_targets(&_selected)
                    .map_err(|error| AppError::Message(error.to_string()))?;
                store.set(STORE_KEY_APP_CONFIG_DIR, Value::String(trimmed.to_string()));
                log::info!("已将 app_config_dir 写入 Store: {trimmed}");
            } else {
                store.delete(STORE_KEY_APP_CONFIG_DIR);
                log::info!("已从 Store 中删除 app_config_dir 配置");
            }
        }
        None => {
            store.delete(STORE_KEY_APP_CONFIG_DIR);
            log::info!("已从 Store 中删除 app_config_dir 配置");
        }
    }

    store
        .save()
        .map_err(|e| AppError::Message(format!("保存 Store 失败: {e}")))?;

    Ok(())
}

/// 解析路径，支持 ~ 开头的相对路径
pub(crate) fn resolve_path(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if let Some(stripped) = raw.strip_prefix("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }

    PathBuf::from(raw)
}

fn validate_override_path(raw: &str) -> Result<PathBuf, AppError> {
    let selected = resolve_path(raw);
    crate::data_paths::validate_data_dir(&selected)
        .map_err(|error| AppError::Message(error.to_string()))?;
    if !selected.is_dir() {
        return Err(AppError::Message(format!(
            "CC Switch 数据目录必须已经存在且可用：{}",
            selected.display()
        )));
    }
    #[cfg(windows)]
    {
        let selected_real = std::fs::canonicalize(&selected)
            .map_err(|error| AppError::Message(error.to_string()))?;
        let broad = selected.parent().is_none()
            || [dirs::home_dir(), dirs::data_dir(), dirs::data_local_dir()]
                .into_iter()
                .flatten()
                .any(|root| {
                    std::fs::canonicalize(root).is_ok_and(|root| root.starts_with(&selected_real))
                });
        if broad {
            return Err(AppError::Message(
                "请选择 CC Switch 专用数据目录，不能使用整盘或用户公共根目录".into(),
            ));
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_validation_requires_an_existing_directory() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().to_str().unwrap();
        assert_eq!(validate_override_path(existing).unwrap(), directory.path());
        let missing = directory.path().join("not-created");
        assert!(validate_override_path(missing.to_str().unwrap()).is_err());
        assert!(!missing.exists());
    }

    #[cfg(windows)]
    #[test]
    fn override_validation_rejects_other_drives() {
        assert!(validate_override_path(r"C:\Users").is_err());
        assert!(validate_override_path(r"D:\").is_err());
    }
}

/// 从旧的 settings.json 迁移 app_config_dir 到 Store
pub fn migrate_app_config_dir_from_settings(app: &tauri::AppHandle) -> Result<(), AppError> {
    // app_config_dir 已从 settings.json 移除，此函数保留但不再执行迁移
    // 如果用户在旧版本设置过 app_config_dir，需要在 Store 中手动配置
    log::info!("app_config_dir 迁移功能已移除，请在设置中重新配置");

    refresh_app_config_dir_override(app)?;
    Ok(())
}
