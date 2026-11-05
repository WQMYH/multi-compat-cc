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
}

struct Legacy {
    home: PathBuf,
    home_env: Option<PathBuf>,
    tauri: PathBuf,
    local_data: PathBuf,
    webview: PathBuf,
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
        local_data: dirs::data_local_dir()
            .ok_or_else(|| invalid("Cannot resolve the legacy local-data directory"))?,
        webview: crate::data_paths::legacy_webview_dir(),
    };
    migrate_from(
        &targets,
        &legacy,
        crate::data_paths::validate_data_dir,
        is_d_path,
    )
}

fn migrate_from(
    targets: &Targets,
    legacy: &Legacy,
    validate_dir: fn(&Path) -> io::Result<()>,
    on_d: fn(&Path) -> bool,
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
    if let Some(metadata) = metadata_if_present(&marker)? {
        require_plain_file(&marker, &metadata)?;
        let completion: Completion = serde_json::from_slice(&fs::read(&marker)?)
            .map_err(|error| invalid(format!("Invalid migration marker: {error}")))?;
        if completion.version != 1 || completion.targets != *targets {
            return Err(invalid(
                "Migration marker targets do not match this application",
            ));
        }
        validate_active_app(targets, legacy, validate_dir, on_d)?;
        return Ok(());
    }

    let (mut store, original_store) = read_legacy_store(&legacy.tauri)?;
    let override_path = match store.get("app_config_dir_override") {
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            let path = resolve_override(raw.trim(), &legacy.home);
            reject_broad_source(&path, legacy)?;
            Some(path)
        }
        Some(Value::String(_)) | None => None,
        Some(_) => return Err(invalid("Legacy app_config_dir_override must be a string")),
    };

    let legacy_device = legacy.home.join(".cc-switch");
    let source_app = match &override_path {
        Some(path) => path.clone(),
        None => legacy_default_app(&legacy_device, legacy.home_env.as_deref())?,
    };
    if override_path.is_some() {
        reject_target_overlap(&source_app, targets)?;
    }
    let app_target = if override_path.is_some() && on_d(&fs::canonicalize(&source_app)?) {
        validate_dir(&source_app)?;
        validate_owned_tree(&source_app, true)?;
        source_app.clone()
    } else {
        targets.app.clone()
    };
    if metadata_if_present(&legacy.webview)?.is_some() {
        reject_broad_source(&legacy.webview, legacy)?;
    }
    let store_bytes = if override_path.is_some() && app_target != source_app {
        store.insert(
            "app_config_dir_override".into(),
            Value::String(path_string(&app_target)?),
        );
        serde_json::to_vec_pretty(&store).map_err(|error| invalid(error.to_string()))?
    } else {
        original_store.unwrap_or_else(|| b"{}".to_vec())
    };

    // The legacy default mixed application and device files in one directory.
    // Preserve its complete layout in both new namespaces rather than guessing
    // which unknown historical files can be discarded.
    let trees: [(&Path, &Path, Option<&str>, bool); 4] = [
        (&source_app, &app_target, None, true),
        (&legacy_device, &targets.device, None, true),
        (&legacy.tauri, &targets.tauri, Some(STORE_FILE), false),
        (&legacy.webview, &targets.webview, None, false),
    ];
    let snapshots = trees
        .iter()
        .map(|(source, _, skip, allow)| snapshot_source(source, *skip, *allow))
        .collect::<io::Result<Vec<_>>>()?;
    for (source, target, skip, allow) in trees {
        copy_tree(source, target, skip, allow)?;
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

    let completion = serde_json::to_vec_pretty(&Completion {
        version: 1,
        targets: targets.clone(),
    })
    .map_err(|error| invalid(error.to_string()))?;
    publish_bytes(&marker, &completion)
}

fn read_legacy_store(directory: &Path) -> io::Result<(Map<String, Value>, Option<Vec<u8>>)> {
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

fn validate_active_app(
    targets: &Targets,
    legacy: &Legacy,
    validate_dir: fn(&Path) -> io::Result<()>,
    on_d: fn(&Path) -> bool,
) -> io::Result<()> {
    let (store, bytes) = read_legacy_store(&targets.tauri)?;
    if bytes.is_none() {
        return Err(invalid("Completed migration is missing app_paths.json"));
    }
    match store.get("app_config_dir_override") {
        None => Ok(()),
        Some(Value::String(raw)) if raw.trim().is_empty() => Ok(()),
        Some(Value::String(raw)) => {
            let path = resolve_override(raw.trim(), &legacy.home);
            if !on_d(&path) {
                return Err(invalid("The active application directory must be on D"));
            }
            reject_broad_source(&path, legacy)?;
            reject_target_overlap(&path, targets)?;
            validate_dir(&path)?;
            validate_owned_tree(&path, true)
        }
        Some(_) => Err(invalid("app_config_dir_override must be a string")),
    }
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

fn snapshot_source(source: &Path, skip: Option<&str>, allow: bool) -> io::Result<SourceSnapshot> {
    if metadata_if_present(source)?.is_none() {
        return Ok(None);
    }
    require_source_directory(source)?;
    let mut entries = BTreeMap::new();
    snapshot_contents(source, source, skip, allow, &mut entries)?;
    Ok(Some(entries))
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
        if skip.is_some_and(|name| entry.file_name() == name) {
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
        if skip_root_file.is_some_and(|name| entry.file_name() == name) {
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

fn path_string(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("Migration target cannot be represented in app_paths.json"))
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

    fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) {
        let path = path.as_ref();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
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
            fs::read(fixture.targets.device.join("unknown/nested.bin")).unwrap(),
            [255, 0, 7]
        );
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
    fn rewrites_non_d_override_and_preserves_other_store_values() {
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
        assert_eq!(
            store["app_config_dir_override"],
            fixture.targets.app.to_str().unwrap()
        );
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
    fn retains_valid_d_custom_directory_and_store_bytes() {
        let fixture = Fixture::new();
        let custom = fixture.legacy.home.join("custom-d");
        write(custom.join("cc-switch.db"), b"existing D db");
        fixture.store(serde_json::json!({"app_config_dir_override": "~/custom-d", "other": 42}));
        let original = fs::read(fixture.legacy.tauri.join(STORE_FILE)).unwrap();

        fixture.run().unwrap();
        assert_eq!(
            fs::read(fixture.targets.tauri.join(STORE_FILE)).unwrap(),
            original
        );
        assert_eq!(
            fs::read(custom.join("cc-switch.db")).unwrap(),
            b"existing D db"
        );
        assert!(!fixture.targets.app.join("cc-switch.db").exists());
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
    fn interrupted_copy_retries_identical_files_and_rejects_conflicts() {
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
        assert!(!fixture.marker().exists());
        assert_eq!(
            fs::read(fixture.targets.app.join("two")).unwrap(),
            b"different"
        );
        write(fixture.targets.app.join("two"), b"source");
        fixture.run().unwrap();
        assert!(fixture.marker().is_file());
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
            assert_eq!(
                absolute_link_target(&fixture.targets.device.join(relative)).unwrap(),
                absolute_link_target(&source_link).unwrap()
            );
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
        assert!(!fixture.marker().exists());
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
            assert!(!fixture.marker().exists());
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
