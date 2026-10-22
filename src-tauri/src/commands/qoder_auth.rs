use crate::{AppState, Database};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::Duration,
};
use tauri::{Manager, State};

const STORE_KEY: &str = "qoder_cn_auth_profiles_v1";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderRuntime {
    cli_path: Option<String>,
    config_dir: Option<String>,
}

#[tauri::command]
pub async fn qoder_auth_detect_runtime() -> Result<QoderRuntime, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cli = match super::native_application::qoder::cli_runtime() {
            Ok(path) => Some(path),
            Err(code) if code.starts_with("QODER_SDK_RUNTIME_NOT_FOUND:") => None,
            Err(code) => return Err(code),
        };
        let config_dir = super::native_application::config_root("qoder")?;
        Ok(QoderRuntime {
            cli_path: cli.map(|path| path.to_string_lossy().into_owned()),
            config_dir: Some(config_dir.to_string_lossy().into_owned()),
        })
    })
    .await
    .map_err(|_| "QODER_RUNTIME_DISCOVERY_FAILED".to_string())?
}
// shortcut: serialize local profile edits; use DAO transactions if multiple CC writers are supported.
static PROFILE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum QoderAuthSource {
    #[serde(rename = "pat_env")]
    PatEnv {
        #[serde(rename = "envVar")]
        env_var: String,
    },
    #[serde(rename = "cli")]
    Cli {
        #[serde(rename = "configDir")]
        config_dir: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QoderAccountIdentity {
    user_id: String,
    name: Option<String>,
    email: Option<String>,
    organization_name: Option<String>,
    subscription_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QoderAuthProfileInput {
    id: Option<String>,
    label: String,
    source: QoderAuthSource,
    cli_path: String,
    node_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderAuthProfile {
    id: String,
    label: String,
    source: QoderAuthSource,
    cli_path: String,
    node_path: Option<String>,
    revision: u64,
    account: Option<QoderAccountIdentity>,
    verified_at: Option<String>,
    last_error: Option<String>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderAuthProfiles {
    profiles: Vec<QoderAuthProfile>,
    selected_id: Option<String>,
}

fn load(db: &Database) -> Result<QoderAuthProfiles, String> {
    db.get_setting(STORE_KEY)
        .map_err(|e| e.to_string())?
        .map(|value| {
            serde_json::from_str(&value).map_err(|_| "QODER_PROFILE_STORE_INVALID".to_string())
        })
        .unwrap_or_else(|| Ok(QoderAuthProfiles::default()))
}

fn persist(db: &Database, store: &QoderAuthProfiles) -> Result<(), String> {
    db.set_setting(
        STORE_KEY,
        &serde_json::to_string(store).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn validate(input: &QoderAuthProfileInput) -> Result<(), String> {
    if input.label.trim().is_empty() || input.label.len() > 256 {
        return Err("QODER_PROFILE_LABEL_INVALID".into());
    }
    for file in [Some(input.cli_path.as_str()), input.node_path.as_deref()]
        .into_iter()
        .flatten()
    {
        if !Path::new(file).is_absolute() || !Path::new(file).is_file() {
            return Err("QODER_EXECUTABLE_PATH_INVALID".into());
        }
    }
    match &input.source {
        QoderAuthSource::PatEnv { env_var } => {
            let mut chars = env_var.chars();
            if env_var.len() > 128
                || !chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err("QODER_ENV_REFERENCE_INVALID".into());
            }
        }
        QoderAuthSource::Cli { config_dir } => {
            if !Path::new(config_dir).is_absolute() || !Path::new(config_dir).is_dir() {
                return Err("QODER_CONFIG_DIRECTORY_REQUIRED".into());
            }
        }
    }
    Ok(())
}

fn save_profile(db: &Database, input: QoderAuthProfileInput) -> Result<QoderAuthProfiles, String> {
    validate(&input)?;
    let _guard = PROFILE_LOCK
        .lock()
        .map_err(|_| "QODER_PROFILE_LOCK_FAILED")?;
    let mut store = load(db)?;
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let revision = if input.id.is_some() {
        let prior = store
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or("QODER_PROFILE_NOT_FOUND")?;
        prior
            .revision
            .checked_add(1)
            .ok_or("QODER_PROFILE_REVISION_EXHAUSTED")?
    } else {
        1
    };
    store.profiles.retain(|profile| profile.id != id);
    store.profiles.push(QoderAuthProfile {
        id: id.clone(),
        label: input.label.trim().into(),
        source: input.source,
        cli_path: input.cli_path,
        node_path: input.node_path,
        revision,
        account: None,
        verified_at: None,
        last_error: None,
    });
    if store.selected_id.as_deref() == Some(&id) {
        store.selected_id = None;
    }
    persist(db, &store)?;
    Ok(store)
}

fn select_profile(db: &Database, id: Option<String>) -> Result<QoderAuthProfiles, String> {
    let _guard = PROFILE_LOCK
        .lock()
        .map_err(|_| "QODER_PROFILE_LOCK_FAILED")?;
    let mut store = load(db)?;
    if let Some(id) = id.as_deref() {
        let profile = store
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .ok_or("QODER_PROFILE_NOT_FOUND")?;
        if profile.account.is_none() || profile.last_error.is_some() {
            return Err("QODER_ACCOUNT_VERIFICATION_REQUIRED".into());
        }
    }
    store.selected_id = id;
    persist(db, &store)?;
    Ok(store)
}

fn remove_profile(db: &Database, id: &str) -> Result<QoderAuthProfiles, String> {
    let _guard = PROFILE_LOCK
        .lock()
        .map_err(|_| "QODER_PROFILE_LOCK_FAILED")?;
    let mut store = load(db)?;
    if !store.profiles.iter().any(|profile| profile.id == id) {
        return Err("QODER_PROFILE_NOT_FOUND".into());
    }
    store.profiles.retain(|profile| profile.id != id);
    if store.selected_id.as_deref() == Some(id) {
        store.selected_id = None;
    }
    persist(db, &store)?;
    Ok(store)
}

fn account_terminal_command(profile: &QoderAuthProfile) -> Result<String, String> {
    let QoderAuthSource::Cli { config_dir } = &profile.source else {
        return Err("QODER_NATIVE_CLI_PROFILE_REQUIRED".into());
    };
    validate(&QoderAuthProfileInput {
        id: Some(profile.id.clone()),
        label: profile.label.clone(),
        source: profile.source.clone(),
        cli_path: profile.cli_path.clone(),
        node_path: profile.node_path.clone(),
    })?;
    let mut args = Vec::new();
    if Path::new(&profile.cli_path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            ["js", "mjs", "cjs"]
                .iter()
                .any(|ext| value.eq_ignore_ascii_case(ext))
        })
    {
        let node = match &profile.node_path {
            Some(path) => PathBuf::from(path),
            None => super::misc::resolve_path_default(
                "node",
                super::misc::CommandDeadline::from_timeout(Some(Duration::from_secs(10))),
            )?
            .ok_or("QODER_NODE_NOT_FOUND")?,
        };
        args.push(node.to_string_lossy().into_owned());
    }
    args.push(profile.cli_path.clone());
    let hint = "Complete /login in the official CLI, check /status, then return to CC Switch. No provider synchronization or model request is started automatically.";
    #[cfg(target_os = "windows")]
    {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
        let invocation = args
            .iter()
            .map(|value| quote(value))
            .collect::<Vec<_>>()
            .join(" ");
        let invocation = if Path::new(&profile.cli_path)
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                ["cmd", "bat"]
                    .iter()
                    .any(|ext| value.eq_ignore_ascii_case(ext))
            }) {
            // CMD expands this variable once; CALL or a literal batch path would expand embedded %VAR% again.
            format!(
                "$env:CC_SWITCH_QODER_ACCOUNT_CLI = {}\n& $env:ComSpec /d /v:off /s /c '\"\"%CC_SWITCH_QODER_ACCOUNT_CLI%\"\"'",
                quote(&profile.cli_path),
            )
        } else {
            format!("& {invocation}")
        };
        // Only encoded PowerShell enters the existing batch launcher, never interpolated user paths.
        let script = format!(
            "$ErrorActionPreference = 'Stop'\n$env:QODERCN_PERSONAL_ACCESS_TOKEN = $null\n$env:QODERCN_SERVICE_ACCOUNT_KEY = $null\n$env:QODER_PERSONAL_ACCESS_TOKEN = $null\n$env:QODER_SERVICE_ACCOUNT_KEY = $null\n$env:QODERCN_CONFIG_DIR = {}\nWrite-Host {}\n{}\nexit $LASTEXITCODE\n",
            quote(config_dir), quote(hint), invocation,
        );
        let encoded = STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        Ok(format!(
            "powershell.exe -NoLogo -NoProfile -EncodedCommand {encoded}"
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let invocation = args
            .iter()
            .map(|value| quote(value))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(format!(
            "printf '%s\\n' {}; env -u QODERCN_PERSONAL_ACCESS_TOKEN -u QODERCN_SERVICE_ACCOUNT_KEY -u QODER_PERSONAL_ACCESS_TOKEN -u QODER_SERVICE_ACCOUNT_KEY QODERCN_CONFIG_DIR={} {}",
            quote(hint), quote(config_dir), invocation,
        ))
    }
}

#[derive(Deserialize)]
struct ProbeReply {
    ok: bool,
    account: Option<QoderAccountIdentity>,
    code: Option<String>,
}

fn run_probe(profile: &QoderAuthProfile, helper: &Path) -> Result<QoderAccountIdentity, String> {
    run_probe_with_timeout(profile, helper, Duration::from_secs(125))
}

fn run_probe_with_timeout(
    profile: &QoderAuthProfile,
    helper: &Path,
    timeout: Duration,
) -> Result<QoderAccountIdentity, String> {
    let input = serde_json::json!({
        "cliPath": profile.cli_path, "source": profile.source,
        "expectedUserId": profile.account.as_ref().map(|account| account.user_id.as_str()),
    });
    let output = run_sdk_host(input, profile.node_path.as_deref(), Some(helper), timeout)?;
    if output.stdout.len() > 16384 {
        return Err("QODER_HOST_REPLY_TOO_LARGE".into());
    }
    let reply: ProbeReply =
        serde_json::from_slice(&output.stdout).map_err(|_| "QODER_HOST_REPLY_INVALID")?;
    if !reply.ok || !output.status.success() {
        return Err(reply
            .code
            .filter(|code| {
                code.len() <= 64
                    && code
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            })
            .unwrap_or_else(|| "QODER_ACCOUNT_PROBE_FAILED".into()));
    }
    let account = reply.account.ok_or("QODER_ACCOUNT_UNVERIFIED")?;
    if account.user_id.trim().is_empty() || account.user_id.len() > 256 {
        return Err("QODER_ACCOUNT_UNVERIFIED".into());
    }
    if profile.account.as_ref().is_some_and(|prior| prior.user_id != account.user_id) {
        return Err("QODER_ACCOUNT_IDENTITY_CHANGED".into());
    }
    Ok(account)
}

pub(super) fn run_sdk_host(
    mut input: serde_json::Value,
    node_path: Option<&str>,
    helper: Option<&Path>,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let deadline = super::misc::CommandDeadline::from_timeout(Some(timeout));
    let node = match node_path {
        Some(path) => PathBuf::from(path),
        None => {
            super::misc::resolve_path_default("node", deadline)?.ok_or("QODER_NODE_NOT_FOUND")?
        }
    };
    let temporary = tempfile::Builder::new()
        .prefix("cc-switch-qoder-")
        .tempdir()
        .map_err(|_| "QODER_TEMP_DIRECTORY_FAILED")?;
    let embedded = temporary.path().join("qoder-auth-host.mjs");
    let helper = match helper {
        Some(path) => path,
        None => {
            crate::config::write_text_file_private(&embedded, include_str!("../../resources/qoder/qoder-auth-host.mjs"))
                .map_err(|_| "QODER_HOST_MATERIALIZATION_FAILED")?;
            &embedded
        }
    };
    input["scratchDir"] = serde_json::json!(temporary.path());
    let mut command = Command::new(node);
    command
        .arg(helper)
        .current_dir(temporary.path())
        // The SDK also writes one-shot auth payloads through os.tmpdir().
        .env("TEMP", temporary.path())
        .env("TMP", temporary.path())
        .env("TMPDIR", temporary.path())
        .env_remove("NODE_OPTIONS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    super::misc::isolate_child_process_group(&mut command);
    let mut child = command.spawn().map_err(|_| "QODER_HOST_START_FAILED")?;
    let written = child
        .stdin
        .take()
        .ok_or("QODER_HOST_INPUT_FAILED")?
        .write_all(input.to_string().as_bytes());
    if written.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("QODER_HOST_INPUT_FAILED".into());
    }
    let output = super::misc::wait_child_output(child, deadline).map_err(|error| {
        if error.starts_with("Command timed out after ") {
            "QODER_HOST_TIMEOUT"
        } else {
            "QODER_HOST_EXECUTION_FAILED"
        }
    })?;
    if output.stdout.len() > 131072 {
        return Err("QODER_HOST_REPLY_TOO_LARGE".into());
    }
    Ok(output)
}

fn finish_probe(
    db: &Database,
    snapshot: &QoderAuthProfile,
    result: Result<QoderAccountIdentity, String>,
) -> Result<QoderAuthProfiles, String> {
    let _guard = PROFILE_LOCK
        .lock()
        .map_err(|_| "QODER_PROFILE_LOCK_FAILED")?;
    let mut store = load(db)?;
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == snapshot.id && profile.revision == snapshot.revision)
        .ok_or("QODER_PROFILE_CHANGED_DURING_PROBE")?;
    match result {
        Ok(account) => {
            profile.account = Some(account);
            profile.verified_at = Some(chrono::Utc::now().to_rfc3339());
            profile.last_error = None;
        }
        Err(code) => {
            let invalid_identity = matches!(
                code.as_str(),
                "QODER_CREDENTIAL_MISSING"
                    | "QODER_ACCOUNT_IDENTITY_CHANGED"
                    | "QODER_ACCOUNT_UNVERIFIED"
                    | "AUTH_INVALID"
                    | "AUTH_EXPIRED"
                    | "AUTH_REQUIRED"
                    | "UNAUTHORIZED"
                    | "TOKEN_EXPIRED"
                    | "INVALID_TOKEN"
            );
            profile.last_error = Some(code);
            // A transport failure is not evidence that the selected account expired.
            if invalid_identity && store.selected_id.as_deref() == Some(&profile.id) {
                store.selected_id = None;
            }
        }
    }
    profile.revision = profile
        .revision
        .checked_add(1)
        .ok_or("QODER_PROFILE_REVISION_EXHAUSTED")?;
    persist(db, &store)?;
    Ok(store)
}

#[tauri::command]
pub fn qoder_auth_list_profiles(state: State<'_, AppState>) -> Result<QoderAuthProfiles, String> {
    load(&state.db)
}

#[tauri::command(rename_all = "camelCase")]
pub fn qoder_auth_save_profile(
    input: QoderAuthProfileInput,
    state: State<'_, AppState>,
) -> Result<QoderAuthProfiles, String> {
    save_profile(&state.db, input)
}

#[tauri::command(rename_all = "camelCase")]
pub fn qoder_auth_select_profile(
    id: Option<String>,
    state: State<'_, AppState>,
) -> Result<QoderAuthProfiles, String> {
    select_profile(&state.db, id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn qoder_auth_remove_profile(
    id: String,
    state: State<'_, AppState>,
) -> Result<QoderAuthProfiles, String> {
    remove_profile(&state.db, &id)
}

pub(super) fn account_cli_command(db: &Database, id: Option<&str>) -> Result<String, String> {
    let profile = match id {
        Some(id) => load(db)?.profiles.into_iter().find(|profile| profile.id == id)
            .ok_or("QODER_PROFILE_NOT_FOUND")?,
        // This transient descriptor is never saved or selected as an account profile.
        None => QoderAuthProfile {
            id: String::new(), label: "Qoder CN".into(),
            source: QoderAuthSource::Cli {
                config_dir: super::native_application::config_root("qoder")?.to_string_lossy().into_owned(),
            },
            cli_path: super::native_application::qoder::cli_runtime()?.to_string_lossy().into_owned(),
            node_path: None, revision: 0, account: None, verified_at: None, last_error: None,
        },
    };
    account_terminal_command(&profile)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn qoder_auth_open_cli(id: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        let command = account_cli_command(&db, id.as_deref())?;
        super::misc::launch_terminal_running(
            &command,
            &format!("qoder_account_{}", uuid::Uuid::new_v4().simple()),
        )
    })
    .await
    .map_err(|_| "QODER_CLI_LAUNCH_JOIN_FAILED".to_string())?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn qoder_auth_verify_profile(
    id: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<QoderAuthProfiles, String> {
    let snapshot = load(&state.db)?
        .profiles
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or("QODER_PROFILE_NOT_FOUND")?;
    let helper = if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/qoder/qoder-auth-host.mjs")
    } else {
        app.path()
            .resource_dir()
            .map_err(|_| "QODER_HOST_RESOURCE_MISSING")?
            .join("qoder/qoder-auth-host.mjs")
    };
    if !helper.is_file() {
        return Err("QODER_HOST_RESOURCE_MISSING".into());
    }
    let copy = snapshot.clone();
    let result = tauri::async_runtime::spawn_blocking(move || run_probe(&copy, &helper))
        .await
        .map_err(|_| "QODER_HOST_JOIN_FAILED".to_string())?;
    finish_probe(&state.db, &snapshot, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(directory: &Path, label: &str) -> QoderAuthProfileInput {
        let executable = directory.join("synthetic-cli.mjs");
        std::fs::write(&executable, "// not executed").unwrap();
        QoderAuthProfileInput {
            id: None,
            label: label.into(),
            source: QoderAuthSource::PatEnv {
                env_var: "QODER_FIXTURE_TOKEN".into(),
            },
            cli_path: executable.to_string_lossy().into(),
            node_path: None,
        }
    }
    fn identity(id: &str) -> QoderAccountIdentity {
        QoderAccountIdentity {
            user_id: id.into(),
            name: None,
            email: None,
            organization_name: None,
            subscription_type: None,
        }
    }

    #[test]
    fn profiles_require_verification_and_unlink_never_touches_other_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        db.set_setting("codex_account_fixture", "unchanged")
            .unwrap();
        let a = save_profile(&db, input(temp.path(), "A")).unwrap().profiles[0].clone();
        let b = save_profile(&db, input(temp.path(), "B")).unwrap().profiles[1].clone();
        assert!(select_profile(&db, Some(a.id.clone())).is_err());
        finish_probe(&db, &a, Ok(identity("account-A"))).unwrap();
        finish_probe(&db, &b, Ok(identity("account-B"))).unwrap();
        select_profile(&db, Some(a.id.clone())).unwrap();
        let removed = remove_profile(&db, &a.id).unwrap();
        assert!(removed.selected_id.is_none());
        assert_eq!(
            removed.profiles[0].account.as_ref().unwrap().user_id,
            "account-B"
        );
        assert_eq!(
            db.get_setting("codex_account_fixture").unwrap().as_deref(),
            Some("unchanged")
        );
        assert!(temp.path().join("synthetic-cli.mjs").is_file());
    }

    #[test]
    fn delayed_probe_cannot_restore_edited_or_removed_profile() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let prior = save_profile(&db, input(temp.path(), "A")).unwrap().profiles[0].clone();
        let mut edited = input(temp.path(), "B");
        edited.id = Some(prior.id.clone());
        let current = save_profile(&db, edited).unwrap().profiles[0].clone();
        assert!(finish_probe(&db, &prior, Ok(identity("stale-A"))).is_err());
        assert!(load(&db).unwrap().profiles[0].account.is_none());
        remove_profile(&db, &current.id).unwrap();
        assert!(finish_probe(&db, &current, Ok(identity("late-B"))).is_err());
        assert!(load(&db).unwrap().profiles.is_empty());
    }

    #[test]
    fn failed_probe_clears_selection_without_switching_or_deleting_identity_history() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let profile = save_profile(&db, input(temp.path(), "A")).unwrap().profiles[0].clone();
        finish_probe(&db, &profile, Ok(identity("account-A"))).unwrap();
        select_profile(&db, Some(profile.id.clone())).unwrap();
        let current = load(&db).unwrap().profiles[0].clone();
        let store = finish_probe(&db, &current, Err("AUTH_EXPIRED".into())).unwrap();
        assert!(store.selected_id.is_none());
        assert_eq!(
            store.profiles[0].account.as_ref().unwrap().user_id,
            "account-A"
        );
        assert!(select_profile(&db, Some(profile.id)).is_err());
    }

    #[test]
    fn competing_probes_cannot_overwrite_the_accepted_identity() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let snapshot = save_profile(&db, input(temp.path(), "A")).unwrap().profiles[0].clone();
        finish_probe(&db, &snapshot, Ok(identity("account-A"))).unwrap();
        select_profile(&db, Some(snapshot.id.clone())).unwrap();
        assert!(finish_probe(&db, &snapshot, Ok(identity("account-B"))).is_err());
        let store = load(&db).unwrap();
        assert_eq!(store.selected_id.as_deref(), Some(snapshot.id.as_str()));
        assert_eq!(
            store.profiles[0].account.as_ref().unwrap().user_id,
            "account-A"
        );
    }

    #[test]
    fn unavailable_host_does_not_erase_the_selected_identity() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let profile = save_profile(&db, input(temp.path(), "A")).unwrap().profiles[0].clone();
        finish_probe(&db, &profile, Ok(identity("account-A"))).unwrap();
        select_profile(&db, Some(profile.id.clone())).unwrap();
        let current = load(&db).unwrap().profiles[0].clone();
        let store = finish_probe(&db, &current, Err("QODER_HOST_TIMEOUT".into())).unwrap();
        assert_eq!(store.selected_id.as_deref(), Some(profile.id.as_str()));
        assert_eq!(
            store.profiles[0].last_error.as_deref(),
            Some("QODER_HOST_TIMEOUT")
        );
        assert_eq!(
            store.profiles[0].account.as_ref().unwrap().user_id,
            "account-A"
        );
    }

    #[test]
    fn account_terminal_rejects_pat_and_preserves_profile_state() {
        let temp = tempfile::tempdir().unwrap();
        let db = Database::memory().unwrap();
        let stored = save_profile(&db, input(temp.path(), "PAT fixture")).unwrap();
        let before = serde_json::to_value(&stored).unwrap();
        assert_eq!(
            account_terminal_command(&stored.profiles[0]).unwrap_err(),
            "QODER_NATIVE_CLI_PROFILE_REQUIRED"
        );
        assert_eq!(serde_json::to_value(load(&db).unwrap()).unwrap(), before);
    }

    #[test]
    fn account_terminal_runs_exact_native_config_without_ambient_auth_or_prompt() {
        let temp = tempfile::Builder::new()
            .prefix("qoder_cli_甲_% !' & ")
            .tempdir()
            .unwrap();
        let config = temp.path().join("native config ' & ! %");
        std::fs::create_dir(&config).unwrap();
        let sentinel = config.join("native-credentials.json");
        std::fs::write(&sentinel, "synthetic unchanged").unwrap();
        let cli = temp.path().join("synthetic cli ' & ! %.mjs");
        std::fs::write(
            &cli,
            r#"
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
writeFileSync(join(process.env.QODERCN_CONFIG_DIR, 'observed.json'), JSON.stringify({
  configDir: process.env.QODERCN_CONFIG_DIR,
  args: process.argv.slice(2),
  ambientAuth: ['QODERCN_PERSONAL_ACCESS_TOKEN', 'QODERCN_SERVICE_ACCOUNT_KEY',
    'QODER_PERSONAL_ACCESS_TOKEN', 'QODER_SERVICE_ACCOUNT_KEY'].some(name => !!process.env[name])
}));
"#,
        )
        .unwrap();
        let profile = QoderAuthProfile {
            id: "native-terminal-fixture".into(),
            label: "Native terminal fixture".into(),
            source: QoderAuthSource::Cli {
                config_dir: config.to_string_lossy().into(),
            },
            cli_path: cli.to_string_lossy().into(),
            node_path: None,
            revision: 1,
            account: None,
            verified_at: None,
            last_error: None,
        };
        let line = account_terminal_command(&profile).unwrap();
        #[cfg(target_os = "windows")]
        let mut command = {
            use std::os::windows::process::CommandExt;
            let encoded = line
                .strip_prefix("powershell.exe -NoLogo -NoProfile -EncodedCommand ")
                .unwrap();
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                encoded,
            ]);
            command.creation_flags(0x08000000);
            command
        };
        #[cfg(not(target_os = "windows"))]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", &line]);
            command
        };
        for name in [
            "QODERCN_PERSONAL_ACCESS_TOKEN",
            "QODERCN_SERVICE_ACCOUNT_KEY",
            "QODER_PERSONAL_ACCESS_TOKEN",
            "QODER_SERVICE_ACCOUNT_KEY",
        ] {
            command.env(name, "synthetic-must-not-reach-cli");
        }
        let output = command
            .env("QODERCN_CONFIG_DIR", "ambient-must-not-be-used")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let observed: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config.join("observed.json")).unwrap()).unwrap();
        assert_eq!(observed["configDir"], config.to_string_lossy().as_ref());
        assert_eq!(observed["args"], serde_json::json!([]));
        assert_eq!(observed["ambientAuth"], false);
        assert_eq!(
            std::fs::read_to_string(&sentinel).unwrap(),
            "synthetic unchanged"
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-must-not-reach-cli"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn account_terminal_keeps_literal_batch_path_without_cmd_reexpansion() {
        use std::os::windows::process::CommandExt;
        let temp = tempfile::Builder::new()
            .prefix("qoder_batch_甲_ ' !& ")
            .tempdir()
            .unwrap();
        let literal = temp.path().join("%QODER_ACCOUNT_MARKER%");
        let expanded = temp.path().join("expanded");
        std::fs::create_dir(&literal).unwrap();
        std::fs::create_dir(&expanded).unwrap();
        for extension in ["cmd", "bat"] {
            let filename = format!("synthetic.{extension}");
            std::fs::write(literal.join(&filename), "@echo off\r\necho literal-cli\r\n").unwrap();
            std::fs::write(
                expanded.join(&filename),
                "@echo off\r\necho wrong-expanded-cli\r\n",
            )
            .unwrap();
            let profile = QoderAuthProfile {
                id: "literal-batch-fixture".into(),
                label: "Literal batch fixture".into(),
                source: QoderAuthSource::Cli {
                    config_dir: temp.path().to_string_lossy().into(),
                },
                cli_path: literal.join(&filename).to_string_lossy().into(),
                node_path: None,
                revision: 1,
                account: None,
                verified_at: None,
                last_error: None,
            };
            let line = account_terminal_command(&profile).unwrap();
            let encoded = line
                .strip_prefix("powershell.exe -NoLogo -NoProfile -EncodedCommand ")
                .unwrap();
            let output = Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-EncodedCommand",
                    encoded,
                ])
                .env("QODER_ACCOUNT_MARKER", "expanded")
                .stdin(Stdio::null())
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.lines().any(|line| line == "literal-cli"), "{stdout}");
            assert!(!stdout.contains("wrong-expanded-cli"), "{stdout}");
        }
    }

    #[test]
    fn host_deadline_reclaims_parent_owned_sdk_material_after_native_start() {
        let config = tempfile::tempdir().unwrap();
        std::fs::write(config.path().join("hold-account"), "hold").unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let profile = QoderAuthProfile {
            id: "deadline-fixture".into(),
            label: "Synthetic native CLI".into(),
            source: QoderAuthSource::Cli {
                config_dir: config.path().to_string_lossy().into(),
            },
            cli_path: root
                .join("../compat/qoder-cn/sdk-contract/host-fixture.mjs")
                .to_string_lossy()
                .into(),
            node_path: None,
            revision: 1,
            account: None,
            verified_at: None,
            last_error: None,
        };
        let helper = root.join("resources/qoder/qoder-auth-host.mjs");
        assert!(
            helper.is_file(),
            "build the production helper before this test"
        );
        let result = run_probe_with_timeout(&profile, &helper, Duration::from_secs(12));
        assert_eq!(result.unwrap_err(), "QODER_HOST_TIMEOUT");
        assert!(
            config.path().join("ready.json").is_file(),
            "the native endpoint must reach accountInfo before the timeout"
        );
        let child: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config.path().join("child.json")).unwrap())
                .unwrap();
        let scratch = PathBuf::from(child["temporaryDirectory"].as_str().unwrap());
        assert!(scratch
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("cc-switch-qoder-"));
        assert!(
            !scratch.exists(),
            "the Rust parent must remove SDK scratch even when helper finally never runs"
        );
        assert!(
            config.path().join("hold-account").is_file(),
            "explicit native config must remain untouched"
        );
    }
}
