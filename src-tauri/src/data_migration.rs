//! One-time migration of CC Switch's own legacy data before any application writer starts.

use crate::services::claude_account::shared::LINKED_DIRS;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const STORE_FILE: &str = "app_paths.json";
const MARKER_FILE: &str = ".cc-switch-data-migration-v1.json";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Targets {
    app: PathBuf,
    device: PathBuf,
    tauri: PathBuf,
    webview: PathBuf,
    cache: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    version: u8,
    targets: Targets,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_source: Option<SourceKind>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SourceKind {
    LocalD,
    Official,
    OfficialAux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationDecision {
    Migrate,
    StartFresh,
    Cancel,
}

fn ask_migration(source: &SourceRoots, target: &Path) -> io::Result<MigrationDecision> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDCANCEL, IDNO, IDYES, MB_DEFBUTTON3, MB_ICONQUESTION, MB_YESNOCANCEL,
    };

    let message = if source.auxiliary_only {
        let device = if source.device != source.app {
            format!("设备资料：{}\n", source.device.display())
        } else {
            String::new()
        };
        format!(
            "当前已使用指定目录中的 CC Switch 数据库。\n\n发现旧版辅助资料：\n{}配置目录：{}\n浏览器资料：{}\n\n是否复制这些辅助资料到当前数据目录？\n{}\n\n数据库和原目录不会被删除或覆盖。",
            device, source.tauri.display(), source.webview.display(), target.display()
        )
    } else {
        format!(
        "发现旧版 CC Switch 数据。\n应用目录：{}\n设备目录：{}\n配置目录：{}\n浏览器资料：{}\n\n是否复制到当前数据目录？\n{}\n\n原目录不会被删除。选择“否”将跳过此来源，并继续检查其他来源。",
        source.app.display(),
        source.device.display(),
        source.tauri.display(),
        source.webview.display(),
        target.display()
        )
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "迁移 CC Switch 数据"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNOCANCEL | MB_ICONQUESTION | MB_DEFBUTTON3,
        )
    };
    match result {
        IDYES => Ok(MigrationDecision::Migrate),
        IDNO => Ok(MigrationDecision::StartFresh),
        IDCANCEL => Ok(MigrationDecision::Cancel),
        _ => Err(io::Error::last_os_error()),
    }
}

struct Legacy {
    home: PathBuf,
    home_env: Option<PathBuf>,
    tauri: PathBuf,
    local_data: PathBuf,
    webview: PathBuf,
    local_d: Option<Targets>,
}

#[derive(Debug, PartialEq, Eq)]
enum SourceEntry {
    Directory,
    File {
        length: u64,
        modified: std::time::SystemTime,
        readonly: bool,
    },
    SharedLink(PathBuf),
}

type SourceSnapshot = Option<BTreeMap<PathBuf, SourceEntry>>;

/// The single-instance plugin must have acquired the application's lock first.
pub fn migrate() -> io::Result<()> {
    // Automated runs use isolated fixtures through migrate_from. A test-home
    // application must never inspect the machine's actual legacy directories.
    if !cfg!(windows) || crate::data_paths::test_root().is_some() {
        return Ok(());
    }
    let targets = Targets {
        app: crate::data_paths::app_dir(),
        device: crate::data_paths::device_dir(),
        tauri: crate::data_paths::tauri_dir(),
        webview: crate::data_paths::webview_dir(),
        cache: crate::data_paths::cache_dir(),
    };
    let local_data = dirs::data_local_dir()
        .ok_or_else(|| invalid("Cannot resolve the legacy local-data directory"))?;
    let legacy = Legacy {
        home: dirs::home_dir()
            .ok_or_else(|| invalid("Cannot resolve the legacy home directory"))?,
        home_env: std::env::var_os("HOME")
            .and_then(|value| value.into_string().ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        tauri: dirs::data_dir()
            .ok_or_else(|| invalid("Cannot resolve the legacy Tauri directory"))?
            .join("com.ccswitch.desktop"),
        webview: local_data.join("com.ccswitch.desktop"),
        local_data,
        local_d: Some(Targets {
            app: PathBuf::from(r"D:\Data\CCSwitch\app"),
            device: PathBuf::from(r"D:\Data\CCSwitch\device"),
            tauri: PathBuf::from(r"D:\Data\CCSwitch\tauri"),
            webview: PathBuf::from(r"D:\Data\CCSwitch\webview2"),
            cache: PathBuf::from(r"D:\Cache\CCSwitch"),
        }),
    };
    migrate_prompted(
        &targets,
        &legacy,
        crate::data_paths::validate_data_dir,
        is_d_path,
        crate::data_paths::has_explicit_data_dir(),
        true,
        ask_migration,
    )
}

#[cfg(test)]
fn migrate_from(
    targets: &Targets,
    legacy: &Legacy,
    validate_dir: fn(&Path) -> io::Result<()>,
    on_d: fn(&Path) -> bool,
) -> io::Result<()> {
    migrate_prompted(targets, legacy, validate_dir, on_d, false, false, |_, _| {
        Ok(MigrationDecision::Migrate)
    })
}

fn migrate_prompted(
    targets: &Targets,
    legacy: &Legacy,
    validate_dir: fn(&Path) -> io::Result<()>,
    _on_d: fn(&Path) -> bool,
    explicit_data_dir: bool,
    protect_existing_target: bool,
    mut ask: impl FnMut(&SourceRoots, &Path) -> io::Result<MigrationDecision>,
) -> io::Result<()> {
    for (directory, allow_shared_links) in [
        (&targets.app, true),
        (&targets.device, true),
        (&targets.tauri, false),
        (&targets.webview, false),
        (&targets.cache, false),
    ] {
        validate_dir(directory)?;
        validate_owned_tree(directory, allow_shared_links)?;
    }
    let marker = targets.tauri.join(MARKER_FILE);
    let mut pending_marker: Option<(Vec<u8>, SourceKind)> = None;
    if let Some(metadata) = metadata_if_present(&marker)? {
        require_plain_file(&marker, &metadata)?;
        let original = fs::read(&marker)?;
        let completion: Completion = serde_json::from_slice(&original)
            .map_err(|error| invalid(format!("Invalid migration marker: {error}")))?;
        if completion.version != 1 {
            return Err(invalid(
                "Migration marker targets do not match this application",
            ));
        }
        if completion.targets != *targets {
            rebase_portable_marker(&marker, &original, &completion, targets, legacy)?;
        }
        if completion.pending_source.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!(
                    "上次数据迁移未完成，旧目录和新目录均已保留。请检查后使用 --data-dir 指定新的空目录并重试。\n当前数据目录：{}",
                    targets.app.display()
                ),
            ));
        }
        validate_active_app(targets, legacy, validate_dir, explicit_data_dir)?;
        return Ok(());
    }

    // A chosen target with existing data is already in use. Never merge a
    // different legacy profile into it or ask to replace it.
    if pending_marker.is_none() && protect_existing_target && target_has_data(targets)? {
        let official_root = legacy.home.join(".cc-switch");
        if targets.app == targets.device && metadata_if_present(&official_root)?.is_some() {
            let direct_match = same_official_data(&official_root, &targets.app)?;
            // Explicit data roots must remain usable even when an unrelated old
            // Store cannot be parsed. Only a proven override links device data.
            let override_match = if explicit_data_dir {
                match read_legacy_store(&legacy.tauri) {
                    Ok((store, _)) => store_override_matches(&store, &legacy.home, &targets.app)?,
                    Err(error) => {
                        log::warn!("Cannot inspect optional legacy Store: {error}");
                        false
                    }
                }
            } else {
                false
            };
            if direct_match || override_match {
                let copy_device = override_match
                    && fs::canonicalize(&official_root)? != fs::canonicalize(&targets.app)?;
                return migrate_direct_auxiliary(targets, legacy, None, copy_device, &mut ask);
            }
        }
        return Ok(());
    }

    let candidates = legacy_candidates(legacy, targets)?;
    let mut selected = None;
    for (kind, source) in candidates {
        match ask(&source, &targets.app)? {
            MigrationDecision::Migrate => {
                selected = Some((kind, source));
                break;
            }
            MigrationDecision::StartFresh => continue,
            MigrationDecision::Cancel => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "已取消数据迁移"));
            }
        }
    }
    let (source_kind, shown_source) = match selected {
        Some(selected) => selected,
        None => {
            publish_bytes(&targets.tauri.join(STORE_FILE), b"{}")?;
            return publish_bytes(&marker, &completion_bytes(targets, None)?);
        }
    };

    let source = source_roots(source_kind, legacy)?;
    if source != shown_source {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "迁移来源在确认后发生变化，未开始复制，请重新选择。",
        ));
    }
    let (mut store, original_store) = read_legacy_store(&source.tauri)?;
    let override_path = match store.get("app_config_dir_override") {
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            let path = resolve_override(raw.trim(), &legacy.home);
            reject_broad_source(&path, legacy)?;
            Some(path)
        }
        Some(Value::String(_)) | None => None,
        Some(_) => return Err(invalid("Legacy app_config_dir_override must be a string")),
    };

    let source_app = match &override_path {
        Some(path) => path.clone(),
        None => source.app.clone(),
    };
    if source_app != shown_source.app {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "迁移来源在确认后发生变化，未开始复制，请重新选择。",
        ));
    }
    if override_path.is_some() {
        reject_target_overlap(&source_app, targets)?;
    }
    let app_target = targets.app.clone();
    if metadata_if_present(&source.webview)?.is_some() {
        reject_broad_source(&source.webview, legacy)?;
    }
    let store_bytes = if override_path.is_some() {
        // The selected root now owns the copied app data. Keeping the old
        // absolute override would silently switch back to the legacy source.
        store.remove("app_config_dir_override");
        serde_json::to_vec_pretty(&store).map_err(|error| invalid(error.to_string()))?
    } else {
        original_store.clone().unwrap_or_else(|| b"{}".to_vec())
    };

    // The legacy default mixed application and device files in one directory.
    // Copy it only once when the new layout also shares that directory.
    let mut trees: Vec<(&Path, &Path, Option<&str>, bool)> =
        vec![(&source_app, &app_target, None, true)];
    let same_source = metadata_if_present(&source_app)?.is_some()
        && metadata_if_present(&source.device)?.is_some()
        && fs::canonicalize(&source_app)? == fs::canonicalize(&source.device)?;
    let same_target = app_target == targets.device;
    let copy_known_device = !(same_target && same_source) && (same_target || same_source);
    if !(copy_known_device || same_target && same_source) {
        trees.push((&source.device, &targets.device, None, true));
    }
    trees.push((&source.tauri, &targets.tauri, Some(STORE_FILE), false));
    trees.push((&source.webview, &targets.webview, None, false));
    let mut preflight = trees.clone();
    if copy_known_device {
        preflight.push((&source.device, &targets.device, None, true));
    }
    preflight_disjoint_sources(&preflight, targets)?;
    if pending_marker.is_none() {
        let bytes = completion_bytes(targets, Some(source_kind))?;
        publish_bytes(&marker, &bytes)?;
        pending_marker = Some((bytes, source_kind));
    }
    let snapshots = trees
        .iter()
        .map(|(source, _, skip, allow)| snapshot_source(source, *skip, *allow))
        .collect::<io::Result<Vec<_>>>()?;
    let selected_device = if copy_known_device {
        Some(snapshot_known_device(&source.device)?)
    } else {
        None
    };
    for (source, target, skip, allow) in &trees {
        copy_tree(source, target, *skip, *allow)?;
    }
    if let Some(snapshot) = &selected_device {
        copy_known_device_entries(&source.device, &targets.device, snapshot)?;
    }
    publish_bytes(&targets.tauri.join(STORE_FILE), &store_bytes)?;
    for (directory, allow_shared_links) in [
        (&app_target, true),
        (&targets.device, true),
        (&targets.tauri, false),
        (&targets.webview, false),
        (&targets.cache, false),
    ] {
        validate_owned_tree(directory, allow_shared_links)?;
    }
    // Single-instance ownership makes the old CC writer quiescent. Verify
    // again nevertheless: an external edit, removed source, or partial read
    // must stop startup before publishing the completion marker.
    for ((source, target, skip, allow), snapshot) in trees.into_iter().zip(snapshots) {
        verify_source(source, target, skip, allow, &snapshot)?;
    }
    if let Some(snapshot) = &selected_device {
        verify_known_device(&source.device, &targets.device, snapshot)?;
    }
    if read_legacy_store(&source.tauri)?.1 != original_store {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Legacy store changed during migration",
        ));
    }

    replace_known_file(
        &marker,
        &pending_marker
            .ok_or_else(|| invalid("Missing migration state"))?
            .0,
        &completion_bytes(targets, None)?,
    )
}

fn same_official_data(old: &Path, selected: &Path) -> io::Result<bool> {
    if fs::canonicalize(old)? == fs::canonicalize(selected)? {
        return Ok(true);
    }
    // A copied direct-layout directory is associated only when the old
    // official DB and its WAL, if present, match byte for byte.
    for name in ["cc-switch.db", "cc-switch.db-wal"] {
        let left = old.join(name);
        let right = selected.join(name);
        match (metadata_if_present(&left)?, metadata_if_present(&right)?) {
            (None, None) if name.ends_with("-wal") => {}
            (Some(left_meta), Some(right_meta)) => {
                require_plain_file(&left, &left_meta)?;
                require_plain_file(&right, &right_meta)?;
                if let Err(error) = require_equal(&left, &right) {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        return Ok(false);
                    }
                    return Err(error);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn migrate_direct_auxiliary(
    targets: &Targets,
    legacy: &Legacy,
    pending: Option<Vec<u8>>,
    copy_device: bool,
    ask: &mut impl FnMut(&SourceRoots, &Path) -> io::Result<MigrationDecision>,
) -> io::Result<()> {
    let source = SourceRoots {
        app: targets.app.clone(),
        device: if copy_device {
            legacy.home.join(".cc-switch")
        } else {
            targets.device.clone()
        },
        tauri: legacy.tauri.clone(),
        webview: legacy.webview.clone(),
        auxiliary_only: true,
    };
    let trees: [(&Path, &Path, Option<&str>, bool); 2] = [
        (&source.tauri, &targets.tauri, Some(STORE_FILE), false),
        (&source.webview, &targets.webview, None, false),
    ];
    let device_snapshot = if copy_device {
        snapshot_known_device(&source.device)?
    } else {
        None
    };
    if pending.is_none() {
        let mut found = device_snapshot
            .as_ref()
            .is_some_and(|entries| !entries.is_empty());
        for (old, target, _, _) in trees {
            if directory_has_entries(old)? && fs::canonicalize(old)? != fs::canonicalize(target)? {
                found = true;
            }
        }
        if !found {
            return Ok(());
        }
        match ask(&source, &targets.app)? {
            MigrationDecision::StartFresh => {
                return publish_bytes(
                    &targets.tauri.join(MARKER_FILE),
                    &completion_bytes(targets, None)?,
                );
            }
            MigrationDecision::Cancel => {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "已取消数据迁移"));
            }
            MigrationDecision::Migrate => {}
        }
    }
    let mut preflight = trees.to_vec();
    if copy_device {
        preflight.push((&source.device, &targets.device, None, true));
    }
    preflight_disjoint_sources(&preflight, targets)?;
    if copy_device {
        if snapshot_known_device(&source.device)? != device_snapshot {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Device source changed after confirmation",
            ));
        }
        preflight_known_device(&source.device, &targets.device, &device_snapshot)?;
    }
    let (mut store, original_store) = read_legacy_store(&source.tauri)?;
    if copy_device && !store_override_matches(&store, &legacy.home, &targets.app)? {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Legacy Store no longer selects this data directory",
        ));
    }
    let store_bytes = if original_store.is_some() {
        store.remove("app_config_dir_override");
        Some(serde_json::to_vec_pretty(&store).map_err(|error| invalid(error.to_string()))?)
    } else {
        None
    };
    let marker = targets.tauri.join(MARKER_FILE);
    let original_marker = match pending {
        Some(bytes) => bytes,
        None => {
            let bytes = completion_bytes(targets, Some(SourceKind::OfficialAux))?;
            publish_bytes(&marker, &bytes)?;
            bytes
        }
    };
    let snapshots = trees
        .iter()
        .map(|(old, _, skip, allow)| snapshot_source(old, *skip, *allow))
        .collect::<io::Result<Vec<_>>>()?;
    for (old, target, skip, allow) in trees {
        copy_tree(old, target, skip, allow)?;
    }
    if copy_device {
        copy_known_device_entries(&source.device, &targets.device, &device_snapshot)?;
    }
    if let Some(bytes) = store_bytes {
        publish_bytes(&targets.tauri.join(STORE_FILE), &bytes)?;
    }
    if read_legacy_store(&source.tauri)?.1 != original_store {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Legacy store changed during migration",
        ));
    }
    for ((old, target, skip, allow), snapshot) in trees.into_iter().zip(snapshots) {
        verify_source(old, target, skip, allow, &snapshot)?;
    }
    if copy_device {
        verify_known_device(&source.device, &targets.device, &device_snapshot)?;
    }
    replace_known_file(&marker, &original_marker, &completion_bytes(targets, None)?)
}

fn store_override_matches(
    store: &Map<String, Value>,
    home: &Path,
    target: &Path,
) -> io::Result<bool> {
    let Some(Value::String(raw)) = store.get("app_config_dir_override") else {
        return Ok(false);
    };
    if raw.trim().is_empty() {
        return Ok(false);
    }
    let target = fs::canonicalize(target)?;
    match fs::canonicalize(resolve_override(raw.trim(), home)) {
        Ok(source) => Ok(source == target),
        Err(error) => {
            log::warn!("Cannot resolve optional legacy override: {error}");
            Ok(false)
        }
    }
}

type LegacyStore = (Map<String, Value>, Option<Vec<u8>>);

fn read_legacy_store(directory: &Path) -> io::Result<LegacyStore> {
    if metadata_if_present(directory)?.is_none() {
        return Ok((Map::new(), None));
    }
    require_source_directory(directory)?;
    let path = directory.join(STORE_FILE);
    let Some(metadata) = metadata_if_present(&path)? else {
        return Ok((Map::new(), None));
    };
    require_plain_file(&path, &metadata)?;
    let bytes = fs::read(&path)?;
    let store = serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| invalid(format!("Invalid legacy app_paths.json: {error}")))?;
    let Value::Object(store) = store else {
        return Err(invalid("Legacy app_paths.json must contain an object"));
    };
    Ok((store, Some(bytes)))
}

fn completion_bytes(targets: &Targets, pending_source: Option<SourceKind>) -> io::Result<Vec<u8>> {
    serde_json::to_vec_pretty(&Completion {
        version: 1,
        targets: targets.clone(),
        pending_source,
    })
    .map_err(|error| invalid(error.to_string()))
}

fn replace_known_file(path: &Path, expected: &[u8], replacement: &[u8]) -> io::Result<()> {
    require_plain_file(path, &fs::symlink_metadata(path)?)?;
    if fs::read(path)? != expected {
        return Err(conflict(path));
    }
    crate::config::atomic_write_private(path, replacement)
        .map_err(|error| io::Error::other(error.to_string()))
}

fn directory_has_entries(path: &Path) -> io::Result<bool> {
    let Some(_) = metadata_if_present(path)? else {
        return Ok(false);
    };
    require_source_directory(path)?;
    Ok(fs::read_dir(path)?.next().transpose()?.is_some())
}

fn target_has_data(targets: &Targets) -> io::Result<bool> {
    if targets.app == targets.device {
        for entry in fs::read_dir(&targets.app)? {
            let path = entry?.path();
            if path == targets.tauri || path == targets.webview || path == targets.cache {
                continue;
            }
            return Ok(true);
        }
        return Ok(
            directory_has_entries(&targets.tauri)? || directory_has_entries(&targets.webview)?
        );
    }
    for path in [
        &targets.app,
        &targets.device,
        &targets.tauri,
        &targets.webview,
    ] {
        if directory_has_entries(path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Debug, PartialEq, Eq)]
struct SourceRoots {
    app: PathBuf,
    device: PathBuf,
    tauri: PathBuf,
    webview: PathBuf,
    auxiliary_only: bool,
}

fn source_roots(kind: SourceKind, legacy: &Legacy) -> io::Result<SourceRoots> {
    let mut source = match kind {
        SourceKind::LocalD => {
            let local = legacy
                .local_d
                .as_ref()
                .ok_or_else(|| invalid("Missing old D source"))?;
            SourceRoots {
                app: local.app.clone(),
                device: local.device.clone(),
                tauri: local.tauri.clone(),
                webview: local.webview.clone(),
                auxiliary_only: false,
            }
        }
        SourceKind::Official => {
            let device = legacy.home.join(".cc-switch");
            SourceRoots {
                app: legacy_default_app(&device, legacy.home_env.as_deref())?,
                device,
                tauri: legacy.tauri.clone(),
                webview: legacy.webview.clone(),
                auxiliary_only: false,
            }
        }
        SourceKind::OfficialAux => {
            return Err(invalid("Auxiliary source requires direct-root migration"))
        }
    };
    let (store, _) = read_legacy_store(&source.tauri)?;
    match store.get("app_config_dir_override") {
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            let path = resolve_override(raw.trim(), &legacy.home);
            reject_broad_source(&path, legacy)?;
            source.app = path;
        }
        None | Some(Value::String(_)) => {}
        Some(_) => return Err(invalid("Legacy app_config_dir_override must be a string")),
    }
    Ok(source)
}

fn legacy_candidates(
    legacy: &Legacy,
    targets: &Targets,
) -> io::Result<Vec<(SourceKind, SourceRoots)>> {
    let mut candidates = Vec::new();
    for kind in [SourceKind::LocalD, SourceKind::Official] {
        if kind == SourceKind::LocalD {
            let Some(local) = &legacy.local_d else {
                continue;
            };
            let root = local
                .app
                .parent()
                .ok_or_else(|| invalid("Missing old D source root"))?;
            match fs::symlink_metadata(root) {
                Ok(metadata) => {
                    reject_link(root, &metadata)?;
                    require_source_directory(root)?;
                }
                Err(error) if optional_local_missing(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        let discover = || -> io::Result<Option<SourceRoots>> {
            let source = source_roots(kind, legacy)?;
            let found = {
                let mut found = false;
                for (old, target) in [
                    (&source.app, &targets.app),
                    (&source.device, &targets.device),
                    (&source.tauri, &targets.tauri),
                    (&source.webview, &targets.webview),
                ] {
                    if directory_has_entries(old)?
                        && fs::canonicalize(old)? != fs::canonicalize(target)?
                    {
                        found = true;
                        break;
                    }
                }
                found
            };
            Ok(found.then_some(source))
        };
        match discover() {
            Ok(Some(source)) => candidates.push((kind, source)),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(candidates)
}

fn optional_local_missing(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound || matches!(error.raw_os_error(), Some(15) | Some(21))
}

fn preflight_disjoint_sources(
    trees: &[(&Path, &Path, Option<&str>, bool)],
    targets: &Targets,
) -> io::Result<()> {
    for (source, selected_target, _, _) in trees {
        if metadata_if_present(source)?.is_none() {
            continue;
        }
        require_source_directory(source)?;
        let source_real = fs::canonicalize(source)?;
        for target in [
            &targets.app,
            &targets.device,
            &targets.tauri,
            &targets.webview,
            &targets.cache,
        ] {
            let target_real = fs::canonicalize(target)?;
            if source_real == target_real && *selected_target == target.as_path() {
                continue;
            }
            if source_real.starts_with(&target_real) || target_real.starts_with(&source_real) {
                return Err(invalid(
                    "Migration source overlaps the selected data directories",
                ));
            }
        }
    }
    Ok(())
}

// Only recognize the portable default layout: <exe>/data/{app,device,tauri,
// webview2} and <exe>/cache. A split or externally configured layout cannot
// be inferred safely from a moved marker.
fn portable_base(targets: &Targets) -> Option<&Path> {
    let direct = targets.app == targets.device;
    let data = if direct {
        targets.app.as_path()
    } else {
        targets.app.parent()?
    };
    let base = data.parent()?;
    if data.file_name()? != "data"
        || (!direct && targets.app != data.join("app"))
        || (!direct && targets.device != data.join("device"))
        || targets.tauri != data.join("tauri")
        || targets.webview != data.join("webview2")
        || targets.cache != base.join("cache")
    {
        return None;
    }
    Some(base)
}

fn rebase_portable_marker(
    marker: &Path,
    original: &[u8],
    completion: &Completion,
    targets: &Targets,
    legacy: &Legacy,
) -> io::Result<()> {
    if completion.pending_source.is_none()
        && completion.targets.app == targets.app
        && completion.targets.device == targets.device
        && completion.targets.tauri == targets.tauri
        && completion.targets.webview == targets.webview
    {
        return replace_known_file(marker, original, &completion_bytes(targets, None)?);
    }
    if completion.pending_source.is_some()
        || portable_base(&completion.targets).is_none()
        || portable_base(targets).is_none()
    {
        return Err(invalid(
            "Migration marker targets do not match this application",
        ));
    }
    // A completed, moved profile must still contain recognizable application
    // state; a copied marker alone is not evidence that data was moved.
    let has_state = [
        &targets.app.join("cc-switch.db"),
        &targets.app.join("config.json"),
        &targets.device.join("settings.json"),
    ]
    .into_iter()
    .try_fold(false, |found, path| {
        Ok::<_, io::Error>(found || metadata_if_present(path)?.is_some())
    })?;
    if !has_state {
        return Err(invalid("Moved CC Switch data is incomplete"));
    }
    for (source, target) in [
        (&completion.targets.app, &targets.app),
        (&completion.targets.device, &targets.device),
    ] {
        if let Some(snapshot) = snapshot_source(source, None, true)? {
            verify_source(source, target, None, true, &Some(snapshot))?;
        }
    }
    let (mut store, store_bytes) = read_legacy_store(&targets.tauri)?;
    let Some(store_bytes) = store_bytes else {
        if targets.app == targets.device {
            return replace_known_file(marker, original, &completion_bytes(targets, None)?);
        }
        return Err(invalid("Completed migration is missing app_paths.json"));
    };
    match store.get("app_config_dir_override") {
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            if resolve_override(raw.trim(), &legacy.home) != completion.targets.app {
                return Err(invalid(
                    "Moved profile has an external application directory override",
                ));
            }
            store.remove("app_config_dir_override");
            let updated =
                serde_json::to_vec_pretty(&store).map_err(|error| invalid(error.to_string()))?;
            replace_known_file(&targets.tauri.join(STORE_FILE), &store_bytes, &updated)?;
        }
        None | Some(Value::String(_)) => {}
        Some(_) => return Err(invalid("app_config_dir_override must be a string")),
    }
    replace_known_file(marker, original, &completion_bytes(targets, None)?)
}

fn validate_active_app(
    targets: &Targets,
    legacy: &Legacy,
    validate_dir: fn(&Path) -> io::Result<()>,
    explicit_data_dir: bool,
) -> io::Result<()> {
    if explicit_data_dir {
        return Ok(());
    }
    let (store, bytes) = read_legacy_store(&targets.tauri)?;
    if bytes.is_none() {
        if targets.app == targets.device {
            return Ok(());
        }
        return Err(invalid("Completed migration is missing app_paths.json"));
    }
    match store.get("app_config_dir_override") {
        None => Ok(()),
        Some(Value::String(raw)) if raw.trim().is_empty() => Ok(()),
        Some(Value::String(raw)) => {
            let path = resolve_override(raw.trim(), &legacy.home);
            reject_broad_source(&path, legacy)?;
            validate_app_override(&path, targets, validate_dir)
        }
        Some(_) => Err(invalid("app_config_dir_override must be a string")),
    }
}

fn validate_app_override(
    path: &Path,
    targets: &Targets,
    validate_dir: fn(&Path) -> io::Result<()>,
) -> io::Result<()> {
    validate_dir(path)?;
    if fs::canonicalize(path)? != fs::canonicalize(&targets.app)? {
        reject_target_overlap(path, targets)?;
    }
    validate_owned_tree(path, true)
}

pub(crate) fn validate_app_override_for_current_targets(path: &Path) -> io::Result<()> {
    let targets = Targets {
        app: crate::data_paths::app_dir(),
        device: crate::data_paths::device_dir(),
        tauri: crate::data_paths::tauri_dir(),
        webview: crate::data_paths::webview_dir(),
        cache: crate::data_paths::cache_dir(),
    };
    validate_app_override(path, &targets, crate::data_paths::validate_data_dir)
}

fn resolve_override(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        return home.to_path_buf();
    }
    raw.strip_prefix("~/")
        .or_else(|| raw.strip_prefix("~\\"))
        .map_or_else(|| PathBuf::from(raw), |suffix| home.join(suffix))
}

fn legacy_default_app(default: &Path, home_env: Option<&Path>) -> io::Result<PathBuf> {
    if metadata_if_present(&default.join("cc-switch.db"))?.is_none() {
        if let Some(home_env) = home_env {
            let candidate = home_env.join(".cc-switch");
            if metadata_if_present(&candidate.join("cc-switch.db"))?.is_some() {
                return Ok(candidate);
            }
        }
    }
    Ok(default.to_path_buf())
}

fn snapshot_known_device(source: &Path) -> io::Result<SourceSnapshot> {
    if metadata_if_present(source)?.is_none() {
        return Ok(None);
    }
    require_source_directory(source)?;
    let mut entries = BTreeMap::new();
    for name in [
        "settings.json",
        "live-state.json",
        "codex-login-stash.json",
        "codex-catalog-history.json",
        "codex-login-history.json",
        "codex-client-ack.json",
        "codex-official-models.json",
    ] {
        snapshot_known_entry(source, Path::new(name), &mut entries)?;
    }
    let backups = source.join("backups");
    if metadata_if_present(&backups)?.is_some() {
        require_source_directory(&backups)?;
        let before = entries.len();
        for name in ["live-first-write", "proxy-live-backup"] {
            snapshot_known_entry(source, &Path::new("backups").join(name), &mut entries)?;
        }
        for entry in fs::read_dir(&backups)? {
            let entry = entry?;
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|value| value.starts_with("env-backup-") && value.ends_with(".json"))
            {
                snapshot_known_entry(source, &Path::new("backups").join(name), &mut entries)?;
            }
        }
        if entries.len() != before {
            entries.insert(PathBuf::from("backups"), SourceEntry::Directory);
        }
    }
    Ok(Some(entries))
}

fn snapshot_known_entry(
    root: &Path,
    relative: &Path,
    entries: &mut BTreeMap<PathBuf, SourceEntry>,
) -> io::Result<()> {
    let path = root.join(relative);
    let Some(metadata) = metadata_if_present(&path)? else {
        return Ok(());
    };
    reject_link(&path, &metadata)?;
    if metadata.is_dir() {
        entries.insert(relative.to_path_buf(), SourceEntry::Directory);
        snapshot_contents(&path, root, None, false, entries)?;
    } else {
        require_plain_file(&path, &metadata)?;
        entries.insert(
            relative.to_path_buf(),
            SourceEntry::File {
                length: metadata.len(),
                modified: metadata.modified()?,
                readonly: metadata.permissions().readonly(),
            },
        );
    }
    Ok(())
}

fn copy_known_device_entries(
    source: &Path,
    target: &Path,
    snapshot: &SourceSnapshot,
) -> io::Result<()> {
    if let Some(entries) = snapshot {
        for (relative, entry) in entries {
            let destination = target.join(relative);
            match entry {
                SourceEntry::Directory => ensure_directory(&destination)?,
                SourceEntry::File { .. } => copy_file(&source.join(relative), &destination)?,
                SourceEntry::SharedLink(_) => return Err(invalid("Unexpected device link")),
            }
        }
    }
    Ok(())
}

fn preflight_known_device(
    source: &Path,
    target: &Path,
    snapshot: &SourceSnapshot,
) -> io::Result<()> {
    if let Some(entries) = snapshot {
        for (relative, entry) in entries {
            let destination = target.join(relative);
            let Some(metadata) = metadata_if_present(&destination)? else {
                continue;
            };
            match entry {
                SourceEntry::Directory => require_plain_directory(&destination)?,
                SourceEntry::File { .. } => {
                    require_plain_file(&destination, &metadata)?;
                    require_equal(&source.join(relative), &destination)?;
                }
                SourceEntry::SharedLink(_) => return Err(invalid("Unexpected device link")),
            }
        }
    }
    Ok(())
}

fn verify_known_device(source: &Path, target: &Path, before: &SourceSnapshot) -> io::Result<()> {
    if &snapshot_known_device(source)? != before {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Device source changed during migration",
        ));
    }
    if let Some(entries) = before {
        for (relative, entry) in entries {
            let destination = target.join(relative);
            match entry {
                SourceEntry::Directory => require_plain_directory(&destination)?,
                SourceEntry::File { .. } => require_equal(&source.join(relative), &destination)?,
                SourceEntry::SharedLink(_) => return Err(invalid("Unexpected device link")),
            }
        }
    }
    Ok(())
}

fn snapshot_source(source: &Path, skip: Option<&str>, allow: bool) -> io::Result<SourceSnapshot> {
    if metadata_if_present(source)?.is_none() {
        return Ok(None);
    }
    require_source_directory(source)?;
    let mut entries = BTreeMap::new();
    snapshot_contents(source, source, skip, allow, &mut entries)?;
    Ok(Some(entries))
}

fn skip_root_entry(name: &std::ffi::OsStr, skip: Option<&str>) -> bool {
    skip.is_some_and(|value| name == value) || (skip == Some(STORE_FILE) && name == MARKER_FILE)
}

fn snapshot_contents(
    directory: &Path,
    root: &Path,
    skip: Option<&str>,
    allow: bool,
    entries: &mut BTreeMap<PathBuf, SourceEntry>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if skip_root_entry(&entry.file_name(), skip) {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|error| invalid(error.to_string()))?;
        let metadata = fs::symlink_metadata(&path)?;
        if let Some(target) = managed_shared_link_target(&path, root, &metadata, allow)? {
            entries.insert(relative.to_path_buf(), SourceEntry::SharedLink(target));
            continue;
        }
        reject_link(&path, &metadata)?;
        if metadata.is_dir() {
            entries.insert(relative.to_path_buf(), SourceEntry::Directory);
            snapshot_contents(&path, root, None, allow, entries)?;
        } else {
            require_plain_file(&path, &metadata)?;
            entries.insert(
                relative.to_path_buf(),
                SourceEntry::File {
                    length: metadata.len(),
                    modified: metadata.modified()?,
                    readonly: metadata.permissions().readonly(),
                },
            );
        }
    }
    Ok(())
}

fn verify_source(
    source: &Path,
    target: &Path,
    skip: Option<&str>,
    allow: bool,
    before: &SourceSnapshot,
) -> io::Result<()> {
    if snapshot_source(source, skip, allow)? != *before {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!(
                "Migration source changed before completion: {}",
                source.display()
            ),
        ));
    }
    if let Some(entries) = before {
        for (relative, entry) in entries {
            let target_path = target.join(relative);
            match entry {
                SourceEntry::Directory => require_plain_directory(&target_path)?,
                SourceEntry::File { .. } => {
                    require_plain_file(&target_path, &fs::symlink_metadata(&target_path)?)?;
                    require_equal(&source.join(relative), &target_path)?;
                }
                SourceEntry::SharedLink(expected) => {
                    if !is_directory_link(&fs::symlink_metadata(&target_path)?)
                        || absolute_link_target(&target_path)? != *expected
                    {
                        return Err(conflict(&target_path));
                    }
                }
            }
        }
    }
    Ok(())
}

fn copy_tree(
    source: &Path,
    target: &Path,
    skip_root_file: Option<&str>,
    allow_shared_links: bool,
) -> io::Result<()> {
    if metadata_if_present(source)?.is_none() {
        return Ok(());
    }
    require_source_directory(source)?;
    require_plain_directory(target)?;
    let source_real = fs::canonicalize(source)?;
    let target_real = fs::canonicalize(target)?;
    if source_real == target_real {
        return validate_owned_tree(source, allow_shared_links);
    }
    if target_real.starts_with(&source_real) || source_real.starts_with(&target_real) {
        return Err(invalid(format!(
            "Migration directories overlap: {} and {}",
            source.display(),
            target.display()
        )));
    }
    copy_contents(source, target, skip_root_file, source, allow_shared_links)
}

fn copy_contents(
    source: &Path,
    target: &Path,
    skip_root_file: Option<&str>,
    source_root: &Path,
    allow_shared_links: bool,
) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        if skip_root_entry(&entry.file_name(), skip_root_file) {
            continue;
        }
        let metadata = fs::symlink_metadata(&source_path)?;
        let target_path = target.join(entry.file_name());
        if let Some(link_target) =
            managed_shared_link_target(&source_path, source_root, &metadata, allow_shared_links)?
        {
            copy_shared_link(&link_target, &target_path)?;
            continue;
        }
        reject_link(&source_path, &metadata)?;
        if metadata.is_dir() {
            ensure_directory(&target_path)?;
            copy_contents(
                &source_path,
                &target_path,
                None,
                source_root,
                allow_shared_links,
            )?;
        } else if metadata.is_file() {
            copy_file(&source_path, &target_path)?;
        } else {
            return Err(invalid(format!(
                "Unsupported migration source: {}",
                source_path.display()
            )));
        }
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path) -> io::Result<()> {
    if let Some(metadata) = metadata_if_present(target)? {
        require_plain_file(target, &metadata)?;
        return require_equal(source, target);
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent(target)?)?;
    io::copy(&mut File::open(source)?, &mut temporary)?;
    temporary.as_file().sync_all()?;
    require_equal(source, temporary.path())?;
    match temporary.persist_noclobber(target) {
        Ok(_) => require_equal(source, target),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            require_plain_file(target, &fs::symlink_metadata(target)?)?;
            require_equal(source, target)
        }
        Err(error) => Err(error.error),
    }
}

fn publish_bytes(target: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(metadata) = metadata_if_present(target)? {
        require_plain_file(target, &metadata)?;
        if fs::read(target)? == bytes {
            return Ok(());
        }
        return Err(conflict(target));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent(target)?)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    match temporary.persist_noclobber(target) {
        Ok(_) => {}
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            require_plain_file(target, &fs::symlink_metadata(target)?)?;
        }
        Err(error) => return Err(error.error),
    }
    if fs::read(target)? != bytes {
        return Err(conflict(target));
    }
    Ok(())
}

fn require_equal(source: &Path, target: &Path) -> io::Result<()> {
    let mut source_file = File::open(source)?;
    let mut target_file = File::open(target)?;
    if source_file.metadata()?.len() != target_file.metadata()?.len() {
        return Err(conflict(target));
    }
    let mut source_bytes = [0; 64 * 1024];
    let mut target_bytes = [0; 64 * 1024];
    loop {
        let count = source_file.read(&mut source_bytes)?;
        if count == 0 {
            return Ok(());
        }
        target_file.read_exact(&mut target_bytes[..count])?;
        if source_bytes[..count] != target_bytes[..count] {
            return Err(conflict(target));
        }
    }
}

fn metadata_if_present(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn reject_link(path: &Path, metadata: &Metadata) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid(format!(
                "Reparse point in migration: {}",
                path.display()
            )));
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(invalid(format!("Link in migration: {}", path.display())));
    }
    Ok(())
}

fn require_plain_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    reject_link(path, &metadata)?;
    if !metadata.is_dir() {
        return Err(invalid(format!("Expected a directory: {}", path.display())));
    }
    Ok(())
}

fn require_source_directory(path: &Path) -> io::Result<()> {
    require_plain_directory(path)?;
    let absolute = std::path::absolute(path)?;
    for ancestor in absolute.ancestors() {
        reject_link(ancestor, &fs::symlink_metadata(ancestor)?)?;
    }
    Ok(())
}

fn reject_broad_source(source: &Path, legacy: &Legacy) -> io::Result<()> {
    if std::path::absolute(source)?.parent().is_none() {
        return Err(invalid(
            "A drive root cannot be an application migration source",
        ));
    }
    require_source_directory(source)?;
    let source = fs::canonicalize(source)?;
    for root in [
        Some(legacy.home.as_path()),
        legacy.tauri.parent(),
        Some(legacy.local_data.as_path()),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(root) = fs::canonicalize(root) {
            if root.starts_with(&source) {
                return Err(invalid(format!(
                    "Legacy configuration points at a broad user directory: {}",
                    source.display()
                )));
            }
        }
    }
    if source.parent().is_none() {
        return Err(invalid(
            "A drive root cannot be an application migration source",
        ));
    }
    Ok(())
}

fn reject_target_overlap(source: &Path, targets: &Targets) -> io::Result<()> {
    let source = fs::canonicalize(source)?;
    for target in [
        &targets.app,
        &targets.device,
        &targets.tauri,
        &targets.webview,
        &targets.cache,
    ] {
        let target_real = fs::canonicalize(target)?;
        if (target_real.starts_with(&source)
            && (source != target_real || target.as_path() != targets.app.as_path()))
            || (target.as_path() != targets.app.as_path() && source.starts_with(&target_real))
        {
            return Err(invalid(
                "Legacy app directory overlaps another migration target",
            ));
        }
    }
    Ok(())
}

fn require_plain_file(path: &Path, metadata: &Metadata) -> io::Result<()> {
    reject_link(path, metadata)?;
    if !metadata.is_file() {
        return Err(invalid(format!("Expected a file: {}", path.display())));
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    if metadata_if_present(path)?.is_none() {
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    require_plain_directory(path)
}

fn validate_owned_tree(path: &Path, allow_shared_links: bool) -> io::Result<()> {
    require_plain_directory(path)?;
    validate_owned_contents(path, path, allow_shared_links)
}

fn validate_owned_contents(path: &Path, root: &Path, allow_shared_links: bool) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if managed_shared_link_target(&path, root, &metadata, allow_shared_links)?.is_some() {
            continue;
        }
        reject_link(&path, &metadata)?;
        if metadata.is_dir() {
            validate_owned_contents(&path, root, allow_shared_links)?;
        } else {
            require_plain_file(&path, &metadata)?;
            File::open(&path)?;
        }
    }
    Ok(())
}

fn managed_shared_link_target(
    path: &Path,
    root: &Path,
    metadata: &Metadata,
    allow_shared_links: bool,
) -> io::Result<Option<PathBuf>> {
    if !allow_shared_links || !is_link(metadata) {
        return Ok(None);
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return Ok(None);
    };
    let components: Vec<_> = relative.iter().collect();
    let claude_shared = components.len() == 4
        && components[0] == "claude-accounts"
        && components[2] == "config"
        && components[1]
            .to_str()
            .is_some_and(crate::services::claude_account::layout::valid_local_id)
        && components[3]
            .to_str()
            .is_some_and(|name| LINKED_DIRS.contains(&name));
    let agy_shared = components.len() == 3
        && components[0] == "agy"
        && components[2] == "config"
        && components[1].to_str().is_some_and(|name| {
            // agy_config::profile_dir_name uses this exact safe-segment rule.
            !name.is_empty()
                && name != "."
                && name != ".."
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        });
    if !claude_shared && !agy_shared {
        return Ok(None);
    }
    if !is_directory_link(metadata) {
        return Err(invalid("A managed shared link must be a directory link"));
    }
    absolute_link_target(path).map(Some)
}

fn absolute_link_target(path: &Path) -> io::Result<PathBuf> {
    let target = fs::read_link(path)?;
    if target.is_absolute() {
        std::path::absolute(target)
    } else {
        std::path::absolute(parent(path)?.join(target))
    }
}

fn copy_shared_link(expected: &Path, target: &Path) -> io::Result<()> {
    if let Some(metadata) = metadata_if_present(target)? {
        if is_directory_link(&metadata) && absolute_link_target(target)? == expected {
            return Ok(());
        }
        return Err(conflict(target));
    }
    if let Err(error) = crate::agy_config::create_dir_link(expected, target) {
        if let Some(metadata) = metadata_if_present(target)? {
            if is_directory_link(&metadata) && absolute_link_target(target)? == expected {
                return Ok(());
            }
        }
        return Err(io::Error::other(error.to_string()));
    }
    if !is_directory_link(&fs::symlink_metadata(target)?)
        || absolute_link_target(target)? != expected
    {
        return Err(conflict(target));
    }
    Ok(())
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn is_directory_link(metadata: &Metadata) -> bool {
    if !is_link(metadata) {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x10 != 0
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn parent(path: &Path) -> io::Result<&Path> {
    path.parent()
        .ok_or_else(|| invalid("Migration target has no parent"))
}

fn is_d_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        matches!(
            path.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(b'D' | b'd') | Prefix::VerbatimDisk(b'D' | b'd'))
        )
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn conflict(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "Migration target has different contents: {}",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        targets: Targets,
        legacy: Legacy,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let targets = Targets {
                app: root.path().join("target/app"),
                device: root.path().join("target/device"),
                tauri: root.path().join("target/tauri"),
                webview: root.path().join("target/webview"),
                cache: root.path().join("target/cache"),
            };
            let legacy = Legacy {
                home: root.path().join("old/home"),
                home_env: None,
                tauri: root.path().join("old/tauri"),
                local_data: root.path().join("old/local"),
                webview: root.path().join("old/webview"),
                local_d: None,
            };
            Self {
                _root: root,
                targets,
                legacy,
            }
        }

        fn run(&self) -> io::Result<()> {
            migrate_from(
                &self.targets,
                &self.legacy,
                prepare_fixture_dir,
                fixture_on_d,
            )
        }

        fn marker(&self) -> PathBuf {
            self.targets.tauri.join(MARKER_FILE)
        }

        fn assert_pending(&self) {
            let marker: Completion =
                serde_json::from_slice(&fs::read(self.marker()).unwrap()).unwrap();
            assert!(marker.pending_source.is_some());
        }

        fn store(&self, value: Value) {
            write(
                self.legacy.tauri.join(STORE_FILE),
                serde_json::to_vec(&value).unwrap(),
            );
        }
    }

    fn fixture_on_d(path: &Path) -> bool {
        path.file_name().is_some_and(|name| name == "custom-d")
    }

    fn prepare_fixture_dir(path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn prompted(
        fixture: &Fixture,
        ask: impl FnMut(&SourceRoots, &Path) -> io::Result<MigrationDecision>,
    ) -> io::Result<()> {
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            false,
            true,
            ask,
        )
    }

    #[test]
    fn app_override_rejects_other_roots_children_and_common_parent() {
        let fixture = Fixture::new();
        for path in [
            &fixture.targets.app,
            &fixture.targets.device,
            &fixture.targets.tauri,
            &fixture.targets.webview,
            &fixture.targets.cache,
        ] {
            fs::create_dir_all(path.join("child")).unwrap();
        }
        let separate = fixture._root.path().join("separate");
        fs::create_dir_all(&separate).unwrap();
        assert!(
            validate_app_override(&fixture.targets.app, &fixture.targets, prepare_fixture_dir)
                .is_ok()
        );
        assert!(validate_app_override(&separate, &fixture.targets, prepare_fixture_dir).is_ok());
        assert!(validate_app_override(
            fixture.targets.app.parent().unwrap(),
            &fixture.targets,
            prepare_fixture_dir
        )
        .is_err());
        for root in [
            &fixture.targets.device,
            &fixture.targets.tauri,
            &fixture.targets.webview,
            &fixture.targets.cache,
        ] {
            assert!(validate_app_override(root, &fixture.targets, prepare_fixture_dir).is_err());
            assert!(validate_app_override(
                &root.join("child"),
                &fixture.targets,
                prepare_fixture_dir
            )
            .is_err());
        }
    }

    #[test]
    fn direct_root_override_uses_same_validation_for_save_and_restart() {
        let mut fixture = Fixture::new();
        let root = fixture._root.path().join("direct/data");
        fixture.targets = Targets {
            app: root.clone(),
            device: root.clone(),
            tauri: root.join("tauri"),
            webview: root.join("webview2"),
            cache: root.join("cache"),
        };
        for path in [
            &fixture.targets.app,
            &fixture.targets.tauri,
            &fixture.targets.webview,
            &fixture.targets.cache,
        ] {
            fs::create_dir_all(path).unwrap();
        }
        assert!(validate_app_override(&root, &fixture.targets, prepare_fixture_dir).is_ok());
        let store = serde_json::json!({"app_config_dir_override":root.to_str().unwrap()});
        write(
            fixture.targets.tauri.join(STORE_FILE),
            serde_json::to_vec(&store).unwrap(),
        );
        assert!(validate_active_app(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            false
        )
        .is_ok());
        assert!(validate_app_override(
            &fixture.targets.tauri,
            &fixture.targets,
            prepare_fixture_dir
        )
        .is_err());
    }

    fn portable_targets(base: &Path) -> Targets {
        Targets {
            app: base.join("data/app"),
            device: base.join("data/device"),
            tauri: base.join("data/tauri"),
            webview: base.join("data/webview2"),
            cache: base.join("cache"),
        }
    }

    fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
        let path = path.as_ref();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn prompt_yes_copies_old_data_and_keeps_the_source() {
        let fixture = Fixture::new();
        let old = fixture.legacy.home.join(".cc-switch/cc-switch.db");
        write(&old, b"old database");
        let mut asked = 0;
        prompted(&fixture, |_, _| {
            asked += 1;
            Ok(MigrationDecision::Migrate)
        })
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(fs::read(&old).unwrap(), b"old database");
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
            b"old database"
        );
        assert!(fixture.marker().is_file());
    }

    #[test]
    fn prompt_displays_resolved_legacy_override_and_related_directories() {
        let fixture = Fixture::new();
        let custom = fixture.legacy.home.join("chosen-old-data");
        write(custom.join("cc-switch.db"), b"old");
        fixture.store(serde_json::json!({"app_config_dir_override":"~/chosen-old-data"}));
        prompted(&fixture, |source, target| {
            assert_eq!(source.app, custom);
            assert_eq!(source.device, fixture.legacy.home.join(".cc-switch"));
            assert_eq!(source.tauri, fixture.legacy.tauri);
            assert_eq!(source.webview, fixture.legacy.webview);
            assert_eq!(target, fixture.targets.app.as_path());
            Ok(MigrationDecision::StartFresh)
        })
        .unwrap();
        assert_eq!(fs::read(custom.join("cc-switch.db")).unwrap(), b"old");
    }

    #[test]
    fn prompt_no_records_choice_and_cancel_does_not_publish() {
        let fixture = Fixture::new();
        let old = fixture.legacy.home.join(".cc-switch/cc-switch.db");
        write(&old, b"old database");
        let err = prompted(&fixture, |_, _| Ok(MigrationDecision::Cancel)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(!fixture.marker().exists());
        assert!(!fixture.targets.app.join("cc-switch.db").exists());
        prompted(&fixture, |_, _| Ok(MigrationDecision::StartFresh)).unwrap();
        prompted(&fixture, |_, _| {
            panic!("declined source must not prompt again")
        })
        .unwrap();
        assert!(!fixture.targets.app.join("cc-switch.db").exists());
        assert_eq!(fs::read(old).unwrap(), b"old database");
    }

    #[test]
    fn no_legacy_or_existing_target_never_prompts_or_overwrites() {
        let fresh = Fixture::new();
        prompted(&fresh, |_, _| panic!("no source")).unwrap();
        let existing = Fixture::new();
        write(existing.targets.app.join("cc-switch.db"), b"current");
        write(existing.legacy.home.join(".cc-switch/cc-switch.db"), b"old");
        prompted(&existing, |_, _| panic!("target already has data")).unwrap();
        assert_eq!(
            fs::read(existing.targets.app.join("cc-switch.db")).unwrap(),
            b"current"
        );
    }

    #[test]
    fn direct_root_initial_scaffolding_is_empty_but_existing_webview_is_not() {
        let mut fixture = Fixture::new();
        let root = fixture._root.path().join("new/data");
        fixture.targets = Targets {
            app: root.clone(),
            device: root.clone(),
            tauri: root.join("tauri"),
            webview: root.join("webview2"),
            cache: root.join("cache"),
        };
        for path in [
            &fixture.targets.app,
            &fixture.targets.tauri,
            &fixture.targets.webview,
            &fixture.targets.cache,
        ] {
            fs::create_dir_all(path).unwrap();
        }
        write(fixture.legacy.home.join(".cc-switch/cc-switch.db"), b"old");
        let mut asked = 0;
        prompted(&fixture, |_, _| {
            asked += 1;
            Ok(MigrationDecision::Migrate)
        })
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(fs::read(root.join("cc-switch.db")).unwrap(), b"old");

        let mut occupied = Fixture::new();
        let root = occupied._root.path().join("new/data");
        occupied.targets = Targets {
            app: root.clone(),
            device: root.clone(),
            tauri: root.join("tauri"),
            webview: root.join("webview2"),
            cache: root.join("cache"),
        };
        for path in [
            &occupied.targets.app,
            &occupied.targets.tauri,
            &occupied.targets.webview,
            &occupied.targets.cache,
        ] {
            fs::create_dir_all(path).unwrap();
        }
        write(occupied.targets.webview.join("profile"), b"existing");
        write(occupied.legacy.home.join(".cc-switch/cc-switch.db"), b"old");
        prompted(&occupied, |_, _| panic!("existing profile")).unwrap();
        assert_eq!(
            fs::read(occupied.targets.webview.join("profile")).unwrap(),
            b"existing"
        );
        assert!(!root.join("cc-switch.db").exists());
    }

    #[test]
    fn local_source_errors_skip_only_confirmed_missing_paths() {
        assert!(optional_local_missing(&io::Error::new(
            io::ErrorKind::NotFound,
            "missing"
        )));
        assert!(!optional_local_missing(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "denied"
        )));
        assert!(!optional_local_missing(&io::Error::new(
            io::ErrorKind::InvalidData,
            "bad"
        )));
        let mut fixture = Fixture::new();
        let local = portable_targets(&fixture._root.path().join("bad-local"));
        write(&local.tauri, b"not a directory");
        fixture.legacy.local_d = Some(local);
        assert!(prompted(&fixture, |_, _| panic!("invalid source")).is_err());
        assert!(!fixture.marker().exists());
    }

    #[test]
    fn existing_local_root_with_missing_override_is_not_skipped() {
        let mut fixture = Fixture::new();
        let local = portable_targets(&fixture._root.path().join("old-d"));
        let store_path = local.tauri.join(STORE_FILE);
        let store = br#"{"app_config_dir_override":"~/missing-old-app"}"#;
        write(&store_path, store);
        fixture.legacy.local_d = Some(local);
        assert_eq!(
            prompted(&fixture, |_, _| panic!("invalid source must not prompt"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(!fixture.marker().exists());
        assert_eq!(fs::read(store_path).unwrap(), store);
    }

    #[test]
    fn explicit_direct_old_root_is_reused_without_copy_or_prompt() {
        let mut fixture = Fixture::new();
        let old = fixture.legacy.home.join(".cc-switch");
        write(old.join("cc-switch.db"), b"existing");
        fixture.targets = Targets {
            app: old.clone(),
            device: old.clone(),
            tauri: old.join("tauri"),
            webview: old.join("webview2"),
            cache: old.join("cache"),
        };
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            true,
            true,
            |_, _| panic!("already in use"),
        )
        .unwrap();
        assert_eq!(fs::read(old.join("cc-switch.db")).unwrap(), b"existing");
        assert!(!fixture.marker().exists());
    }

    #[test]
    fn direct_root_offers_official_auxiliary_without_touching_database() {
        for decision in [MigrationDecision::Migrate, MigrationDecision::StartFresh] {
            let mut fixture = Fixture::new();
            let old = fixture.legacy.home.join(".cc-switch");
            write(old.join("cc-switch.db"), b"existing");
            fixture.targets = Targets {
                app: old.clone(),
                device: old.clone(),
                tauri: old.join("tauri"),
                webview: old.join("webview2"),
                cache: old.join("cache"),
            };
            fixture
                .store(serde_json::json!({"app_config_dir_override":"~/elsewhere", "other":true}));
            write(fixture.legacy.webview.join("profile"), b"browser");
            let mut asked = 0;
            migrate_prompted(
                &fixture.targets,
                &fixture.legacy,
                prepare_fixture_dir,
                fixture_on_d,
                true,
                true,
                |source, _| {
                    asked += 1;
                    assert!(source.auxiliary_only);
                    assert_eq!(source.tauri, fixture.legacy.tauri);
                    assert_eq!(source.webview, fixture.legacy.webview);
                    Ok(decision)
                },
            )
            .unwrap();
            assert_eq!(asked, 1);
            assert_eq!(fs::read(old.join("cc-switch.db")).unwrap(), b"existing");
            assert_eq!(
                fs::read(fixture.legacy.webview.join("profile")).unwrap(),
                b"browser"
            );
            if decision == MigrationDecision::Migrate {
                assert_eq!(
                    fs::read(fixture.targets.webview.join("profile")).unwrap(),
                    b"browser"
                );
                let store: Value = serde_json::from_slice(
                    &fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap(),
                )
                .unwrap();
                assert!(store.get("app_config_dir_override").is_none());
                assert_eq!(store["other"], true);
            } else {
                assert!(!fixture.targets.webview.join("profile").exists());
            }
            migrate_prompted(
                &fixture.targets,
                &fixture.legacy,
                prepare_fixture_dir,
                fixture_on_d,
                true,
                true,
                |_, _| panic!("already decided"),
            )
            .unwrap();
        }
    }

    #[test]
    fn copied_direct_db_offers_auxiliary_only_for_matching_official_source() {
        let mut fixture = Fixture::new();
        let root = fixture._root.path().join("new/data");
        fixture.targets = Targets {
            app: root.clone(),
            device: root.clone(),
            tauri: root.join("tauri"),
            webview: root.join("webview2"),
            cache: root.join("cache"),
        };
        write(root.join("cc-switch.db"), b"same");
        write(fixture.legacy.home.join(".cc-switch/cc-switch.db"), b"same");
        write(fixture.legacy.webview.join("profile"), b"browser");
        let mut asked = false;
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            false,
            true,
            |source, _| {
                asked = true;
                assert!(source.auxiliary_only);
                Ok(MigrationDecision::StartFresh)
            },
        )
        .unwrap();
        assert!(asked);
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            false,
            true,
            |_, _| panic!("already declined"),
        )
        .unwrap();
    }

    #[test]
    fn in_place_custom_override_offers_known_device_and_auxiliary_data() {
        for decision in [MigrationDecision::Migrate, MigrationDecision::StartFresh] {
            let mut fixture = Fixture::new();
            let custom = fixture.legacy.home.join("custom");
            let device = fixture.legacy.home.join(".cc-switch");
            fixture.targets = Targets {
                app: custom.clone(),
                device: custom.clone(),
                tauri: custom.join("tauri"),
                webview: custom.join("webview2"),
                cache: custom.join("cache"),
            };
            write(custom.join("cc-switch.db"), b"current database");
            write(device.join("cc-switch.db"), b"unrelated old database");
            write(device.join("settings.json"), b"device settings");
            write(device.join("unknown/keep"), b"old only");
            write(fixture.legacy.webview.join("profile"), b"browser");
            fixture.store(serde_json::json!({"app_config_dir_override":"~/custom", "other":true}));
            let mut asked = 0;
            migrate_prompted(
                &fixture.targets,
                &fixture.legacy,
                prepare_fixture_dir,
                fixture_on_d,
                true,
                true,
                |source, _| {
                    asked += 1;
                    assert!(source.auxiliary_only);
                    assert_eq!(source.device, device);
                    Ok(decision)
                },
            )
            .unwrap();
            assert_eq!(asked, 1);
            assert_eq!(
                fs::read(custom.join("cc-switch.db")).unwrap(),
                b"current database"
            );
            assert_eq!(
                fs::read(device.join("cc-switch.db")).unwrap(),
                b"unrelated old database"
            );
            assert_eq!(fs::read(device.join("unknown/keep")).unwrap(), b"old only");
            assert!(!custom.join("unknown/keep").exists());
            if decision == MigrationDecision::Migrate {
                assert_eq!(
                    fs::read(custom.join("settings.json")).unwrap(),
                    b"device settings"
                );
                assert_eq!(
                    fs::read(fixture.targets.webview.join("profile")).unwrap(),
                    b"browser"
                );
                let store: Value = serde_json::from_slice(
                    &fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap(),
                )
                .unwrap();
                assert!(store.get("app_config_dir_override").is_none());
                assert_eq!(store["other"], true);
            } else {
                assert!(!custom.join("settings.json").exists());
            }
            migrate_prompted(
                &fixture.targets,
                &fixture.legacy,
                prepare_fixture_dir,
                fixture_on_d,
                true,
                true,
                |_, _| panic!("already decided"),
            )
            .unwrap();
        }
    }

    #[test]
    fn custom_override_must_match_and_device_conflicts_leave_no_pending_state() {
        for conflict in [false, true] {
            let mut fixture = Fixture::new();
            let custom = fixture.legacy.home.join("custom");
            let device = fixture.legacy.home.join(".cc-switch");
            fixture.targets = Targets {
                app: custom.clone(),
                device: custom.clone(),
                tauri: custom.join("tauri"),
                webview: custom.join("webview2"),
                cache: custom.join("cache"),
            };
            write(custom.join("cc-switch.db"), b"current");
            write(device.join("cc-switch.db"), b"different");
            write(device.join("settings.json"), b"old settings");
            if conflict {
                write(custom.join("settings.json"), b"current settings");
            }
            fixture.store(serde_json::json!({"app_config_dir_override":if conflict { "~/custom" } else { "~/other" }}));
            let mut asked = 0;
            let result = migrate_prompted(
                &fixture.targets,
                &fixture.legacy,
                prepare_fixture_dir,
                fixture_on_d,
                true,
                true,
                |_, _| {
                    asked += 1;
                    Ok(MigrationDecision::Migrate)
                },
            );
            if conflict {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
                assert_eq!(asked, 1);
                assert_eq!(
                    fs::read(custom.join("settings.json")).unwrap(),
                    b"current settings"
                );
            } else {
                result.unwrap();
                assert_eq!(asked, 0);
            }
            assert!(!fixture.marker().exists());
            assert_eq!(fs::read(custom.join("cc-switch.db")).unwrap(), b"current");
            assert_eq!(
                fs::read(device.join("settings.json")).unwrap(),
                b"old settings"
            );
        }
    }

    #[test]
    fn overlapping_source_stops_before_recording_in_progress() {
        let mut fixture = Fixture::new();
        let old = fixture.legacy.home.join(".cc-switch");
        write(old.join("cc-switch.db"), b"old");
        let nested = old.join("new-install");
        fixture.targets = portable_targets(&nested);
        assert!(prompted(&fixture, |_, _| Ok(MigrationDecision::Migrate)).is_err());
        assert!(!fixture.marker().exists());
        assert_eq!(fs::read(old.join("cc-switch.db")).unwrap(), b"old");
    }

    #[test]
    fn matching_marker_and_explicit_data_ignore_stale_store_override() {
        let fixture = Fixture::new();
        write(fixture.legacy.home.join(".cc-switch/cc-switch.db"), b"data");
        fixture.run().unwrap();
        let bytes = br#"{"app_config_dir_override":"Z:\\unavailable"}"#;
        write(fixture.targets.tauri.join(STORE_FILE), bytes);
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            true,
            true,
            |_, _| panic!("matching marker"),
        )
        .unwrap();
        assert_eq!(
            fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap(),
            bytes
        );
        write(fixture.targets.tauri.join(STORE_FILE), b"invalid json");
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            true,
            true,
            |_, _| panic!("matching marker"),
        )
        .unwrap();
        assert_eq!(
            fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap(),
            b"invalid json"
        );
        fs::remove_file(fixture.targets.tauri.join(STORE_FILE)).unwrap();
        migrate_prompted(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            true,
            true,
            |_, _| panic!("matching marker"),
        )
        .unwrap();
        assert!(!fixture.targets.tauri.join(STORE_FILE).exists());
    }

    #[test]
    fn multiple_sources_require_selection_and_never_mix_or_copy_cache() {
        for choose_local in [true, false] {
            let mut fixture = Fixture::new();
            let local = portable_targets(&fixture._root.path().join("old-d"));
            write(local.app.join("cc-switch.db"), b"local database");
            write(local.tauri.join(STORE_FILE), b"{}");
            write(
                local.tauri.join(MARKER_FILE),
                completion_bytes(&local, None).unwrap(),
            );
            write(local.cache.join("tmp/unrelated"), b"do not copy");
            write(
                fixture.legacy.home.join(".cc-switch/cc-switch.db"),
                b"official database",
            );
            fixture.legacy.local_d = Some(local.clone());
            let mut prompted_paths = Vec::new();
            prompted(&fixture, |source, _| {
                prompted_paths.push(source.app.clone());
                Ok(if choose_local || prompted_paths.len() > 1 {
                    MigrationDecision::Migrate
                } else {
                    MigrationDecision::StartFresh
                })
            })
            .unwrap();
            assert_eq!(prompted_paths.len(), if choose_local { 1 } else { 2 });
            assert_eq!(
                fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
                if choose_local {
                    b"local database".as_slice()
                } else {
                    b"official database".as_slice()
                }
            );
            assert!(!fixture.targets.cache.join("tmp/unrelated").exists());
            assert!(local.cache.join("tmp/unrelated").exists());
        }
    }

    #[test]
    fn completed_marker_allows_cache_change_without_copying_data() {
        let mut fixture = Fixture::new();
        write(fixture.legacy.home.join(".cc-switch/cc-switch.db"), b"old");
        fixture.run().unwrap();
        fixture.targets.cache = fixture._root.path().join("new-cache");
        prompted(&fixture, |_, _| panic!("cache change must not prompt")).unwrap();
        let marker: Completion =
            serde_json::from_slice(&fs::read(fixture.marker()).unwrap()).unwrap();
        assert_eq!(marker.targets, fixture.targets);
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn moving_complete_portable_layout_rebases_marker() {
        let fixture = Fixture::new();
        let old_base = fixture._root.path().join("old-install");
        let new_base = fixture._root.path().join("moved-install");
        let old = portable_targets(&old_base);
        write(old.app.join("cc-switch.db"), b"data");
        for dir in [&old.device, &old.tauri, &old.webview, &old.cache] {
            fs::create_dir_all(dir).unwrap();
        }
        write(old.tauri.join(STORE_FILE), b"{}");
        write(
            old.tauri.join(MARKER_FILE),
            completion_bytes(&old, None).unwrap(),
        );
        fs::rename(&old_base, &new_base).unwrap();
        let moved = portable_targets(&new_base);
        migrate_prompted(
            &moved,
            &fixture.legacy,
            prepare_fixture_dir,
            fixture_on_d,
            false,
            true,
            |_, _| panic!("moved profile"),
        )
        .unwrap();
        let marker: Completion =
            serde_json::from_slice(&fs::read(moved.tauri.join(MARKER_FILE)).unwrap()).unwrap();
        assert_eq!(marker.targets, moved);
        assert_eq!(fs::read(moved.app.join("cc-switch.db")).unwrap(), b"data");
    }

    #[test]
    fn changed_source_after_prompt_stops_before_copying() {
        let fixture = Fixture::new();
        write(fixture.legacy.home.join("first/cc-switch.db"), b"first");
        write(fixture.legacy.home.join("second/cc-switch.db"), b"second");
        fixture.store(serde_json::json!({"app_config_dir_override":"~/first"}));
        let result = prompted(&fixture, |_, _| {
            fixture.store(serde_json::json!({"app_config_dir_override":"~/second"}));
            Ok(MigrationDecision::Migrate)
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(!fixture.marker().exists());
        assert!(!fixture.targets.app.join("cc-switch.db").exists());
    }

    #[test]
    fn moving_direct_layout_without_store_rebases_and_restarts() {
        let fixture = Fixture::new();
        let old_base = fixture._root.path().join("direct-old");
        let new_base = fixture._root.path().join("direct-moved");
        let data = old_base.join("data");
        let old = Targets {
            app: data.clone(),
            device: data.clone(),
            tauri: data.join("tauri"),
            webview: data.join("webview2"),
            cache: old_base.join("cache"),
        };
        write(old.app.join("cc-switch.db"), b"data");
        for dir in [&old.tauri, &old.webview, &old.cache] {
            fs::create_dir_all(dir).unwrap();
        }
        write(
            old.tauri.join(MARKER_FILE),
            completion_bytes(&old, None).unwrap(),
        );
        fs::rename(&old_base, &new_base).unwrap();
        let data = new_base.join("data");
        let moved = Targets {
            app: data.clone(),
            device: data.clone(),
            tauri: data.join("tauri"),
            webview: data.join("webview2"),
            cache: new_base.join("cache"),
        };
        for _ in 0..2 {
            migrate_prompted(
                &moved,
                &fixture.legacy,
                prepare_fixture_dir,
                fixture_on_d,
                false,
                true,
                |_, _| panic!("moved profile"),
            )
            .unwrap();
        }
        let marker: Completion =
            serde_json::from_slice(&fs::read(moved.tauri.join(MARKER_FILE)).unwrap()).unwrap();
        assert_eq!(marker.targets, moved);
        assert_eq!(fs::read(moved.app.join("cc-switch.db")).unwrap(), b"data");
        assert!(!moved.tauri.join(STORE_FILE).exists());
    }

    #[test]
    fn separate_override_migrates_known_device_data_without_copying_unknowns() {
        let mut fixture = Fixture::new();
        let target = fixture._root.path().join("direct/data");
        fixture.targets = Targets {
            app: target.clone(),
            device: target.clone(),
            tauri: target.join("tauri"),
            webview: target.join("webview2"),
            cache: target.join("cache"),
        };
        let app = fixture.legacy.home.join("custom-app");
        let device = fixture.legacy.home.join(".cc-switch");
        write(app.join("cc-switch.db"), b"chosen database");
        write(device.join("cc-switch.db"), b"other database");
        write(device.join("settings.json"), b"device settings");
        write(device.join("codex-login-stash.json"), b"device state");
        write(
            device.join("backups/live-first-write/state"),
            b"known backup",
        );
        write(device.join("unknown/keep"), b"old only");
        fixture.store(serde_json::json!({"app_config_dir_override":"~/custom-app"}));
        prompted(&fixture, |_, _| Ok(MigrationDecision::Migrate)).unwrap();
        assert_eq!(
            fs::read(target.join("cc-switch.db")).unwrap(),
            b"chosen database"
        );
        assert_eq!(
            fs::read(target.join("settings.json")).unwrap(),
            b"device settings"
        );
        assert_eq!(
            fs::read(target.join("codex-login-stash.json")).unwrap(),
            b"device state"
        );
        assert_eq!(
            fs::read(target.join("backups/live-first-write/state")).unwrap(),
            b"known backup"
        );
        assert!(!target.join("unknown/keep").exists());
        assert_eq!(fs::read(device.join("unknown/keep")).unwrap(), b"old only");
    }

    #[test]
    fn migrates_complete_owned_trees_and_preserves_sources() {
        let fixture = Fixture::new();
        let old = fixture.legacy.home.join(".cc-switch");
        write(old.join("cc-switch.db"), [0, 1, 255]);
        write(old.join("cc-switch.db-wal"), b"quiescent wal");
        write(
            old.join("settings.json"),
            br#"{"claudeConfigDir":"C:/external"}"#,
        );
        write(old.join("unknown/nested.bin"), [255, 0, 7]);
        fixture.store(serde_json::json!({"other": {"keep": true}}));
        write(
            fixture.legacy.tauri.join(".window-state.json"),
            b"window state",
        );
        write(
            fixture
                .legacy
                .webview
                .join("EBWebView/Default/Local Storage/state"),
            b"browser data",
        );

        fixture.run().unwrap();
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
            [0, 1, 255]
        );
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db-wal")).unwrap(),
            b"quiescent wal"
        );
        assert_eq!(
            fs::read(fixture.targets.device.join("settings.json")).unwrap(),
            fs::read(old.join("settings.json")).unwrap()
        );
        assert_eq!(
            fs::read(fixture.targets.app.join("unknown/nested.bin")).unwrap(),
            [255, 0, 7]
        );
        assert!(!fixture.targets.device.join("unknown/nested.bin").exists());
        assert_eq!(
            fs::read(fixture.targets.tauri.join(".window-state.json")).unwrap(),
            b"window state"
        );
        assert_eq!(
            fs::read(
                fixture
                    .targets
                    .webview
                    .join("EBWebView/Default/Local Storage/state")
            )
            .unwrap(),
            b"browser data"
        );
        assert_eq!(fs::read(old.join("cc-switch.db")).unwrap(), [0, 1, 255]);
        assert!(fixture.marker().is_file());
    }

    #[test]
    fn migrates_custom_override_and_preserves_other_store_values() {
        let fixture = Fixture::new();
        let custom = fixture.legacy.home.join("custom-c");
        write(custom.join("cc-switch.db"), b"custom db");
        fixture.store(
            serde_json::json!({"app_config_dir_override": "~/custom-c", "other": [1, 2, 3]}),
        );
        let original = fs::read(fixture.legacy.tauri.join(STORE_FILE)).unwrap();

        fixture.run().unwrap();
        let store: Value =
            serde_json::from_slice(&fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap())
                .unwrap();
        assert!(store.get("app_config_dir_override").is_none());
        assert_eq!(store["other"], serde_json::json!([1, 2, 3]));
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
            b"custom db"
        );
        assert_eq!(
            fs::read(fixture.legacy.tauri.join(STORE_FILE)).unwrap(),
            original
        );
    }

    #[test]
    fn migrates_old_d_custom_directory_without_deleting_source() {
        let fixture = Fixture::new();
        let custom = fixture.legacy.home.join("custom-d");
        write(custom.join("cc-switch.db"), b"existing D db");
        fixture.store(serde_json::json!({"app_config_dir_override": "~/custom-d", "other": 42}));
        let original = fs::read(fixture.legacy.tauri.join(STORE_FILE)).unwrap();

        fixture.run().unwrap();
        let store: Value =
            serde_json::from_slice(&fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap())
                .unwrap();
        assert!(store.get("app_config_dir_override").is_none());
        assert_eq!(store["other"], 42);
        assert_eq!(
            fs::read(fixture.legacy.tauri.join(STORE_FILE)).unwrap(),
            original
        );
        assert_eq!(
            fs::read(custom.join("cc-switch.db")).unwrap(),
            b"existing D db"
        );
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
            b"existing D db"
        );
    }

    #[test]
    fn invalid_store_and_missing_explicit_directory_fail_before_completion() {
        for store in [
            b"bad json".as_slice(),
            b"[]",
            br#"{"app_config_dir_override":"~/missing"}"#,
        ] {
            let fixture = Fixture::new();
            write(fixture.legacy.tauri.join(STORE_FILE), store);
            assert!(fixture.run().is_err());
            assert!(!fixture.marker().exists());
        }
    }

    #[test]
    fn unavailable_target_fails_without_a_completion_marker() {
        fn unavailable(_: &Path) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unavailable",
            ))
        }
        let fixture = Fixture::new();
        assert_eq!(
            migrate_from(&fixture.targets, &fixture.legacy, unavailable, fixture_on_d)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(!fixture.marker().exists());
    }

    #[test]
    fn broad_custom_roots_are_rejected_without_copying_profile_contents() {
        let fixture = Fixture::new();
        write(
            fixture.legacy.home.join("unrelated-private-file"),
            b"private",
        );
        fixture.store(serde_json::json!({"app_config_dir_override": "~"}));
        assert!(fixture.run().is_err());
        assert!(!fixture.targets.app.join("unrelated-private-file").exists());
        assert!(!fixture.marker().exists());
    }

    #[test]
    fn interrupted_copy_keeps_both_sides_and_requires_manual_review() {
        let fixture = Fixture::new();
        let old = fixture.legacy.home.join(".cc-switch");
        write(old.join("one"), b"same");
        write(old.join("two"), b"source");
        write(fixture.targets.app.join("one"), b"same");
        write(fixture.targets.app.join("two"), b"different");
        assert_eq!(
            fixture.run().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        fixture.assert_pending();
        assert_eq!(
            fs::read(fixture.targets.app.join("two")).unwrap(),
            b"different"
        );
        write(fixture.targets.app.join("two"), b"source");
        assert_eq!(
            fixture.run().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        fixture.assert_pending();
        assert_eq!(fs::read(old.join("two")).unwrap(), b"source");
        assert_eq!(
            fs::read(fixture.targets.app.join("two")).unwrap(),
            b"source"
        );
    }

    #[test]
    fn completed_migration_does_not_replay_old_data_and_rejects_invalid_marker() {
        let fixture = Fixture::new();
        write(fixture.legacy.home.join(".cc-switch/cc-switch.db"), b"old");
        fixture.run().unwrap();
        write(fixture.targets.app.join("cc-switch.db"), b"current");
        fixture.run().unwrap();
        assert_eq!(
            fs::read(fixture.targets.app.join("cc-switch.db")).unwrap(),
            b"current"
        );
        fs::remove_file(fixture.targets.tauri.join(STORE_FILE)).unwrap();
        assert!(fixture.run().is_err());
        write(fixture.targets.tauri.join(STORE_FILE), b"{}");
        write(fixture.marker(), br#"{"version":1,"targets":{}}"#);
        assert!(fixture.run().is_err());
    }

    #[test]
    fn overlapping_trees_fail_and_same_directory_is_safe() {
        let root = tempfile::tempdir().unwrap();
        write(root.path().join("file"), b"keep");
        let nested = root.path().join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(copy_tree(root.path(), &nested, None, false).is_err());
        assert!(copy_tree(&nested, root.path(), None, false).is_err());
        copy_tree(root.path(), root.path(), None, false).unwrap();
        assert_eq!(fs::read(root.path().join("file")).unwrap(), b"keep");
    }

    #[test]
    fn home_fallback_only_selects_an_existing_legacy_database() {
        let fixture = Fixture::new();
        let default = fixture.legacy.home.join(".cc-switch");
        let alternate = fixture.legacy.home.join("alternate");
        assert_eq!(
            legacy_default_app(&default, Some(&alternate)).unwrap(),
            default
        );
        write(alternate.join(".cc-switch/cc-switch.db"), b"legacy");
        assert_eq!(
            legacy_default_app(&default, Some(&alternate)).unwrap(),
            alternate.join(".cc-switch")
        );
        write(default.join("cc-switch.db"), b"default");
        assert_eq!(
            legacy_default_app(&default, Some(&alternate)).unwrap(),
            default
        );
    }

    #[test]
    fn legacy_home_selection_propagates_path_errors_instead_of_falling_back() {
        let fixture = Fixture::new();
        let alternate = fixture.legacy.home.join("alternate");
        write(alternate.join(".cc-switch/cc-switch.db"), b"alternate");
        let invalid_path = fixture.legacy.home.join("invalid\0directory");
        assert!(legacy_default_app(&invalid_path, Some(&alternate)).is_err());
        assert_eq!(
            fs::read(alternate.join(".cc-switch/cc-switch.db")).unwrap(),
            b"alternate"
        );
    }

    #[test]
    fn final_verification_detects_changed_or_added_source_entries() {
        for add_entry in [false, true] {
            let fixture = Fixture::new();
            let source = fixture.legacy.home.join(".cc-switch");
            write(source.join("earlier"), b"original");
            prepare_fixture_dir(&fixture.targets.app).unwrap();
            let before = snapshot_source(&source, None, true).unwrap();
            copy_tree(&source, &fixture.targets.app, None, true).unwrap();
            if add_entry {
                write(source.join("new-entry"), b"new");
            } else {
                write(source.join("earlier"), b"changed length");
            }
            let error =
                verify_source(&source, &fixture.targets.app, None, true, &before).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert_eq!(
                fs::read(fixture.targets.app.join("earlier")).unwrap(),
                b"original"
            );
            assert!(!fixture.marker().exists());
        }
    }

    #[test]
    fn final_verification_compares_bytes_even_when_source_metadata_is_preserved() {
        let fixture = Fixture::new();
        let source = fixture.legacy.home.join(".cc-switch");
        let file = source.join("same-length");
        write(&file, b"old");
        prepare_fixture_dir(&fixture.targets.app).unwrap();
        let before = snapshot_source(&source, None, true).unwrap();
        let modified = fs::metadata(&file).unwrap().modified().unwrap();
        copy_tree(&source, &fixture.targets.app, None, true).unwrap();
        write(&file, b"new");
        fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(snapshot_source(&source, None, true).unwrap(), before);
        assert_eq!(
            verify_source(&source, &fixture.targets.app, None, true, &before)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(!fixture.marker().exists());
    }

    fn create_shared_link(root: &Path, relative: &Path, target: &Path) -> PathBuf {
        let link = root.join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        crate::agy_config::create_dir_link(target, &link).unwrap();
        link
    }

    #[cfg(windows)]
    #[test]
    fn d_drive_root_overrides_fail_before_in_place_or_completed_tree_validation() {
        let fixture = Fixture::new();
        fixture.store(serde_json::json!({"app_config_dir_override": "D:\\"}));
        let error = migrate_from(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            is_d_path,
        )
        .unwrap_err();
        assert!(error.to_string().contains("drive root"));
        assert!(!fixture.marker().exists());

        fixture.store(serde_json::json!({}));
        fixture.run().unwrap();
        write(
            fixture.targets.tauri.join(STORE_FILE),
            br#"{"app_config_dir_override":"D:\\"}"#,
        );
        let error = migrate_from(
            &fixture.targets,
            &fixture.legacy,
            prepare_fixture_dir,
            is_d_path,
        )
        .unwrap_err();
        assert!(error.to_string().contains("drive root"));
    }

    #[test]
    fn managed_shared_links_copy_and_restart_without_reading_external_contents() {
        for relative in [
            Path::new("claude-accounts/0123456789abcdef0123456789abcdef/config/skills"),
            Path::new("agy/provider-key.1/config"),
        ] {
            let fixture = Fixture::new();
            let external = fixture.legacy.home.join("external-tool/shared");
            let payload = external.join("external-private-data");
            write(&payload, [1, 0, 255, 42]);
            let source = fixture.legacy.home.join(".cc-switch");
            let source_link = create_shared_link(&source, relative, &external);
            // An identical destination left by an interrupted copy must be reusable.
            let target_link = create_shared_link(&fixture.targets.app, relative, &external);
            #[cfg(windows)]
            let locked = {
                use std::os::windows::fs::OpenOptionsExt;
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(&payload)
                    .unwrap()
            };

            fixture.run().unwrap();
            assert_eq!(
                absolute_link_target(&target_link).unwrap(),
                absolute_link_target(&source_link).unwrap()
            );
            assert!(!fixture.targets.device.join(relative).exists());
            copy_tree(&source, &fixture.targets.app, None, true).unwrap();
            fixture.run().unwrap();

            #[cfg(windows)]
            drop(locked);
            assert_eq!(fs::read(&payload).unwrap(), [1, 0, 255, 42]);
            assert!(is_directory_link(
                &fs::symlink_metadata(source_link).unwrap()
            ));
            assert!(fixture.marker().is_file());
        }
    }

    #[test]
    fn in_place_d_custom_app_keeps_managed_shared_links() {
        let fixture = Fixture::new();
        let custom = fixture.legacy.home.join("custom-d");
        let external = fixture.legacy.home.join("external-tool/shared");
        write(external.join("keep"), b"external");
        let link = create_shared_link(
            &custom,
            Path::new("claude-accounts/0123456789abcdef0123456789abcdef/config/plugins"),
            &external,
        );
        let original_target = absolute_link_target(&link).unwrap();
        fixture.store(serde_json::json!({"app_config_dir_override": "~/custom-d"}));
        fixture.run().unwrap();
        fixture.run().unwrap();
        assert_eq!(absolute_link_target(&link).unwrap(), original_target);
        assert_eq!(fs::read(external.join("keep")).unwrap(), b"external");
    }

    #[test]
    fn conflicting_managed_link_target_is_preserved_and_stops_completion() {
        let fixture = Fixture::new();
        let relative = Path::new("agy/provider-key/config");
        let source = fixture.legacy.home.join(".cc-switch");
        let external = fixture.legacy.home.join("external-tool/shared");
        let other = fixture.legacy.home.join("external-tool/other");
        write(external.join("keep"), b"source target");
        write(other.join("keep"), b"existing target");
        create_shared_link(&source, relative, &external);
        let existing = create_shared_link(&fixture.targets.app, relative, &other);
        let before = absolute_link_target(&existing).unwrap();

        assert_eq!(
            fixture.run().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(absolute_link_target(&existing).unwrap(), before);
        assert_eq!(fs::read(external.join("keep")).unwrap(), b"source target");
        assert_eq!(fs::read(other.join("keep")).unwrap(), b"existing target");
        fixture.assert_pending();
    }

    #[test]
    fn only_recognized_managed_link_layouts_are_permitted() {
        for relative in [
            Path::new("claude-accounts/short/config/skills"),
            Path::new("claude-accounts/0123456789abcdef0123456789abcdef/config/unrecognized"),
            Path::new("agy/unsafe profile/config"),
            Path::new("unmanaged/linked"),
        ] {
            let fixture = Fixture::new();
            let external = fixture.legacy.home.join("external-tool/shared");
            write(external.join("keep"), b"external");
            create_shared_link(&fixture.legacy.home.join(".cc-switch"), relative, &external);
            assert!(fixture.run().is_err());
            assert_eq!(fs::read(external.join("keep")).unwrap(), b"external");
            fixture.assert_pending();
        }
    }

    #[cfg(unix)]
    #[test]
    fn source_and_destination_links_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        write(source.join("file"), b"source");
        fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(source.join("file"), source.join("linked")).unwrap();
        assert!(copy_tree(&source, &target, None, false).is_err());
        fs::remove_file(source.join("linked")).unwrap();
        std::os::unix::fs::symlink(source.join("file"), target.join("file")).unwrap();
        assert!(copy_tree(&source, &target, None, false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn completed_and_in_place_targets_reject_linked_descendants() {
        let fixture = Fixture::new();
        fixture.run().unwrap();
        let outside = fixture.legacy.home.join("outside");
        write(&outside, b"outside");
        std::os::unix::fs::symlink(&outside, fixture.targets.app.join("cc-switch.db")).unwrap();
        assert!(fixture.run().is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"outside");

        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&outside, root.path().join("linked")).unwrap();
        assert!(copy_tree(root.path(), root.path(), None, false).is_err());
    }
}
