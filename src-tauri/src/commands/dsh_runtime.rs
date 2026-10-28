use super::{build_tool_search_paths, locate_default_tool, resolve_path_default, CommandDeadline};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct NativeRuntime {
    pub anchor: PathBuf,
    node: PathBuf,
    electron: bool,
}

impl NativeRuntime {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.node);
        command.env_remove("ELECTRON_RUN_AS_NODE");
        if self.electron {
            // This is the installed DSH launcher's Node/ASAR mode, not a desktop launch.
            command
                .env("ELECTRON_RUN_AS_NODE", "1")
                .arg("--expose-internals");
        }
        command
    }
}

#[cfg(windows)]
fn desktop_at(root: &Path) -> Option<NativeRuntime> {
    let node = root.join("DeepSeek Harness.exe");
    let archive = root.join("resources/app.asar");
    (root.is_absolute() && node.is_file() && archive.is_file()).then(|| NativeRuntime {
        node,
        anchor: archive.join("dsh/node_modules/@deepseek-ai/dsh/package.json"),
        electron: true,
    })
}

#[cfg(windows)]
fn desktop_for_cli(cli: &Path) -> Option<NativeRuntime> {
    let real = std::fs::canonicalize(cli).ok()?;
    let root = real.ancestors().nth(5)?;
    let expected = root.join("resources/runtime/cli/bin/dsh.cmd");
    if std::fs::canonicalize(expected).ok()? != real {
        return None;
    }
    desktop_at(root)
}

#[cfg(windows)]
fn desktop_runtimes() -> Vec<NativeRuntime> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut roots = std::collections::BTreeSet::new();
    for (hive, key) in [
        (
            HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ] {
        let Ok(uninstall) = RegKey::predef(hive).open_subkey(key) else {
            continue;
        };
        for name in uninstall.enum_keys().flatten() {
            let Ok(entry) = uninstall.open_subkey(name) else {
                continue;
            };
            let Ok(name) = entry.get_value::<String, _>("DisplayName") else {
                continue;
            };
            if name != "DeepSeek Harness" && !name.starts_with("DeepSeek Harness ") {
                continue;
            }
            if let Ok(location) = entry.get_value::<String, _>("InstallLocation") {
                let path = PathBuf::from(location.trim().trim_matches('"'));
                if let Ok(real) = std::fs::canonicalize(path) {
                    roots.insert(real);
                }
            }
            if let Ok(icon) = entry.get_value::<String, _>("DisplayIcon") {
                let file = icon
                    .rsplit_once(',')
                    .filter(|(_, index)| index.trim().parse::<i32>().is_ok())
                    .map_or(icon.as_str(), |(file, _)| file)
                    .trim()
                    .trim_matches('"');
                if let Some(root) = Path::new(file)
                    .parent()
                    .and_then(|path| std::fs::canonicalize(path).ok())
                {
                    roots.insert(root);
                }
            }
        }
    }
    roots.iter().filter_map(|root| desktop_at(root)).collect()
}

#[cfg(not(windows))]
fn desktop_runtimes() -> Vec<NativeRuntime> {
    Vec::new()
}

pub(super) fn desktop_cli_dirs() -> Vec<PathBuf> {
    desktop_runtimes()
        .iter()
        .filter_map(|runtime| runtime.node.parent())
        .map(|root| root.join("resources/runtime/cli/bin"))
        .collect()
}

pub(super) fn is_desktop_cli(path: &Path) -> bool {
    #[cfg(windows)]
    {
        desktop_for_cli(path).is_some()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn package_anchor(directory: &Path) -> Option<PathBuf> {
    let manifest = directory.join("@deepseek-ai/dsh/package.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).ok()?).ok()?;
    (value["name"] == "@deepseek-ai/dsh")
        .then(|| std::fs::canonicalize(manifest).ok())
        .flatten()
}

fn standalone(anchor: PathBuf, deadline: Option<CommandDeadline>) -> Result<NativeRuntime, String> {
    Ok(NativeRuntime {
        anchor,
        node: locate_default_tool("node", deadline).map_err(|error| {
            if error == "node is not installed" {
                "DSH_NODE_NOT_FOUND".into()
            } else {
                error
            }
        })?,
        electron: false,
    })
}

pub(crate) fn resolve(deadline: Option<CommandDeadline>) -> Result<NativeRuntime, String> {
    if let Some(cli) = resolve_path_default("dsh", deadline)? {
        #[cfg(windows)]
        if let Some(runtime) = desktop_for_cli(&cli) {
            return Ok(runtime);
        }
        return standalone(cli, deadline);
    }
    // The Desktop distribution owns the profile library even before its CLI is registered.
    let desktops = desktop_runtimes();
    if desktops.len() > 1 {
        return Err("DSH_NATIVE_RUNTIME_AMBIGUOUS".into());
    }
    if let Some(runtime) = desktops.into_iter().next() {
        return Ok(runtime);
    }
    match locate_default_tool("dsh", deadline) {
        Ok(cli) => return standalone(cli, deadline),
        Err(error) if error == "dsh is not installed" => {}
        Err(error) => return Err(error),
    }
    let mut packages = std::collections::BTreeSet::new();
    for directory in build_tool_search_paths("dsh") {
        let mut modules = vec![directory.join("node_modules")];
        if let Some(parent) = directory.parent() {
            modules.extend([parent.join("node_modules"), parent.join("lib/node_modules")]);
        }
        packages.extend(modules.iter().filter_map(|path| package_anchor(path)));
    }
    match packages.len() {
        0 => Err("DSH_NATIVE_CONFIG_LIBRARY_UNAVAILABLE".into()),
        1 => standalone(packages.into_iter().next().unwrap(), deadline),
        _ => Err("DSH_NATIVE_RUNTIME_AMBIGUOUS".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_without_a_registered_command_is_discoverable_but_unrelated_manifests_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("@deepseek-ai/dsh");
        std::fs::create_dir_all(&package).unwrap();
        let manifest = package.join("package.json");
        std::fs::write(&manifest, r#"{"name":"@deepseek-ai/dsh"}"#).unwrap();
        assert_eq!(
            package_anchor(dir.path()),
            Some(std::fs::canonicalize(&manifest).unwrap())
        );
        std::fs::write(&manifest, r#"{"name":"unrelated"}"#).unwrap();
        assert!(package_anchor(dir.path()).is_none());
    }

    #[test]
    #[cfg(windows)]
    fn desktop_without_path_registration_uses_its_official_node_mode() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            desktop_at(dir.path()).is_none(),
            "a residual profile directory is not an install"
        );
        std::fs::create_dir_all(dir.path().join("resources/runtime/cli/bin")).unwrap();
        std::fs::write(dir.path().join("DeepSeek Harness.exe"), "fixture").unwrap();
        std::fs::write(dir.path().join("resources/app.asar"), "fixture").unwrap();
        let cli = dir.path().join("resources/runtime/cli/bin/dsh.cmd");
        std::fs::write(&cli, "fixture").unwrap();
        let runtime = desktop_for_cli(&cli).unwrap();
        assert_eq!(
            runtime.node,
            std::fs::canonicalize(dir.path())
                .unwrap()
                .join("DeepSeek Harness.exe")
        );
        assert!(runtime
            .anchor
            .ends_with("resources/app.asar/dsh/node_modules/@deepseek-ai/dsh/package.json"));
        let command = runtime.command();
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--expose-internals"]
        );
        assert!(command
            .get_envs()
            .any(|(name, value)| name == "ELECTRON_RUN_AS_NODE"
                && value == Some(std::ffi::OsStr::new("1"))));
        let install = super::super::ToolInstallation {
            path: cli.to_string_lossy().into_owned(),
            real: std::fs::canonicalize(&cli).unwrap(),
            version: Some("0.2.1".into()),
            runnable: true,
            error: None,
            exited_with_error: false,
            source: "system".into(),
            is_path_default: false,
        };
        assert!(
            matches!(
                super::super::resolve_update_command("dsh", &[install]),
                super::super::UpdateCommand::Unmanaged
            ),
            "Desktop updates must not install a second npm CLI"
        );
    }
}
