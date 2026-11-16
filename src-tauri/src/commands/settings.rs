#![allow(non_snake_case)]

use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// 应用更新下载进度（通过 `update-download-progress` 事件发给前端）。
#[derive(Clone, serde::Serialize)]
struct UpdateDownloadProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// 开启统一会话历史后，后台把官方会话迁进共享组的那一轮跑完了（或失败）。
const CODEX_UNIFY_MIGRATION_FINISHED_EVENT: &str = "codex-unify-migration-finished";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexUnifyMigrationFinished {
    migrated_state_rows: usize,
    migrated_jsonl_files: usize,
    error: Option<String>,
}

fn merge_settings_for_save(
    mut incoming: crate::settings::AppSettings,
    existing: &crate::settings::AppSettings,
) -> crate::settings::AppSettings {
    match (&mut incoming.webdav_sync, &existing.webdav_sync) {
        // incoming 没有 webdav → 保留现有
        (None, _) => {
            incoming.webdav_sync = existing.webdav_sync.clone();
        }
        // incoming 有 webdav 但密码为空，且现有有密码 → 填回现有密码
        // （get_settings_for_frontend 总是清空密码，所以通过 save_settings
        //   传入的空密码意味着"保持现有"而非"用户主动清空"）
        (Some(incoming_sync), Some(existing_sync))
            if incoming_sync.password.is_empty() && !existing_sync.password.is_empty() =>
        {
            incoming_sync.password = existing_sync.password.clone();
        }
        _ => {}
    }
    match (&mut incoming.s3_sync, &existing.s3_sync) {
        // incoming 没有 s3 → 保留现有
        (None, _) => {
            incoming.s3_sync = existing.s3_sync.clone();
        }
        // incoming 有 s3 但密钥为空，且现有有密钥 → 填回现有密钥
        (Some(incoming_sync), Some(existing_sync))
            if incoming_sync.secret_access_key.is_empty()
                && !existing_sync.secret_access_key.is_empty() =>
        {
            incoming_sync.secret_access_key = existing_sync.secret_access_key.clone();
        }
        _ => {}
    }
    // local_migrations 是纯后端状态（迁移完成标记），前端没有合法的修改场景，
    // 无条件取现有值。若按 incoming 透传：后端清掉 marker（如关闭统一会话
    // 开关）后、前端 query 缓存刷新前的一次全量保存会把旧 marker 重放回来，
    // 重新开启时被"复活"的标记挡住而漏迁。
    incoming.local_migrations = existing.local_migrations.clone();
    // 客户端地址只经 set_proxy_client_host 保存（它还要重写客户端）；全量保存带的是前端
    // 缓存里的旧值，透传会把刚保存的地址冲掉。
    incoming.proxy_client_host = existing.proxy_client_host.clone();
    // 轻量模式状态只由进入 / 退出轻量模式时写，前端缓存里的值总是旧的。
    incoming.lightweight_mode = existing.lightweight_mode;
    incoming
}

#[tauri::command]
pub async fn get_settings() -> Result<crate::settings::AppSettings, String> {
    Ok(crate::settings::get_settings_for_frontend())
}

#[tauri::command]
pub async fn save_settings(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::store::AppState>,
    settings: crate::settings::AppSettings,
) -> Result<bool, String> {
    let existing = crate::settings::get_settings();
    let merged = merge_settings_for_save(settings, &existing);
    let unify_codex_changed =
        merged.unify_codex_session_history != existing.unify_codex_session_history;
    let unify_codex_enabled = merged.unify_codex_session_history;
    // 经典子 agent 开关、聚合子 agent 模型/强度和聚合自动审批模型都只影响 Stack 模式下
    // 写给 Codex 的内容。
    let stack_settings_changed = merged.codex_stack_classic_tools
        != existing.codex_stack_classic_tools
        || merged.codex_stack_subagent_model != existing.codex_stack_subagent_model
        || merged.codex_stack_subagent_reasoning_effort
            != existing.codex_stack_subagent_reasoning_effort
        || merged.codex_stack_auto_review_model != existing.codex_stack_auto_review_model;
    let tray_style_changed = merged.native_tray_menu != existing.native_tray_menu;
    let without_login_changed =
        merged.codex_proxy_without_official_login != existing.codex_proxy_without_official_login;
    let preserve_login_changed = merged.preserve_codex_official_auth_on_switch
        != existing.preserve_codex_official_auth_on_switch;
    crate::settings::update_settings(merged).map_err(|e| e.to_string())?;

    // 原生托盘菜单 / 托盘面板当场切换，不用重启。
    if tray_style_changed {
        crate::tray::apply_tray_style(&app);
    }

    // 立即重写，失败时回滚设置，免得界面显示已改好、客户端还是旧的。
    if stack_settings_changed {
        if let Err(err) = crate::mode::controller::resync_codex_stack_catalog(state.inner()).await {
            log::warn!("Codex 聚合设置变更后重写客户端配置失败，回滚设置: {err}");
            let mut rollback = crate::settings::get_settings();
            rollback.codex_stack_classic_tools = existing.codex_stack_classic_tools;
            rollback.codex_stack_subagent_model = existing.codex_stack_subagent_model.clone();
            rollback.codex_stack_subagent_reasoning_effort =
                existing.codex_stack_subagent_reasoning_effort.clone();
            rollback.codex_stack_auto_review_model = existing.codex_stack_auto_review_model.clone();
            if let Err(rollback_err) = crate::settings::update_settings(rollback) {
                log::error!("回滚 Codex 聚合设置失败: {rollback_err}");
            }
            return Err(format!(
                "聚合设置未生效（Codex 配置重写失败） (The aggregation setting did not take effect: rewriting the Codex config failed): {err}"
            ));
        }
    }

    // 「暂停使用官方登录」只影响路由 / 聚合模式下写给 Codex 的内容：立即重写，失败回滚。
    if without_login_changed {
        if let Err(err) = crate::mode::controller::resync_codex_proxy(state.inner()).await {
            log::warn!("「暂停使用官方登录」变更后重写 Codex 配置失败，回滚设置: {err}");
            let mut rollback = crate::settings::get_settings();
            rollback.codex_proxy_without_official_login =
                existing.codex_proxy_without_official_login;
            if let Err(rollback_err) = crate::settings::update_settings(rollback) {
                log::error!("回滚「暂停使用官方登录」设置失败: {rollback_err}");
            }
            return Err(format!(
                "「暂停使用官方登录」未生效（Codex 配置重写失败） (\"Pause the official login\" did not take effect: rewriting the Codex config failed): {err}"
            ));
        }
    }

    // 「非接管切换时保留官方登录」只影响直连切到第三方：当前就是直连第三方时立即重写，
    // 不必先切回官方卡再切回来；失败回滚。
    if preserve_login_changed {
        if let Err(err) =
            crate::services::provider::reapply_current_codex_third_party_live(state.inner())
        {
            log::warn!("「非接管切换时保留官方登录」变更后重写 Codex 配置失败，回滚设置: {err}");
            let mut rollback = crate::settings::get_settings();
            rollback.preserve_codex_official_auth_on_switch =
                existing.preserve_codex_official_auth_on_switch;
            if let Err(rollback_err) = crate::settings::update_settings(rollback) {
                log::error!("回滚「非接管切换时保留官方登录」设置失败: {rollback_err}");
            }
            return Err(format!(
                "「非接管切换时保留官方登录」未生效（Codex 配置重写失败） (\"Keep official login for direct switches\" did not take effect: rewriting the Codex config failed): {err}"
            ));
        }
    }

    // 统一会话开关变更时立即重写当前官方 Codex 供应商的 live 配置，
    // 不必等下一次切换才生效。
    if unify_codex_changed {
        // live 重写失败时回滚设置并把保存整体报失败：若设置保持已切换状态，
        // live 仍跑旧桶，后续的历史迁移/还原会让会话再次分裂（开启=历史
        // 迁走而新会话仍写 openai 桶；关闭=会话还原而 live 仍写 custom）。
        // 报错让前端 saved=false 短路还原；回滚是整次保存的事务语义
        // （本开关的保存只携带开关相关字段）。
        if let Err(err) =
            crate::services::provider::reapply_current_codex_official_live(state.inner())
        {
            log::warn!("统一 Codex 会话历史开关变更后重写 live 配置失败，回滚设置: {err}");
            if let Err(rollback_err) = crate::settings::update_settings(existing) {
                log::error!("回滚统一会话开关设置失败: {rollback_err}");
            }
            return Err(format!(
                "统一 Codex 会话历史开关未生效（live 配置重写失败）: {err}"
            ));
        }

        if unify_codex_enabled {
            // 后台把留在 openai 桶的会话迁进 custom 桶（仅当用户勾选了迁入既有
            // 会话，函数内部自门控）。大会话目录可能要读数秒，不能阻塞设置保存；
            // 失败时下次启动自动重试。
            // 迁完（或失败）发事件给前端提示：「开启统一」时前端只说了“后台迁入中”，
            // 用户要等到这一刻再重启 Codex，旧会话才会解锁。
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let finished = match crate::codex_history_migration::maybe_migrate_codex_official_history_to_unified_bucket() {
                    Ok(outcome) => {
                        if let Some(reason) = outcome.skipped_reason {
                            log::debug!("○ Codex official history unify migration skipped: {reason}");
                            None
                        } else {
                            log::info!(
                                "✓ Codex official history unify migration completed: jsonl_files={}, state_rows={}",
                                outcome.migrated_jsonl_files,
                                outcome.migrated_state_rows
                            );
                            Some(CodexUnifyMigrationFinished {
                                migrated_state_rows: outcome.migrated_state_rows,
                                migrated_jsonl_files: outcome.migrated_jsonl_files,
                                error: None,
                            })
                        }
                    }
                    Err(e) => {
                        log::warn!("✗ Codex official history unify migration failed: {e}");
                        Some(CodexUnifyMigrationFinished {
                            migrated_state_rows: 0,
                            migrated_jsonl_files: 0,
                            error: Some(e.to_string()),
                        })
                    }
                };
                if let Some(payload) = finished {
                    if let Err(e) = app.emit(CODEX_UNIFY_MIGRATION_FINISHED_EVENT, payload) {
                        log::warn!("发送 Codex 统一迁移完成事件失败: {e}");
                    }
                }
            });
        } else {
            // 清除标记与迁移意愿，让重新开启并再次勾选时能补迁
            // 关闭期间落入 openai 桶的官方会话。
            if let Err(err) = crate::settings::clear_codex_official_history_unify_migration() {
                log::warn!("清除统一会话迁移标记失败: {err}");
            }
            if let Err(err) = crate::settings::clear_codex_unify_migrate_existing() {
                log::warn!("清除统一会话迁移意愿失败: {err}");
            }
        }
    }
    Ok(true)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUnifyHistoryRestoreResult {
    pub restored_jsonl_files: usize,
    pub restored_state_rows: usize,
    /// 还原被跳过的原因（如当前目录没有账本），前端据此提示而非报"成功 0 项"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped_reason: Option<String>,
}

/// 是否存在统一会话开关的迁移备份（决定关闭弹窗里是否显示"恢复备份"勾选）。
#[tauri::command]
pub async fn has_codex_unify_history_backup() -> Result<bool, String> {
    Ok(crate::codex_history_migration::has_codex_official_history_unify_backup())
}

/// Codex 的 `config.toml` 是不是用 `[features] multi_agent_v2` 强制了新版子 agent 工具
/// （这时「经典子 agent 工具」开关不生效，界面上提示）。
#[tauri::command]
pub async fn codex_forces_multi_agent_v2() -> Result<bool, String> {
    let text = crate::codex_config::read_codex_config_text().map_err(|e| e.to_string())?;
    Ok(crate::codex_config::codex_config_forces_multi_agent_v2(
        &text,
    ))
}

/// Codex 会话压缩开关（`[features] local_thread_store_compression`）当前是否开启。
#[tauri::command]
pub async fn get_codex_session_compression() -> Result<bool, String> {
    crate::services::codex_session_compression::is_enabled().map_err(|e| e.to_string())
}

/// 开关 Codex 会话压缩，返回写入后的状态。
#[tauri::command]
pub async fn set_codex_session_compression(
    state: tauri::State<'_, crate::store::AppState>,
    enabled: bool,
) -> Result<bool, String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::services::codex_session_compression::set_enabled(&db, enabled)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// Codex 会话目录（sessions + archived_sessions）当前占用的字节数。
#[tauri::command]
pub async fn get_codex_sessions_disk_usage() -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(
        crate::services::codex_session_compression::sessions_disk_usage,
    )
    .await
    .map_err(|e| e.to_string())
}

/// 按迁移备份账本把当时迁入共享桶的官方会话还原回 "openai" 桶。
/// 由关闭统一会话开关的确认弹窗触发；幂等，可安全重试。
#[tauri::command]
pub async fn restore_codex_unified_history() -> Result<CodexUnifyHistoryRestoreResult, String> {
    let outcome = tauri::async_runtime::spawn_blocking(|| {
        crate::codex_history_migration::restore_codex_official_history_from_backups()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    if let Some(reason) = &outcome.skipped_reason {
        log::debug!("○ Codex official history restore skipped: {reason}");
    } else {
        log::info!(
            "✓ Codex official history restored from backups: jsonl_files={}, state_rows={}",
            outcome.restored_jsonl_files,
            outcome.restored_state_rows
        );
    }

    Ok(CodexUnifyHistoryRestoreResult {
        restored_jsonl_files: outcome.restored_jsonl_files,
        restored_state_rows: outcome.restored_state_rows,
        skipped_reason: outcome.skipped_reason,
    })
}

/// 重启应用程序（当 app_config_dir 变更后使用）
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<bool, String> {
    crate::save_window_state_before_exit(&app);

    // 在后台延迟重启，让函数有时间返回响应
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        // 本命令用于 app_config_dir 变更后的重启：新实例会切到新数据库，拿不到
        // 旧库里的 Live 备份，无法恢复被接管的 Live 配置。因此必须趁旧实例的
        // 事件循环仍存活，在这里同步完成恢复（保留代理状态，新实例启动时自动
        // 重新接管）。
        crate::cleanup_before_exit(&app).await;
        // 与自更新共用 restart_process，而不是 app.restart()：macOS 上经
        // LaunchServices 启动，新窗口才能回到前台。
        crate::restart_process(&app);
    });
    Ok(true)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn require_registered_msi_installation(is_registered: bool) -> Result<(), String> {
    if is_registered {
        Ok(())
    } else {
        Err(format!(
            "当前安装由外部管理，请前往发布页获取更新：{}",
            super::misc::RELEASES_URL
        ))
    }
}

#[cfg(any(target_os = "windows", test))]
fn require_registered_after_exit_cleanup(is_registered: bool) -> Result<(), String> {
    require_registered_msi_installation(is_registered).map_err(|error| {
        format!(
            "{error}。已执行退出前清理，代理或 Live 接管可能已暂停；请重启应用或重新开启代理后再试。"
        )
    })
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn is_owned_msi_url(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .path()
            .starts_with("/WQMYH/multi-compat-cc/releases/download/")
        && url.path().to_ascii_lowercase().ends_with(".msi")
}

#[cfg(any(target_os = "windows", test))]
fn is_msi_payload(bytes: &[u8]) -> bool {
    // Matches the updater plugin's own MSI dispatch check; never dispatch an
    // EXE/NSIS payload through the MSI-only update path.
    bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1])
}

/// The updater's `download` must verify the artifact and minisign global
/// signature before this function is called. Only then is the trusted comment
/// authenticated and safe to use for binding the artifact to the version.
#[cfg(any(target_os = "windows", test))]
fn require_signed_update_version(signature: &str, expected: &str) -> Result<(), String> {
    use base64::Engine;

    let invalid = || "更新包签名版本缺失、重复或不匹配，已停止安装".to_string();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .map_err(|_| invalid())?;
    let signature = std::str::from_utf8(&decoded).map_err(|_| invalid())?;
    let mut lines = signature.lines();
    let _untrusted_comment = lines.next().ok_or_else(invalid)?;
    let _artifact_signature = lines.next().ok_or_else(invalid)?;
    let trusted_comment = lines
        .next()
        .and_then(|line| line.strip_prefix("trusted comment: "))
        .ok_or_else(invalid)?;
    let _global_signature = lines.next().ok_or_else(invalid)?;

    let mut versions = trusted_comment
        .split('\t')
        .filter_map(|field| field.trim_start_matches(' ').strip_prefix("version:"));
    match (versions.next(), versions.next()) {
        (Some(version), None) if !version.is_empty() && version == expected => Ok(()),
        _ => Err(invalid()),
    }
}

/// Call only after the updater has verified the package signature. This same
/// check is used for freshly downloaded and previously verified cached bytes.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn require_verified_msi_update(
    url: &url::Url,
    signature: &str,
    version: &str,
    bytes: &[u8],
    is_registered: bool,
) -> Result<(), String> {
    require_registered_msi_installation(is_registered)?;
    if !is_owned_msi_url(url) {
        return Err("更新包不是本项目发布的 MSI，已停止安装".to_string());
    }
    require_signed_update_version(signature, version)?;
    if !is_msi_payload(bytes) {
        return Err("更新包内容不是 MSI，已停止安装".to_string());
    }
    Ok(())
}

/// 下载并安装应用更新，然后由后端直接重启应用。
/// 后台缓存只在版本、签名、下载地址和目标架构均一致时复用。
#[tauri::command]
pub async fn install_update_and_restart(app: AppHandle) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    require_registered_msi_installation(super::misc::is_registered_msi_installation())?;

    let Some(_installing) = crate::app_update::begin_install() else {
        return Err("正在安装更新，请稍候".to_string());
    };
    let checked = match crate::app_update::build_updater(&app)?.check().await {
        Ok(update) => update,
        Err(e) => {
            if let Some((update, bytes)) = crate::app_update::any_pending() {
                log::warn!("检查更新失败（{e}），改装已下载的 {}", update.version);
                return install_and_restart(app, update, bytes).await;
            }
            return Err(format!("检查更新失败: {e}"));
        }
    };
    let Some(update) = checked else {
        crate::app_update::discard_pending(&app);
        return Ok(false);
    };

    #[cfg(target_os = "windows")]
    if !is_owned_msi_url(&update.download_url) {
        return Err("更新包不是本项目发布的 MSI，已停止安装".to_string());
    }

    let bytes = match crate::app_update::pending_bytes_for(&update) {
        Some(bytes) => {
            log::info!("使用已下载的应用更新: {}", update.version);
            bytes
        }
        None => {
            log::info!("开始下载应用更新: {}", update.version);
            let progress_handle = app.clone();
            let mut downloaded: u64 = 0;
            update
                .download(
                    move |chunk_len, content_len| {
                        downloaded = downloaded.saturating_add(chunk_len as u64);
                        let _ = progress_handle.emit(
                            "update-download-progress",
                            UpdateDownloadProgress {
                                downloaded,
                                total: content_len,
                            },
                        );
                    },
                    || {},
                )
                .await
                .map_err(|e| format!("下载更新失败: {e}"))?
        }
    };

    install_and_restart(app, update, bytes).await
}

/// 先验证刚下载或已验签缓存中的安装包，再执行退出清理和安装。
async fn install_and_restart(
    app: AppHandle,
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    require_verified_msi_update(
        &update.download_url,
        &update.signature,
        &update.version,
        &bytes,
        super::misc::is_registered_msi_installation(),
    )?;
    log::info!("开始安装应用更新: {}", update.version);

    #[cfg(target_os = "windows")]
    {
        // Windows updater 会在 install() 内启动安装器并直接退出当前进程
        // （插件内部 std::process::exit(0)，绕过 TrayIcon::drop、不发
        // NIM_DELETE，会残留死图标——与托盘"退出"路径相同的问题）。
        // 因此清理只能放在 install 前执行，且必须显式移除托盘图标。
        crate::save_window_state_before_exit(&app);
        crate::cleanup_before_exit(&app).await;
        // Registration may have changed while cleanup was running. Fail before
        // removing the tray or single-instance lock, and explain the stopped routing.
        require_registered_after_exit_cleanup(super::misc::is_registered_msi_installation())?;
        crate::remove_tray_icon_before_exit(&app);
        crate::destroy_single_instance_lock(&app);
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        update.install(bytes).map_err(|e| {
            format!(
                "Windows 更新安装失败: {e}。已执行退出前清理，代理或 Live 接管可能已暂停；请重启应用或重新开启代理后再试。"
            )
        })?;
        Ok(true)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // macOS/Linux install() 会返回；先安装，避免安装失败时误停代理/撤回接管。
        update
            .install(bytes)
            .map_err(|e| format!("安装更新失败: {e}"))?;

        crate::save_window_state_before_exit(&app);
        crate::cleanup_before_exit(&app).await;

        log::info!("应用更新安装完成，正在重启应用");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        crate::restart_process(&app);
    }
}

/// 自动更新的状态：能不能自动安装、是否在下载、已下好待装的版本。
#[tauri::command]
pub async fn get_auto_update_status(
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<crate::app_update::AutoUpdateStatus, String> {
    Ok(crate::app_update::status(crate::tray::routing_in_use(
        state.inner(),
    )))
}

/// 立刻在后台检查并下载最新版本（打开自动更新开关时调用）。不支持自动安装的机器直接返回空。
#[tauri::command]
pub async fn download_app_update(
    app: AppHandle,
) -> Result<Option<crate::app_update::ReadyUpdate>, String> {
    if !crate::app_update::auto_install_supported() {
        return Ok(None);
    }
    crate::app_update::download_latest(&app).await
}

/// 检查是否有可用的应用更新，返回可用的新版本号（无更新时返回 None）。
///
/// 数据库版本过新的恢复界面用它判断：升级应用能否解决问题。若返回 None，说明
/// 已是最新版本，但数据库仍不兼容（通常由第三方客户端或更高版本创建），应提示用户
/// 升级无法解决，而不是让其反复尝试。
#[tauri::command]
pub async fn check_app_update_available(app: AppHandle) -> Result<Option<String>, String> {
    let updater = app
        .updater_builder()
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;
    Ok(update.map(|u| u.version))
}

/// 获取 app_config_dir 覆盖配置 (从 Store)
#[tauri::command]
pub async fn get_app_config_dir_override(app: AppHandle) -> Result<Option<String>, String> {
    Ok(crate::app_store::read_app_config_dir_override(&app)
        .map_err(|e| e.to_string())?
        .map(|p| p.to_string_lossy().to_string()))
}

/// 返回 CC Switch 的默认数据目录，供设置页显示和重置。
#[tauri::command]
pub async fn get_default_app_config_dir() -> String {
    crate::data_paths::app_dir().to_string_lossy().into_owned()
}

/// 保存下次启动使用的 app_config_dir 覆盖配置。
#[tauri::command]
pub async fn set_app_config_dir_override(
    app: AppHandle,
    path: Option<String>,
) -> Result<bool, String> {
    let path = path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let data_dir = path
        .map(crate::app_store::resolve_path)
        .unwrap_or_else(crate::data_paths::app_dir);
    crate::data_paths::validate_data_dir(&data_dir).map_err(|e| e.to_string())?;
    crate::app_store::set_app_config_dir_to_store(&app, path)?;
    Ok(true)
}

#[tauri::command]
pub async fn set_auto_launch(enabled: bool) -> Result<bool, String> {
    if enabled {
        crate::auto_launch::enable_auto_launch().map_err(|e| format!("启用开机自启失败: {e}"))?;
    } else {
        crate::auto_launch::disable_auto_launch().map_err(|e| format!("禁用开机自启失败: {e}"))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{
        is_msi_payload, is_owned_msi_url, merge_settings_for_save,
        require_registered_after_exit_cleanup, require_registered_msi_installation,
        require_signed_update_version, require_verified_msi_update,
    };
    use crate::settings::{
        AppSettings, CodexOfficialHistoryUnifyMigration, CodexProviderTemplateMigration,
        CodexThirdPartyHistoryProviderBucketMigration, LocalMigrations, S3SyncSettings,
        WebDavSyncSettings,
    };

    // These strings test parsing only; they are deliberately not real signatures.
    // Production invokes the parser exclusively after the updater verifies one.
    fn synthetic_signature(trusted_comment: &str) -> String {
        use base64::Engine;

        let signature = format!(
            "untrusted comment: synthetic\nsynthetic-artifact-signature\ntrusted comment: {trusted_comment}\nsynthetic-global-signature\n"
        );
        base64::engine::general_purpose::STANDARD.encode(signature)
    }

    #[test]
    fn signed_update_version_accepts_exactly_one_matching_version() {
        let signature = synthetic_signature("timestamp:1\tfile:CC Switch.msi\tversion:4.0.32");
        assert!(require_signed_update_version(&signature, "4.0.32").is_ok());

        let prerelease = synthetic_signature(
            "timestamp:1\tfile:with version:in-filename.msi\tversion:4.0.32-beta.1+build.2",
        );
        assert!(require_signed_update_version(&prerelease, "4.0.32-beta.1+build.2").is_ok());
    }

    #[test]
    fn signed_update_version_rejects_missing_duplicate_or_mismatched_versions() {
        for comment in [
            "timestamp:1\tfile:CC Switch.msi",
            "timestamp:1\tfile:CC Switch.msi\tversion:",
            "timestamp:1\tfile:CC Switch.msi\tversion:4.0.31",
            "timestamp:1\tfile:CC Switch.msi\tversion:v4.0.32",
            "timestamp:1\tfile:CC Switch.msi\tversion:4.0.32 ",
            "timestamp:1\tfile:CC Switch.msi\tversion:4.0.32\tversion:4.0.32",
            "timestamp:1\tfile:CC Switch.msi\tversion:4.0.32\t version:4.0.31",
        ] {
            assert!(
                require_signed_update_version(&synthetic_signature(comment), "4.0.32").is_err(),
                "accepted {comment:?}"
            );
        }
    }

    #[test]
    fn signed_update_version_rejects_malformed_signature_containers() {
        use base64::Engine;

        let encode = |value: &[u8]| base64::engine::general_purpose::STANDARD.encode(value);
        for signature in [
            "not base64!".to_string(),
            encode(&[0xff, 0xfe]),
            encode(b"untrusted comment\nartifact\n"),
            encode(b"untrusted comment\nartifact\nversion:4.0.32\nglobal\n"),
            encode(b"untrusted comment\nartifact\ntrusted comment: version:4.0.32\n"),
        ] {
            assert!(require_signed_update_version(&signature, "4.0.32").is_err());
        }
    }

    #[test]
    fn windows_update_rejects_an_unregistered_installation() {
        assert!(require_registered_msi_installation(true).is_ok());
        let error = require_registered_msi_installation(false).unwrap_err();
        assert!(error.contains("https://github.com/WQMYH/multi-compat-cc/releases"));
    }

    #[test]
    fn registration_loss_after_cleanup_reports_possible_routing_interruption() {
        assert!(require_registered_after_exit_cleanup(true).is_ok());
        let error = require_registered_after_exit_cleanup(false).unwrap_err();
        assert!(error.contains("当前安装由外部管理"));
        assert!(error.contains("代理或 Live 接管可能已暂停"));
        assert!(error.contains("请重启应用或重新开启代理"));
    }

    #[test]
    fn downloaded_or_cached_msi_must_pass_every_install_guard() {
        let owned = url::Url::parse(
            "https://github.com/WQMYH/multi-compat-cc/releases/download/v4.0.33/cc-switch.msi",
        )
        .unwrap();
        let foreign = url::Url::parse(
            "https://github.com/farion1231/cc-switch/releases/download/v4.0.33/cc-switch.msi",
        )
        .unwrap();
        let signed = synthetic_signature("timestamp:1\tfile:cc-switch.msi\tversion:4.0.33");
        let msi = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
        assert!(require_verified_msi_update(&owned, &signed, "4.0.33", &msi, true).is_ok());
        assert!(require_verified_msi_update(&owned, &signed, "4.0.33", &msi, false).is_err());
        assert!(require_verified_msi_update(&foreign, &signed, "4.0.33", &msi, true).is_err());
        assert!(require_verified_msi_update(&owned, &signed, "4.0.34", &msi, true).is_err());
        assert!(require_verified_msi_update(&owned, &signed, "4.0.33", b"not-msi", true).is_err());
    }

    #[test]
    fn windows_update_only_accepts_owned_msi_urls() {
        let good = url::Url::parse(
            "https://github.com/WQMYH/multi-compat-cc/releases/download/v4.0.32/cc-switch.msi",
        )
        .unwrap();
        assert!(is_owned_msi_url(&good));
        for bad in [
            "https://github.com/farion1231/cc-switch/releases/download/v4.0.32/cc-switch.msi",
            "https://github.com/WQMYH/multi-compat-cc/releases/download/v4.0.32/cc-switch.exe",
            "https://github.com/WQMYH/multi-compat-cc/releases/download/v4.0.32/cc-switch.msi.zip",
            "https://example.com/WQMYH/multi-compat-cc/releases/download/v4.0.32/cc-switch.msi",
            "http://github.com/WQMYH/multi-compat-cc/releases/download/v4.0.32/cc-switch.msi",
        ] {
            assert!(!is_owned_msi_url(&url::Url::parse(bad).unwrap()), "{bad}");
        }
    }

    #[test]
    fn windows_update_rejects_exe_zip_and_truncated_payloads() {
        assert!(is_msi_payload(&[
            0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1
        ]));
        assert!(!is_msi_payload(b"MZinstaller"));
        assert!(!is_msi_payload(b"PK\x03\x04"));
        assert!(!is_msi_payload(&[0xd0, 0xcf, 0x11]));
    }

    #[test]
    fn save_settings_should_keep_the_existing_lightweight_mode() {
        let existing = AppSettings {
            lightweight_mode: true,
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(AppSettings::default(), &existing);

        assert!(merged.lightweight_mode);
    }

    #[test]
    fn save_settings_should_keep_the_existing_proxy_client_host() {
        let existing = AppSettings {
            proxy_client_host: Some("172.25.144.1".to_string()),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(AppSettings::default(), &existing);

        assert_eq!(merged.proxy_client_host.as_deref(), Some("172.25.144.1"));
    }

    #[test]
    fn save_settings_should_preserve_existing_webdav_when_payload_omits_it() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "secret".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.webdav_sync.is_some());
        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.base_url.as_str()),
            Some("https://dav.example.com")
        );
    }

    #[test]
    fn save_settings_should_keep_incoming_webdav_when_present() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.old.example.com".to_string(),
                username: "old".to_string(),
                password: "old-pass".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.new.example.com".to_string(),
                username: "new".to_string(),
                password: "new-pass".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.base_url.as_str()),
            Some("https://dav.new.example.com")
        );
    }

    /// Regression test: frontend always receives empty password from
    /// get_settings_for_frontend(). If a component accidentally spreads
    /// the full settings object into save_settings, the empty password
    /// must NOT overwrite the existing one.
    #[test]
    fn save_settings_should_preserve_password_when_incoming_has_empty_password() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "secret".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        // Simulate frontend sending settings with cleared password
        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.password.as_str()),
            Some("secret"),
            "empty password from frontend must not overwrite existing password"
        );
    }

    /// When both incoming and existing have no password, merge should
    /// work without panicking and keep the empty state.
    #[test]
    fn save_settings_should_handle_both_empty_passwords() {
        let existing = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            webdav_sync: Some(WebDavSyncSettings {
                base_url: "https://dav.example.com".to_string(),
                username: "alice".to_string(),
                password: "".to_string(),
                ..WebDavSyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged.webdav_sync.as_ref().map(|v| v.password.as_str()),
            Some("")
        );
    }

    #[test]
    fn save_settings_should_preserve_existing_s3_when_payload_omits_it() {
        let existing = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "secret".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.s3_sync.is_some());
        assert_eq!(
            merged
                .s3_sync
                .as_ref()
                .map(|v| v.secret_access_key.as_str()),
            Some("secret")
        );
    }

    #[test]
    fn save_settings_should_preserve_s3_secret_when_incoming_has_empty_secret() {
        let existing = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "secret".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            s3_sync: Some(S3SyncSettings {
                bucket: "bucket".to_string(),
                access_key_id: "ak".to_string(),
                secret_access_key: "".to_string(),
                ..S3SyncSettings::default()
            }),
            ..AppSettings::default()
        };

        let merged = merge_settings_for_save(incoming, &existing);

        assert_eq!(
            merged
                .s3_sync
                .as_ref()
                .map(|v| v.secret_access_key.as_str()),
            Some("secret")
        );
    }

    #[test]
    fn save_settings_should_preserve_local_migrations_when_payload_omits_it() {
        let existing = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_third_party_history_provider_bucket_v1: Some(
                    CodexThirdPartyHistoryProviderBucketMigration {
                        completed_at: "2026-05-20T00:00:00Z".to_string(),
                        target_provider_id: "custom".to_string(),
                        source_provider_ids: vec!["rightcode".to_string()],
                        migrated_jsonl_files: 2,
                        migrated_state_rows: 3,
                        scanned_history_files: true,
                    },
                ),
                codex_provider_template_v1: Some(CodexProviderTemplateMigration {
                    completed_at: "2026-05-20T00:01:00Z".to_string(),
                    migrated_provider_ids: vec!["legacy".to_string()],
                }),
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    migrated_jsonl_files: 5,
                    migrated_state_rows: 7,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings::default();
        let merged = merge_settings_for_save(incoming, &existing);

        let migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| {
                migrations
                    .codex_third_party_history_provider_bucket_v1
                    .as_ref()
            })
            .expect("local migration marker should be preserved");
        assert_eq!(migration.target_provider_id, "custom");
        assert_eq!(migration.migrated_jsonl_files, 2);
        assert_eq!(migration.migrated_state_rows, 3);

        let template_migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_provider_template_v1.as_ref())
            .expect("template migration marker should be preserved");
        assert_eq!(
            template_migration.migrated_provider_ids,
            vec!["legacy".to_string()]
        );

        let unify_migration = merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_official_history_unify_v1.as_ref())
            .expect("official unify migration marker should be preserved");
        assert_eq!(unify_migration.migrated_jsonl_files, 5);
        assert_eq!(unify_migration.migrated_state_rows, 7);
    }

    /// incoming 带有 local_migrations（哪怕是空的）也不能覆盖后端维护的标记。
    #[test]
    fn save_settings_should_keep_backend_migration_markers_over_incoming() {
        let existing = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_third_party_history_provider_bucket_v1: None,
                codex_provider_template_v1: None,
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    migrated_jsonl_files: 1,
                    migrated_state_rows: 2,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };

        let incoming = AppSettings {
            local_migrations: Some(LocalMigrations::default()),
            ..AppSettings::default()
        };
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged
            .local_migrations
            .as_ref()
            .and_then(|migrations| migrations.codex_official_history_unify_v1.as_ref())
            .is_some());
    }

    /// 后端清掉 marker 后（如关闭统一会话开关）、前端缓存刷新前的全量保存
    /// 会携带旧 marker；merge 必须忽略它，否则被"复活"的标记会让重新开启
    /// 时误判已迁移而漏迁。
    #[test]
    fn save_settings_should_ignore_stale_incoming_migration_markers() {
        let existing = AppSettings::default();

        let incoming = AppSettings {
            local_migrations: Some(LocalMigrations {
                codex_official_history_unify_v1: Some(CodexOfficialHistoryUnifyMigration {
                    completed_at: "2026-06-12T00:00:00Z".to_string(),
                    target_provider_id: "custom".to_string(),
                    migrated_jsonl_files: 1,
                    migrated_state_rows: 2,
                    codex_config_dir: None,
                }),
                ..LocalMigrations::default()
            }),
            ..AppSettings::default()
        };
        let merged = merge_settings_for_save(incoming, &existing);

        assert!(merged.local_migrations.is_none());
    }
}

#[tauri::command]
pub async fn get_auto_launch_status() -> Result<bool, String> {
    crate::auto_launch::is_auto_launch_enabled().map_err(|e| format!("获取开机自启状态失败: {e}"))
}

#[tauri::command]
pub async fn get_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::RectifierConfig, String> {
    state.db.get_rectifier_config().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_rectifier_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::RectifierConfig,
) -> Result<bool, String> {
    state
        .db
        .set_rectifier_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn get_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::OptimizerConfig, String> {
    state.db.get_optimizer_config().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::OptimizerConfig,
) -> Result<bool, String> {
    state
        .db
        .set_optimizer_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn get_privacy_filter_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::PrivacyFilterConfig, String> {
    state
        .db
        .get_privacy_filter_config()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_privacy_filter_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::PrivacyFilterConfig,
) -> Result<bool, String> {
    state
        .db
        .set_privacy_filter_config(&config)
        .map_err(|e| e.to_string())?;
    if config.enabled {
        // 规则编译有一次性耗时，开关打开时在后台预热，不压到第一个请求上
        std::thread::spawn(crate::proxy::privacy_filter::warm_up);
    }
    Ok(true)
}

#[tauri::command]
pub async fn get_copilot_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::CopilotOptimizerConfig, String> {
    state
        .db
        .get_copilot_optimizer_config()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_copilot_optimizer_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::CopilotOptimizerConfig,
) -> Result<bool, String> {
    state
        .db
        .set_copilot_optimizer_config(&config)
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn get_log_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::proxy::types::LogConfig, String> {
    state.db.get_log_config().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_log_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::proxy::types::LogConfig,
) -> Result<bool, String> {
    state
        .db
        .set_log_config(&config)
        .map_err(|e| e.to_string())?;
    log::set_max_level(config.to_level_filter());
    log::info!(
        "日志配置已更新: enabled={}, level={}",
        config.enabled,
        config.level
    );
    Ok(true)
}
