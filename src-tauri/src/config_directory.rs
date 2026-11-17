//! Visible, read-only configuration-directory discovery.
//!
//! Upstream #3101 was closed without merging an implicit process-environment
//! fallback: a GUI and a terminal may have different environments. Discovery
//! therefore never changes a resolver, saves settings, migrates files, or reads
//! configuration contents. The existing directory override is the adoption path.
//! See https://github.com/farion1231/cc-switch/pull/3101.
//!
//! Only documented directory variables are candidates. OPENCODE_CONFIG is an
//! additional configuration *file*, OPENCODE_CONFIG_DIR is an additional resource
//! directory, and OPENCLAW_CONFIG_PATH may be outside its state directory. Their
//! parent directories cannot faithfully represent the existing override contract.
//! We do not scan drives, WSL distributions, installations, or other users.
//!
//! Directory semantics: https://code.claude.com/docs/en/settings,
//! https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/paths.ts,
//! https://docs.x.ai/build/settings, https://docs.openclaw.ai/help/environment,
//! https://dev.opencode.ai/v2/docs/cli/config,
//! https://hermes-agent.nousresearch.com/docs/user-guide/windows-native,
//! https://github.com/can1357/oh-my-pi/blob/main/docs/environment-variables.md.
//! XDG_CONFIG_HOME is a directory candidate only: its existence does not validate
//! OpenCode v2's cli.json or extend our existing json/jsonc writer.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::app_config::AppType;
use crate::error::AppError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDirectoryDetection {
    pub current_dir: String,
    pub default_dir: String,
    pub candidates: Vec<ConfigDirectoryCandidate>,
}

#[derive(Debug, Serialize)]
pub struct ConfigDirectoryCandidate {
    pub path: String,
    pub sources: Vec<String>,
    /// False also means unverified/unavailable: unsafe/network/reparse paths are
    /// never followed, and an access error is not evidence that a path is absent.
    pub exists: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    Process,
    User,
    System,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::User => "user",
            Self::System => "system",
        }
    }
}

type EnvLookup<'a> = dyn Fn(Scope, &str) -> Option<OsString> + 'a;
type RawEnvLookup<'a> = dyn Fn(Scope, &str) -> Option<EnvironmentValue> + 'a;

struct EnvironmentValue {
    value: OsString,
    expand: bool,
}

struct NativeEnvironment {
    #[cfg(windows)]
    user: Option<winreg::RegKey>,
    #[cfg(windows)]
    system: Option<winreg::RegKey>,
}

impl NativeEnvironment {
    fn new() -> Self {
        Self {
            #[cfg(windows)]
            user: winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                .open_subkey("Environment")
                .ok(),
            #[cfg(windows)]
            system: winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
                .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
                .ok(),
        }
    }

    // Read named variables only, never enumerate an environment or the registry.
    // A REG_EXPAND_SZ dependency is looked up individually while expanding a
    // whitelisted tool variable; no values are written or logged.
    fn get(&self, scope: Scope, name: &str) -> Option<OsString> {
        resolve_env_value(
            scope,
            name,
            &|scope, name| self.raw_value(scope, name),
            &mut Vec::new(),
        )
    }

    fn raw_value(&self, scope: Scope, name: &str) -> Option<EnvironmentValue> {
        match scope {
            Scope::Process => std::env::var_os(name).map(|value| EnvironmentValue {
                value,
                expand: false,
            }),
            #[cfg(windows)]
            Scope::User | Scope::System => {
                use winreg::enums::{REG_EXPAND_SZ, REG_SZ};
                use winreg::types::FromRegValue;

                let key = if scope == Scope::User {
                    &self.user
                } else {
                    &self.system
                };
                let raw = key.as_ref()?.get_raw_value(name).ok()?;
                if !matches!(raw.vtype, REG_SZ | REG_EXPAND_SZ) {
                    return None;
                }
                Some(EnvironmentValue {
                    value: OsString::from(String::from_reg_value(&raw).ok()?),
                    expand: raw.vtype == REG_EXPAND_SZ,
                })
            }
            #[cfg(not(windows))]
            Scope::User | Scope::System => None,
        }
    }
}

/// Prefer the process setting, but also find a Windows user or system setting
/// when the launching shell has not refreshed its environment yet.
pub(crate) fn preferred_codex_home() -> Option<PathBuf> {
    let environment = NativeEnvironment::new();
    preferred_codex_home_with(&|scope, name| environment.get(scope, name))
}

fn preferred_codex_home_with(lookup: &EnvLookup<'_>) -> Option<PathBuf> {
    [Scope::Process, Scope::User, Scope::System]
        .into_iter()
        .find_map(|scope| {
            lookup(scope, "CODEX_HOME")
                .filter(|value| !value.to_string_lossy().trim().is_empty())
                .map(PathBuf::from)
        })
}

/// Existing effective roots, without inventing new environment fallbacks.
fn current_dir(app: &AppType) -> Result<PathBuf, AppError> {
    Ok(match app {
        AppType::Claude => crate::config::get_claude_config_dir(),
        AppType::ClaudeDesktop => desktop_dir()?,
        AppType::Codex => crate::codex_config::get_codex_config_dir(),
        AppType::Gemini => crate::gemini_config::get_gemini_dir(),
        AppType::GrokBuild => crate::grok_config::get_grok_config_dir(),
        AppType::OpenCode => crate::opencode_config::get_opencode_dir(),
        AppType::OpenClaw => crate::openclaw_config::get_openclaw_dir(),
        AppType::Hermes => crate::hermes_config::get_hermes_dir(),
        AppType::Pi => crate::pi_config::get_pi_agent_dir()?,
        AppType::Omp => crate::omp_config::get_omp_agent_dir()?,
        AppType::Mcode => crate::mcode_config::data_dir(),
        AppType::Dsh => crate::dsh_config::data_dir(),
        AppType::Kimi => crate::kimi_config::data_dir(),
        AppType::Agy => crate::agy_config::agy_dir(),
        AppType::ZCode => crate::zcode_config::get_zcode_dir(),
        AppType::Copilot => crate::copilot_config::copilot_dir(),
        AppType::WorkBuddy => crate::workbuddy_config::data_dir(),
    })
}

fn desktop_dir() -> Result<PathBuf, AppError> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::config::get_home_dir().join("AppData").join("Local"));
        // Mirror the existing Desktop name/ordering rules without its is_dir()
        // traversal of every LOCALAPPDATA entry. Filter the known name first.
        let exact = local.join("Claude-3p");
        if local_directory_exists(&local) != Some(true) {
            return Ok(exact.join("configLibrary"));
        }
        match local_directory_exists(&exact) {
            Some(false) => {}
            _ => return Ok(exact.join("configLibrary")),
        }
        let mut candidates = std::fs::read_dir(&local)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("Claude") && name.contains("-3p"))
            })
            .map(|entry| entry.path())
            .filter(|path| local_directory_exists(path) == Some(true))
            .collect::<Vec<_>>();
        candidates.sort();
        Ok(candidates
            .into_iter()
            .next()
            .unwrap_or(exact)
            .join("configLibrary"))
    }
    #[cfg(not(windows))]
    crate::claude_desktop_config::get_config_library_path()
}

/// The existing resolver with only the saved CC Switch override removed. Apps
/// already honoring environment variables retain those native default semantics.
pub fn default_dir(app: &AppType) -> Result<PathBuf, AppError> {
    let home = crate::config::get_home_dir();
    Ok(match app {
        AppType::Claude => home.join(".claude"),
        AppType::Codex => home.join(".codex"),
        AppType::Gemini => home.join(".gemini"),
        AppType::GrokBuild => home.join(".grok"),
        AppType::OpenCode => home.join(".config").join("opencode"),
        AppType::OpenClaw => home.join(".openclaw"),
        AppType::ZCode => home.join(".zcode"),
        AppType::Hermes => std::env::var_os("HERMES_HOME")
            .map(|value| value.to_string_lossy().trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(crate::hermes_config::default_hermes_dir),
        AppType::Pi => crate::pi_config::resolve_pi_agent_dir(
            None,
            std::env::var_os("PI_CODING_AGENT_DIR"),
            home.join(".pi").join("agent"),
        )?,
        AppType::Omp => crate::omp_config::get_omp_native_agent_dir()?,
        AppType::Dsh => crate::dsh_config::default_data_dir(),
        // These apps have no saved directory override today.
        AppType::ClaudeDesktop
        | AppType::Mcode
        | AppType::Kimi
        | AppType::Agy
        | AppType::Copilot
        | AppType::WorkBuddy => current_dir(app)?,
    })
}

pub fn detect(app: &AppType) -> Result<ConfigDirectoryDetection, AppError> {
    let environment = NativeEnvironment::new();
    let lookup = |scope, name: &str| environment.get(scope, name);
    let scopes = if cfg!(windows) {
        vec![Scope::Process, Scope::User, Scope::System]
    } else {
        vec![Scope::Process]
    };
    Ok(detect_with(
        app,
        current_dir(app)?,
        default_dir(app)?,
        &crate::config::get_home_dir(),
        &scopes,
        &lookup,
        &local_directory_exists,
    ))
}

fn detect_with(
    app: &AppType,
    current: PathBuf,
    default: PathBuf,
    home: &Path,
    scopes: &[Scope],
    lookup: &EnvLookup<'_>,
    probe: &dyn Fn(&Path) -> Option<bool>,
) -> ConfigDirectoryDetection {
    let mut result = ConfigDirectoryDetection {
        current_dir: current.to_string_lossy().into_owned(),
        default_dir: default.to_string_lossy().into_owned(),
        candidates: Vec::new(),
    };
    add_candidate(
        &mut result.candidates,
        current,
        "current".into(),
        probe,
        true,
    );
    add_candidate(
        &mut result.candidates,
        default,
        "default".into(),
        probe,
        true,
    );
    for &scope in scopes {
        for (path, variable) in environment_candidates(app, home, scope, lookup) {
            add_candidate(
                &mut result.candidates,
                path,
                format!("{}:{variable}", scope.label()),
                probe,
                false,
            );
        }
    }
    result
}

fn environment_candidates(
    app: &AppType,
    home: &Path,
    scope: Scope,
    lookup: &EnvLookup<'_>,
) -> Vec<(PathBuf, &'static str)> {
    let mut candidates = Vec::new();
    let mut push = |variable, suffix: &[&str]| {
        if let Some(mut path) = env_path(scope, variable, lookup) {
            for part in suffix {
                path.push(part);
            }
            candidates.push((path, variable));
        }
    };
    match app {
        AppType::Claude => push("CLAUDE_CONFIG_DIR", &[]),
        AppType::Codex => push("CODEX_HOME", &[]),
        // Gemini's variable replaces homedir(), not getGlobalGeminiDir().
        AppType::Gemini => push("GEMINI_CLI_HOME", &[".gemini"]),
        AppType::GrokBuild => push("GROK_HOME", &[]),
        AppType::OpenCode => push("XDG_CONFIG_HOME", &["opencode"]),
        AppType::Hermes => push("HERMES_HOME", &[]),
        AppType::Pi => push("PI_CODING_AGENT_DIR", &[]),
        AppType::Dsh => push("DSH_HOME", &[]),
        AppType::Copilot => push("COPILOT_HOME", &[]),
        AppType::WorkBuddy => push("WORKBUDDY_CONFIG_DIR", &[]),
        AppType::Kimi => push("KIMI_CODE_HOME", &[]),
        AppType::Mcode => {
            // Preserve the tool's nonblank MINIMAX > MAVIS priority per scope.
            if env_text(scope, "MINIMAX_DATA_DIR", lookup).is_some() {
                push("MINIMAX_DATA_DIR", &[]);
            } else {
                push("MAVIS_DATA_DIR", &[]);
            }
        }
        AppType::OpenClaw => {
            // A separate config file cannot be represented by our root override.
            if env_text(scope, "OPENCLAW_CONFIG_PATH", lookup).is_some() {
                return candidates;
            }
            if env_text(scope, "OPENCLAW_STATE_DIR", lookup).is_some() {
                push("OPENCLAW_STATE_DIR", &[]);
            } else {
                let profile = env_text(scope, "OPENCLAW_PROFILE", lookup);
                if profile.as_deref().is_some_and(|value| !safe_profile(value)) {
                    return candidates;
                }
                let directory = profile
                    .filter(|value| value != "default")
                    .map(|value| format!(".openclaw-{value}"))
                    .unwrap_or_else(|| ".openclaw".into());
                if let Some(base) = env_path(scope, "OPENCLAW_HOME", lookup) {
                    let path = base.join(directory);
                    candidates.push((path.clone(), "OPENCLAW_HOME"));
                    if env_text(scope, "OPENCLAW_PROFILE", lookup).is_some() {
                        candidates.push((path, "OPENCLAW_PROFILE"));
                    }
                } else if env_text(scope, "OPENCLAW_PROFILE", lookup).is_some() {
                    candidates.push((home.join(directory), "OPENCLAW_PROFILE"));
                }
            }
        }
        AppType::Omp => {
            // Reuse the existing profile validator, including reserved device
            // names; an explicitly empty OMP_PROFILE overrides PI_PROFILE.
            let raw_profile = lookup(scope, "OMP_PROFILE").or_else(|| lookup(scope, "PI_PROFILE"));
            let raw_profile = raw_profile.as_deref().map(|value| value.to_string_lossy());
            let Ok(profile) = crate::omp_config::normalize_profile(raw_profile.as_deref()) else {
                return candidates;
            };
            let config_name = env_text(scope, "PI_CONFIG_DIR", lookup);
            let mut root = home.to_path_buf();
            if let Some(name) = config_name.as_deref() {
                // This variable is intentionally a name relative to homedir.
                if !safe_relative_name(name) {
                    return candidates;
                }
                root.push(name);
            } else {
                root.push(".omp");
            }
            if let Some(profile) = profile {
                let path = root.join("profiles").join(profile).join("agent");
                let profile_source = if lookup(scope, "OMP_PROFILE").is_some() {
                    "OMP_PROFILE"
                } else {
                    "PI_PROFILE"
                };
                candidates.push((path.clone(), profile_source));
                if config_name.is_some() {
                    candidates.push((path, "PI_CONFIG_DIR"));
                }
            } else if let Some(agent) = env_path(scope, "PI_CODING_AGENT_DIR", lookup) {
                // Ignore an inherited named-profile export in explicit default
                // mode, matching get_omp_native_agent_dir().
                let inherited = env_text(scope, "PI_PROFILE", lookup)
                    .and_then(|value| crate::omp_config::normalize_profile(Some(&value)).ok())
                    .flatten();
                if inherited
                    .is_none_or(|name| agent != root.join("profiles").join(name).join("agent"))
                {
                    candidates.push((agent, "PI_CODING_AGENT_DIR"));
                } else if config_name.is_some() {
                    candidates.push((root.join("agent"), "PI_CONFIG_DIR"));
                }
            } else if config_name.is_some() {
                candidates.push((root.join("agent"), "PI_CONFIG_DIR"));
            }
        }
        // No documented native configuration-root variable for these apps.
        AppType::ClaudeDesktop | AppType::Agy | AppType::ZCode => {}
    }
    candidates
}

fn safe_profile(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}

fn safe_relative_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn env_text(scope: Scope, variable: &str, lookup: &EnvLookup<'_>) -> Option<String> {
    let mut value = lookup(scope, variable)?.into_string().ok()?;
    if value.trim().is_empty() {
        return None;
    }
    if matches!(
        variable,
        "HERMES_HOME" | "MINIMAX_DATA_DIR" | "MAVIS_DATA_DIR"
    ) {
        value = value.trim().to_string();
    }
    Some(value)
}

fn env_path(scope: Scope, variable: &str, lookup: &EnvLookup<'_>) -> Option<PathBuf> {
    let value = env_text(scope, variable, lookup)?;
    let path = PathBuf::from(value);
    safe_local_path_form(&path).then_some(path)
}

fn resolve_env_value(
    scope: Scope,
    name: &str,
    lookup: &RawEnvLookup<'_>,
    stack: &mut Vec<String>,
) -> Option<OsString> {
    let value = lookup(scope, name)?;
    // Process and REG_SZ values are literal. In particular, a literal percent in
    // an app's root must not silently select a different directory.
    if !value.expand {
        return Some(value.value);
    }
    if stack.len() >= 16 || stack.iter().any(|item| item.eq_ignore_ascii_case(name)) {
        return None;
    }
    let raw = value.value.into_string().ok()?;
    stack.push(name.to_string());
    let result = expand_percent(&raw, scope, lookup, stack).map(OsString::from);
    stack.pop();
    result
}

fn expand_percent(
    raw: &str,
    scope: Scope,
    lookup: &RawEnvLookup<'_>,
    stack: &mut Vec<String>,
) -> Option<String> {
    if raw.len() > 32768 {
        return None;
    }
    let mut expanded = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        if name.is_empty() {
            return None;
        }
        let source = [scope, Scope::Process, Scope::User, Scope::System]
            .into_iter()
            .find(|&source| lookup(source, name).is_some())?;
        let value = resolve_env_value(source, name, lookup, stack)?
            .into_string()
            .ok()?;
        expanded.push_str(&value);
        if expanded.len() > 32768 {
            return None;
        }
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);
    (expanded.len() <= 32768).then_some(expanded)
}

fn candidate_key(path: &Path) -> String {
    let key = path.to_string_lossy();
    if cfg!(windows) {
        key.replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    } else {
        key.trim_end_matches('/').to_string()
    }
}

fn add_candidate(
    candidates: &mut Vec<ConfigDirectoryCandidate>,
    path: PathBuf,
    source: String,
    probe: &dyn Fn(&Path) -> Option<bool>,
    retain_unverified: bool,
) {
    let key = candidate_key(&path);
    if let Some(existing) = candidates
        .iter_mut()
        .find(|item| candidate_key(Path::new(&item.path)) == key)
    {
        if !existing.sources.contains(&source) {
            existing.sources.push(source);
        }
        return;
    }
    let state = if safe_local_path_form(&path) {
        probe(&path)
    } else {
        None
    };
    if state.is_none() && !retain_unverified {
        return;
    }
    candidates.push(ConfigDirectoryCandidate {
        path: path.to_string_lossy().into_owned(),
        sources: vec![source],
        exists: state.unwrap_or(false),
    });
}

fn safe_local_path_form(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    if !path.is_absolute()
        || raw.contains('\0')
        || raw.starts_with("\\\\")
        || raw.starts_with("//")
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return false;
    }
    #[cfg(windows)]
    {
        if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_)))
        {
            return false;
        }
        if path
            .components()
            .any(|part| matches!(part, Component::Normal(name) if !safe_windows_component(name)))
        {
            return false;
        }
    }
    true
}

#[cfg(any(windows, test))]
fn safe_windows_component(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if name.contains(':') {
        return false; // Alternate data streams are not configuration directories.
    }
    let stem = name
        .trim_end_matches([' ', '.'])
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    if let Some(port) = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
    {
        if matches!(
            port,
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        ) {
            return false;
        }
    }
    true
}

/// None is intentionally conservative: no network, device, mapped network drive,
/// symlink or Windows reparse-point traversal. Read directory metadata only.
fn local_directory_exists(path: &Path) -> Option<bool> {
    if !safe_local_path_form(path) {
        return None;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        let root = path.ancestors().last()?;
        let root: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        // DRIVE_REMOVABLE / DRIVE_FIXED / DRIVE_CDROM / DRIVE_RAMDISK only.
        if !matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 5 | 6) {
            return None;
        }
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(false),
            Err(_) => return None,
        };
        if metadata.file_type().is_symlink() {
            return None;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return None;
            }
        }
        if !metadata.is_dir() {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn root(name: &str) -> PathBuf {
        if cfg!(windows) {
            Path::new("D:\\detector-test").join(name)
        } else {
            Path::new("/detector-test").join(name)
        }
    }

    #[test]
    fn codex_home_uses_process_then_user_then_system_environment() {
        let process = root("process");
        let user = root("user");
        let system = root("system");
        let lookup = |scope, name: &str| {
            assert_eq!(name, "CODEX_HOME");
            Some(match scope {
                Scope::Process => process.as_os_str().to_owned(),
                Scope::User => user.as_os_str().to_owned(),
                Scope::System => system.as_os_str().to_owned(),
            })
        };
        assert_eq!(preferred_codex_home_with(&lookup), Some(process));

        let lookup = |scope, _: &str| match scope {
            Scope::Process => Some("  ".into()),
            Scope::User => Some(user.as_os_str().to_owned()),
            Scope::System => Some(system.as_os_str().to_owned()),
        };
        assert_eq!(preferred_codex_home_with(&lookup), Some(user));

        let lookup =
            |scope, _: &str| (scope == Scope::System).then(|| system.as_os_str().to_owned());
        assert_eq!(preferred_codex_home_with(&lookup), Some(system));
        assert_eq!(preferred_codex_home_with(&|_, _| None), None);
    }

    #[test]
    fn candidates_are_visible_without_changing_a_saved_root() {
        let saved = root("saved");
        let native = root("native");
        let lookup = |scope, name: &str| {
            (scope == Scope::Process && name == "CODEX_HOME").then(|| native.as_os_str().to_owned())
        };
        let result = detect_with(
            &AppType::Codex,
            saved.clone(),
            root("home/.codex"),
            &root("home"),
            &[Scope::Process],
            &lookup,
            &|_| Some(true),
        );
        assert_eq!(result.current_dir, saved.to_string_lossy());
        assert_eq!(result.candidates.len(), 3);
        assert_eq!(result.candidates[2].path, native.to_string_lossy());
        assert_eq!(result.candidates[2].sources, ["process:CODEX_HOME"]);
    }

    #[test]
    fn workbuddy_environment_candidate_uses_the_native_directory() {
        let native = root("workbuddy");
        let lookup = |scope, name: &str| {
            (scope == Scope::Process && name == "WORKBUDDY_CONFIG_DIR")
                .then(|| native.as_os_str().to_owned())
        };
        let candidates =
            environment_candidates(&AppType::WorkBuddy, &root("home"), Scope::Process, &lookup);
        assert_eq!(candidates, vec![(native, "WORKBUDDY_CONFIG_DIR")]);
    }

    #[test]
    fn parent_variables_transform_before_deduplication() {
        for (app, variable, suffix) in [
            (AppType::Gemini, "GEMINI_CLI_HOME", ".gemini"),
            (AppType::OpenCode, "XDG_CONFIG_HOME", "opencode"),
        ] {
            let base = root("parent");
            let target = base.join(suffix);
            let lookup = |_, name: &str| (name == variable).then(|| base.as_os_str().to_owned());
            let result = detect_with(
                &app,
                target.clone(),
                target.clone(),
                &root("home"),
                &[Scope::Process, Scope::User],
                &lookup,
                &|_| Some(true),
            );
            assert_eq!(result.candidates.len(), 1);
            assert_eq!(result.candidates[0].path, target.to_string_lossy());
            assert_eq!(
                result.candidates[0].sources,
                [
                    "current",
                    "default",
                    &format!("process:{variable}"),
                    &format!("user:{variable}")
                ]
            );
        }
    }

    #[test]
    fn invalid_candidates_and_unsafe_current_do_not_get_probed() {
        let calls = Cell::new(0);
        let current = PathBuf::from("\\\\server\\share\\saved");
        for value in [
            "",
            "   ",
            "relative",
            "\\\\server\\share",
            "\\\\?\\C:\\device",
        ] {
            let lookup = |_, name: &str| (name == "CODEX_HOME").then(|| OsString::from(value));
            let result = detect_with(
                &AppType::Codex,
                current.clone(),
                root("default"),
                &root("home"),
                &[Scope::Process],
                &lookup,
                &|_| {
                    calls.set(calls.get() + 1);
                    Some(false)
                },
            );
            assert_eq!(result.current_dir, current.to_string_lossy());
            assert_eq!(result.candidates.len(), 2);
            assert!(!result.candidates[0].exists);
        }
        assert_eq!(calls.get(), 5); // Only the safe default on each call.
    }

    #[test]
    fn windows_device_names_and_alternate_streams_are_rejected_lexically() {
        for name in [
            "NUL",
            "con.txt",
            "AUX ",
            "COM1.log",
            "LPT9",
            "CONOUT$",
            "COM¹",
            "file:stream",
        ] {
            assert!(
                !safe_windows_component(std::ffi::OsStr::new(name)),
                "{name}"
            );
        }
        assert!(safe_windows_component(std::ffi::OsStr::new("config.json")));
        #[cfg(windows)]
        for path in ["C:\\NUL", "D:\\config\\CON.txt", "D:\\data:stream"] {
            let result = detect_with(
                &AppType::Codex,
                PathBuf::from(path),
                PathBuf::from(path),
                &root("home"),
                &[Scope::Process],
                &|_, _| Some(OsString::from(path)),
                &|_| panic!("device path must not be probed"),
            );
            assert_eq!(result.candidates.len(), 1);
            assert!(!result.candidates[0].exists);
        }
    }

    #[test]
    fn separate_config_files_are_not_misrepresented_as_root_overrides() {
        let lookup = |_, name: &str| {
            matches!(
                name,
                "OPENCODE_CONFIG"
                    | "OPENCODE_CONFIG_DIR"
                    | "OPENCLAW_CONFIG_PATH"
                    | "OPENCLAW_STATE_DIR"
            )
            .then(|| root(name).into_os_string())
        };
        for app in [AppType::OpenCode, AppType::OpenClaw] {
            assert!(
                environment_candidates(&app, &root("home"), Scope::Process, &lookup).is_empty()
            );
        }
    }

    #[test]
    fn registry_expansion_handles_nested_values_and_rejects_cycles_and_unknowns() {
        let lookup = |scope, name: &str| {
            let (value, expand) = match (scope, name) {
                (Scope::User, "BASE") => (root("user").into_os_string(), false),
                (Scope::Process, "BASE") => (root("old-process").into_os_string(), false),
                (Scope::Process, "CODEX_HOME") | (Scope::User, "LITERAL") => {
                    (root("literal%BASE%").into_os_string(), false)
                }
                (Scope::User, "CODEX_HOME") => (OsString::from("%NESTED%/codex"), true),
                (_, "NESTED") => (OsString::from("%BASE%/agents"), true),
                (_, "LOOP") => (OsString::from("%LOOP%"), true),
                (_, "UNKNOWN") => (OsString::from("%MISSING%"), true),
                _ => return None,
            };
            Some(EnvironmentValue { value, expand })
        };
        assert_eq!(
            resolve_env_value(Scope::User, "CODEX_HOME", &lookup, &mut vec![]),
            Some(OsString::from(format!(
                "{}/agents/codex",
                root("user").display()
            )))
        );
        assert_eq!(
            resolve_env_value(Scope::Process, "CODEX_HOME", &lookup, &mut vec![]),
            Some(root("literal%BASE%").into_os_string())
        );
        assert_eq!(
            resolve_env_value(Scope::User, "LITERAL", &lookup, &mut vec![]),
            Some(root("literal%BASE%").into_os_string())
        );
        assert!(resolve_env_value(Scope::User, "LOOP", &lookup, &mut vec![]).is_none());
        assert!(resolve_env_value(Scope::User, "UNKNOWN", &lookup, &mut vec![]).is_none());
    }

    #[test]
    fn omp_directory_name_and_profile_are_transformed_using_native_rules() {
        let lookup = |_, name: &str| match name {
            "PI_CONFIG_DIR" => Some(OsString::from(".custom-omp")),
            "OMP_PROFILE" => Some(OsString::from("work")),
            "PI_CODING_AGENT_DIR" => Some(root("ignored-agent").into_os_string()),
            _ => None,
        };
        let candidates =
            environment_candidates(&AppType::Omp, &root("home"), Scope::Process, &lookup);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0].0,
            root("home/.custom-omp/profiles/work/agent")
        );
        assert_eq!(candidates[0].1, "OMP_PROFILE");
        assert_eq!(candidates[1].1, "PI_CONFIG_DIR");
    }

    #[test]
    fn omp_legacy_profile_keeps_its_actual_source_label() {
        let lookup = |_, name: &str| (name == "PI_PROFILE").then(|| OsString::from("work"));
        let candidates = environment_candidates(&AppType::Omp, &root("home"), Scope::User, &lookup);
        assert_eq!(
            candidates,
            vec![(root("home/.omp/profiles/work/agent"), "PI_PROFILE")]
        );
    }

    #[test]
    fn metadata_probe_reads_only_owned_synthetic_directories() {
        let temporary =
            tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("synthetic directory");
        let child = temporary.path().join("child");
        std::fs::create_dir(&child).expect("synthetic child");
        assert_eq!(local_directory_exists(&child), Some(true));
        assert_eq!(
            local_directory_exists(&temporary.path().join("absent")),
            Some(false)
        );
        let absent = temporary.path().join("not-created");
        let lookup = |_, name: &str| (name == "CODEX_HOME").then(|| absent.as_os_str().to_owned());
        let result = detect_with(
            &AppType::Codex,
            child.clone(),
            child.clone(),
            temporary.path(),
            &[Scope::Process],
            &lookup,
            &local_directory_exists,
        );
        assert_eq!(result.candidates.len(), 2);
        assert!(!result.candidates[1].exists);
        assert!(
            !absent.exists(),
            "detection must not create a candidate directory"
        );
        #[cfg(unix)]
        {
            let link = temporary.path().join("link");
            std::os::unix::fs::symlink(&child, &link).expect("synthetic symlink");
            assert_eq!(local_directory_exists(&link), None);
        }
        #[cfg(windows)]
        {
            let link = temporary.path().join("junction");
            crate::agy_config::create_dir_link(&child, &link).expect("synthetic directory link");
            assert_eq!(local_directory_exists(&link), None);
            assert_eq!(local_directory_exists(&link.join("not-created")), None);
            assert!(!child.join("not-created").exists());
        }
    }
}
