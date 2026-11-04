//! Read-only DeepSeek Harness session browser; native Web UI owns resume/delete.
use super::{blocks, utils};
use crate::session_manager::{
    model::{ContentRef, MessageMeta, SessionBlock, ToolStatus},
    SessionMessage, SessionMeta,
};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

// Keep the main branch's resource guards for both session generations.
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_EVENTS: usize = 500_000;

pub fn session_roots() -> Vec<PathBuf> {
    legacy::session_roots()
}

fn generation(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("session.v")?
        .strip_suffix(".jsonl.zstd")
        .or_else(|| name.strip_prefix("session.v")?.strip_suffix(".jsonl"))?
        .parse()
        .ok()
}

/// Decompress complete independent frames; a partially appended final frame waits for the next scan.
pub(crate) fn read_session(path: &Path) -> Result<Vec<u8>, String> {
    let (_, source) =
        crate::session_manager::content::resolve_under_roots("dsh", path, &session_roots())?;
    if generation(&source).is_none()
        && source.file_name().and_then(|name| name.to_str()) != Some("session.jsonl.zstd")
    {
        return Err("Invalid DeepSeek Harness session source name".into());
    }
    read_session_data_with_limit(&source, MAX_DECOMPRESSED_BYTES)
}

fn read_session_data_with_limit(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_SESSION_BYTES {
        return Err("DeepSeek Harness session is too large".into());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if path.extension().is_none_or(|ext| ext != "zstd") {
        if bytes.len() as u64 > limit {
            return Err("DeepSeek Harness session exceeds the decompressed safety limit".into());
        }
        return Ok(bytes);
    }
    let mut output = vec![];
    let mut offset = 0;
    while offset < bytes.len() {
        let frame = &bytes[offset..];
        let size = match zstd::zstd_safe::find_frame_compressed_size(frame) {
            Ok(size) => size,
            Err(error)
                if (frame.len() < 4 && [0x28, 0xb5, 0x2f, 0xfd].starts_with(frame))
                    || (frame.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
                        && zstd::zstd_safe::get_error_name(error) == "Src size is incorrect"
                        && !frame
                            .windows(4)
                            .skip(1)
                            .any(|bytes| bytes == [0x28, 0xb5, 0x2f, 0xfd])) =>
            {
                break;
            }
            Err(_) => return Err("Invalid DeepSeek Harness zstd session".into()),
        };
        let decoder =
            zstd::stream::read::Decoder::new(&frame[..size]).map_err(|error| error.to_string())?;
        decoder
            .take(limit.saturating_add(1).saturating_sub(output.len() as u64))
            .read_to_end(&mut output)
            .map_err(|error| error.to_string())?;
        if output.len() as u64 > limit {
            return Err("DeepSeek Harness session is too large".into());
        }
        offset += size;
    }
    Ok(output)
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut result = vec![];
    // Native layout is <project>/<session>/session.vN.jsonl[.zstd]. Do not follow directory links.
    for project in fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .take(100_000)
    {
        if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        for session in fs::read_dir(project.path())
            .into_iter()
            .flatten()
            .flatten()
            .take(100_000)
        {
            if !session.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let selected = fs::read_dir(session.path())
                .into_iter()
                .flatten()
                .flatten()
                .filter(|file| file.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|file| file.path())
                .filter_map(|path| generation(&path).map(|version| (version, path)))
                .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            if let Some((_, path)) = selected {
                result.push(path);
            }
        }
    }
    result
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let Some(root) = session_roots().into_iter().next() else {
        return Vec::new();
    };
    let current = scan(&root);
    let mut sessions = legacy::scan_sessions();
    // A successfully parsed versioned file supersedes the legacy file in the
    // same native session directory. A broken new file cannot hide the old one.
    sessions.retain(|old| {
        !current.iter().any(|new| {
            old.source_path
                .as_deref()
                .and_then(|path| Path::new(path).parent())
                == new
                    .source_path
                    .as_deref()
                    .and_then(|path| Path::new(path).parent())
        })
    });
    sessions.extend(current);
    sessions
}

fn scan(root: &Path) -> Vec<SessionMeta> {
    files(root)
        .into_iter()
        .filter_map(|path| {
            let path = path.canonicalize().ok()?;
            let data = read_session(&path).ok()?;
            let mut lines = data.split(|byte| *byte == b'\n');
            let header: Value = serde_json::from_slice(lines.next()?).ok()?;
            if !matches!(header["type"].as_str(), Some("session" | "header")) {
                return None;
            }
            let id = header["id"]
                .as_str()
                .filter(|id| !id.is_empty())?
                .to_string();
            let messages = parse_messages(&data).ok()?;
            let title = messages
                .iter()
                .find(|message| message.role == "user" && !message.injected)
                .map(|message| utils::truncate_summary(&message.content, utils::TITLE_MAX_CHARS));
            Some(SessionMeta {
                provider_id: "dsh".into(),
                session_id: id,
                title,
                summary: None,
                project_dir: header["cwd"].as_str().map(str::to_string),
                created_at: header["createdAt"].as_i64(),
                last_active_at: messages.iter().filter_map(|message| message.ts).max(),
                source_path: Some(path.to_string_lossy().into_owned()),
                resume_command: None,
            })
        })
        .collect()
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    if path.file_name().and_then(|name| name.to_str()) == Some("session.jsonl.zstd") {
        return legacy::load_messages(path);
    }
    parse_messages(&read_session(path)?)
}

/// Legacy files retain their existing validated deletion path. Versioned
/// sessions are read-only here; their native Web UI owns resume and deletion.
pub fn delete_session(root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
    if generation(path).is_some() {
        return Err(
            "Versioned DeepSeek Harness sessions must be deleted in the native Web UI".into(),
        );
    }
    legacy::delete_session(root, path, session_id)
}
fn parse_messages(data: &[u8]) -> Result<Vec<SessionMessage>, String> {
    let mut messages = vec![];
    let mut seen = HashSet::new();
    let mut offset = 0;
    for (event, line) in data.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let start = offset;
        offset += line.len();
        // An incomplete JSONL tail is still being written.
        if !line.ends_with(b"\n") {
            break;
        }
        if event >= MAX_EVENTS {
            return Err("DeepSeek Harness session exceeds the event safety limit".into());
        }
        let record: Value =
            serde_json::from_slice(line).map_err(|_| "Invalid DeepSeek Harness session JSONL")?;
        // Replacement copies only alter the model-visible surface. The human
        // transcript retains append-origin messages and full tool outputs.
        if record["surfaceOp"]["op"] == "replace" {
            continue;
        }
        let kind = record["type"].as_str().unwrap_or_default();
        let (raw, pointer, role) = match kind {
            "user/message" => (&record["data"], "/data", "user"),
            "assistant/message" => (&record["data"]["message"], "/data/message", "assistant"),
            "tool/result" => (&record["data"]["message"], "/data/message", "tool"),
            _ => continue,
        };
        if let Some(id) = raw["id"].as_str() {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }
        let mut message_blocks = vec![];
        let reference = |pointer: String| {
            Some(ContentRef::Jsonl {
                offset: start as u64,
                len: line.len() as u32,
                pointer,
            })
        };
        if role == "tool" {
            let text = utils::extract_text(&raw["content"]);
            message_blocks.push(blocks::tool_result_block(
                raw["toolCallId"].as_str().unwrap_or_default(),
                if raw["isError"] == true {
                    ToolStatus::Error
                } else {
                    ToolStatus::Success
                },
                &text,
                || reference(format!("{pointer}/content")),
            ));
        } else {
            for (index, part) in raw["content"].as_array().into_iter().flatten().enumerate() {
                match part["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = part["text"].as_str() {
                            message_blocks.push(SessionBlock::text(text));
                        }
                    }
                    Some("thinking" | "reasoning") => {
                        let field = if part.get("text").is_some() {
                            "text"
                        } else {
                            "thinking"
                        };
                        message_blocks.push(blocks::thinking_block(
                            part[field].as_str().unwrap_or_default(),
                            None,
                            None,
                            || reference(format!("{pointer}/content/{index}/{field}")),
                        ));
                    }
                    Some("tool-call") => {
                        let arguments = blocks::parse_arguments(&part["arguments"]);
                        message_blocks.push(blocks::tool_call_block(
                            blocks::ToolSource::Generic,
                            part["id"].as_str().unwrap_or_default(),
                            part["name"].as_str().unwrap_or_default(),
                            &arguments,
                            || reference(format!("{pointer}/content/{index}/arguments")),
                        ));
                    }
                    _ => {}
                }
            }
        }
        if message_blocks.is_empty() {
            continue;
        }
        let mut message =
            SessionMessage::from_blocks(role, record["time"].as_i64(), message_blocks);
        message.injected = role == "user"
            && raw["source"]["kind"]
                .as_str()
                .is_some_and(|kind| kind != "user");
        let usage = &record["data"]["usage"];
        if role == "assistant" {
            message.meta = Some(MessageMeta {
                model: raw["source"]["model"].as_str().map(str::to_string),
                input_tokens: usage["inputTokens"].as_u64(),
                output_tokens: usage["outputTokens"].as_u64(),
                cache_read_tokens: usage["cacheReadTokens"].as_u64(),
                cache_write_tokens: usage["cacheWriteTokens"].as_u64(),
                ..Default::default()
            });
        }
        messages.push(message);
    }
    blocks::assign_turn_ids(&mut messages);
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct DshHome(Option<std::ffi::OsString>);

    impl DshHome {
        fn new(path: &Path) -> Self {
            let previous = std::env::var_os("DSH_HOME");
            std::env::set_var("DSH_HOME", path);
            Self(previous)
        }
    }

    impl Drop for DshHome {
        fn drop(&mut self) {
            match self.0.take() {
                Some(previous) => std::env::set_var("DSH_HOME", previous),
                None => std::env::remove_var("DSH_HOME"),
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn versioned_sessions_supersede_legacy_without_removing_sources() {
        let temp = tempfile::tempdir().unwrap();
        let _home = DshHome::new(temp.path());
        let dir = temp.path().join("sessions/project/test");
        fs::create_dir_all(&dir).unwrap();
        let legacy = dir.join("session.jsonl.zstd");
        fs::write(&legacy, zstd::encode_all(fixture().as_slice(), 1).unwrap()).unwrap();
        let old = scan_sessions();
        assert_eq!(old.len(), 1);
        assert_eq!(load_messages(&legacy).unwrap().len(), 2);
        let versioned = dir.join("session.v4.jsonl.zstd");
        fs::write(
            &versioned,
            zstd::encode_all(fixture().as_slice(), 1).unwrap(),
        )
        .unwrap();
        fs::write(dir.join("session.v3.jsonl"), fixture()).unwrap();
        let current = scan_sessions();
        assert_eq!(current.len(), 1);
        assert_eq!(
            current[0].source_path.as_deref(),
            versioned.canonicalize().unwrap().to_str()
        );
        assert_eq!(load_messages(&versioned).unwrap().len(), 3);
        assert!(delete_session(&temp.path().join("sessions"), &versioned, "test").is_err());
        assert!(versioned.exists() && legacy.exists());
        fs::write(&versioned, b"invalid frame").unwrap();
        let fallback = scan_sessions();
        assert_eq!(fallback.len(), 1);
        assert_eq!(
            fallback[0].source_path.as_deref(),
            legacy.canonicalize().unwrap().to_str()
        );
    }

    #[test]
    #[serial_test::serial]
    fn versioned_sources_outside_the_native_root_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let _home = DshHome::new(home.path());
        fs::create_dir_all(home.path().join("sessions")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let path = outside.path().join("session.v4.jsonl");
        fs::write(&path, fixture()).unwrap();
        assert!(load_messages(&path).is_err());
        assert!(read_session(&path).is_err());
    }

    #[test]
    fn versioned_limits_and_corrupt_middle_frames_never_return_partial_success() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.v4.jsonl.zstd");
        let bytes = fixture();
        fs::write(&path, zstd::encode_all(bytes.as_slice(), 1).unwrap()).unwrap();
        assert!(read_session_data_with_limit(&path, 32).is_err());
        let mut corrupt = zstd::encode_all(bytes.as_slice(), 1).unwrap();
        corrupt.extend([0x28, 0xb5, 0x2f, 0xfd, 0xff]);
        corrupt.extend(zstd::encode_all(bytes.as_slice(), 1).unwrap());
        fs::write(&path, corrupt).unwrap();
        assert!(read_session_data_with_limit(&path, MAX_DECOMPRESSED_BYTES).is_err());
        let excess = b"{}\n".repeat(MAX_EVENTS + 1);
        assert!(parse_messages(&excess).is_err());
    }

    fn fixture() -> Vec<u8> {
        [json!({"type":"session","version":4,"id":"test","cwd":"/project","createdAt":1000}),
         json!({"type":"user/message","seq":1,"surfaceOp":"append","time":1001,"data":{"id":"u","role":"user","source":{"kind":"user"},"content":[{"type":"text","text":"Fix this"}]}}),
         json!({"type":"assistant/message","seq":2,"surfaceOp":"append","time":2000,"data":{"message":{"id":"a","source":{"model":"route:model"},"content":[{"type":"text","text":"Fixed"},{"type":"tool-call","id":"call","name":"read","arguments":"{\"path\":\"test.rs\"}"}]},"usage":{"inputTokens":10,"outputTokens":20,"cacheReadTokens":30}}}),
         json!({"type":"tool/result","seq":3,"surfaceOp":"append","time":2001,"data":{"message":{"id":"result","role":"tool","toolCallId":"call","isError":false,"content":[{"type":"text","text":"contents"}]}}})]
            .iter().map(|line| format!("{line}\n")).collect::<String>().into_bytes()
    }

    #[test]
    #[serial_test::serial]
    fn browser_reads_concatenated_frames_and_ignores_incomplete_tail() {
        let temp = tempfile::tempdir().unwrap();
        let _home = DshHome::new(temp.path());
        let root = temp.path().join("sessions/project/test");
        fs::create_dir_all(&root).unwrap();
        let bytes = fixture();
        let boundary = bytes.iter().position(|byte| *byte == b'\n').unwrap() + 1;
        let mut compressed = zstd::encode_all(&bytes[..boundary], 1).unwrap();
        compressed.extend(zstd::encode_all(&bytes[boundary..], 1).unwrap());
        let tail = zstd::encode_all(b"{\"pending\":true}\n".as_slice(), 1).unwrap();
        compressed.extend(&tail[..tail.len() / 2]);
        let path = root.join("session.v4.jsonl.zstd");
        fs::write(&path, compressed).unwrap();
        assert_eq!(read_session(&path).unwrap(), bytes);
        let sessions = scan(&temp.path().join("sessions"));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("Fix this"));
        let messages = load_messages(&path).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[1].meta.as_ref().unwrap().cache_read_tokens,
            Some(30)
        );
        assert!(matches!(
            messages[1].blocks[1],
            SessionBlock::ToolCall { .. }
        ));
        assert!(matches!(
            messages[2].blocks[0],
            SessionBlock::ToolResult { ref call_id, status: ToolStatus::Success, ref preview, .. }
                if call_id == "call" && preview == "contents"
        ));
    }

    #[test]
    fn duplicate_messages_and_model_only_summaries_are_not_shown() {
        let mut bytes = fixture();
        let original = parse_messages(&bytes).unwrap();
        bytes.extend(format!("{}\n",json!({"type":"assistant/message","seq":4,"data":{"message":{"id":"a","content":[{"type":"text","text":"Duplicate"}]}}})).as_bytes());
        bytes.extend(format!("{}\n",json!({"type":"user/message","seq":5,"surfaceOp":{"op":"replace","startSeq":1,"endSeq":3},"data":{"id":"summary","content":[{"type":"text","text":"Summary"}]}})).as_bytes());
        let parsed = parse_messages(&bytes).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(
            parsed
                .iter()
                .map(|message| &message.content)
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|message| &message.content)
                .collect::<Vec<_>>()
        );
        bytes.extend(b"{partial");
        assert_eq!(parse_messages(&bytes).unwrap().len(), 3);
    }

    #[test]
    fn native_tool_results_preserve_errors_empty_results_and_full_content() {
        let output = "failed output\n".repeat(20);
        for content in [json!([]), json!([{"type":"text","text":output}])] {
            let mut bytes = fixture();
            bytes.extend(format!("{}\n", json!({"type":"tool/result","seq":4,"data":{"message":{"id":"failure","role":"tool","toolCallId":"failed-call","isError":true,"content":content}}})).as_bytes());
            let messages = parse_messages(&bytes).unwrap();
            assert_eq!(messages.len(), 4);
            assert_eq!(messages[3].blocks.len(), 1);
            let SessionBlock::ToolResult {
                call_id,
                status: ToolStatus::Error,
                total_len,
                full,
                ..
            } = &messages[3].blocks[0]
            else {
                panic!("expected a native tool error result");
            };
            assert_eq!(call_id, "failed-call");
            let expected = utils::extract_text(&content);
            assert_eq!(*total_len as usize, expected.chars().count());
            if !expected.is_empty() {
                let ContentRef::Jsonl {
                    offset,
                    len,
                    pointer,
                } = full.as_ref().unwrap()
                else {
                    panic!("expected a JSONL content reference");
                };
                let record: Value = serde_json::from_slice(
                    &bytes[*offset as usize..*offset as usize + *len as usize],
                )
                .unwrap();
                assert_eq!(pointer, "/data/message/content");
                assert_eq!(
                    utils::extract_text(record.pointer(pointer).unwrap()),
                    expected
                );
            }
        }
    }

    #[test]
    fn surface_replacements_preserve_original_transcript_and_do_not_claim_message_ids() {
        let mut bytes = fixture();
        let original = parse_messages(&bytes).unwrap();
        let mut expected: Vec<_> = original
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        expected.extend(["Keep this", "After compaction"]);
        for record in [
            json!({"type":"user/message","seq":4,"surfaceOp":"append","data":{"id":"later","content":[{"type":"text","text":"Keep this"}]}}),
            json!({"type":"user/message","seq":5,"surfaceOp":{"op":"replace","startSeq":1,"endSeq":2},"data":{"id":"summary","content":[{"type":"text","text":"Summary"}]}}),
            json!({"type":"user/message","seq":6,"surfaceOp":{"op":"replace","startSeq":5,"endSeq":3},"data":{"id":"summary-2","content":[{"type":"text","text":"New summary"}]}}),
            json!({"type":"tool/result","seq":7,"surfaceOp":{"op":"replace","startSeq":3,"endSeq":3},"data":{"message":{"id":"result","toolCallId":"call","content":[{"type":"text","text":"Trimmed output"}]}}}),
            json!({"type":"assistant/message","seq":8,"surfaceOp":{"op":"replace","startSeq":2,"endSeq":2},"data":{"message":{"id":"after","content":[{"type":"text","text":"Model-only copy"}]}}}),
            json!({"type":"assistant/message","seq":9,"surfaceOp":"append","data":{"message":{"id":"after","content":[{"type":"text","text":"After compaction"}]}}}),
        ] {
            bytes.extend(format!("{record}\n").as_bytes());
        }
        let messages = parse_messages(&bytes).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(matches!(
            messages[2].blocks[0],
            SessionBlock::ToolResult { ref call_id, ref preview, .. }
                if call_id == "call" && preview == "contents"
        ));
    }
}

// Preserve strict legacy JSONL parsing, cache, path checks, and regression tests.
mod legacy {
    // Adapted from yang227/cc-switch PR7356 (MIT), with native-root validation.
    use std::fs::{self, File};
    use std::io::{self, BufRead, BufReader, Read};
    use std::path::{Path, PathBuf};
    use std::sync::LazyLock;

    use serde_json::Value;

    use crate::session_manager::model::SessionBlock;
    use crate::session_manager::{SessionMessage, SessionMeta};

    use super::super::utils::{
        extract_text, path_basename, truncate_summary, FileParseCache, TITLE_MAX_CHARS,
    };

    const PROVIDER_ID: &str = "dsh";
    const SESSION_FILE_NAME: &str = "session.jsonl.zstd";
    /// Compressed size guard; decompressed content is bounded by the event limit.
    const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
    /// Reject oversized decompressed content instead of returning a partial transcript.
    const MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_EVENTS: usize = 500_000;

    /// 会话页每次打开都会全量扫描；没变过的文件直接复用上次的解析结果。
    /// zstd 解压比 claude/codex 的明文解析更贵，缓存收益更大。
    static PARSE_CACHE: LazyLock<FileParseCache> = LazyLock::new(FileParseCache::new);

    pub fn session_roots() -> Vec<PathBuf> {
        vec![crate::dsh_config::data_dir().join("sessions")]
    }

    pub fn scan_sessions() -> Vec<SessionMeta> {
        let Some(root) = session_roots().into_iter().next() else {
            return Vec::new();
        };
        let Ok(entries) = fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut files = Vec::new();
        for project in entries.flatten() {
            let project_path = project.path();
            if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Ok(session_dirs) = fs::read_dir(&project_path) else {
                continue;
            };
            for session_dir in session_dirs.flatten() {
                if !session_dir.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let file = session_dir.path().join(SESSION_FILE_NAME);
                if file.is_file() {
                    if let Ok((_, source)) = crate::session_manager::content::resolve_under_roots(
                        PROVIDER_ID,
                        &file,
                        std::slice::from_ref(&root),
                    ) {
                        files.push(source);
                    }
                }
            }
        }
        PARSE_CACHE.scan(files, scan_session_file)
    }

    /// 适配 [`FileParseCache`] 的解析入口：解析失败按 io 错误上抛，
    /// 让缓存跳过且不记住「没有会话」，下轮扫描重试。
    fn scan_session_file(path: &Path) -> io::Result<Option<SessionMeta>> {
        match parse_session(path) {
            Ok(meta) => Ok(Some(meta)),
            Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }

    pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
        let events = read_events(path)?;
        let mut messages = Vec::new();
        for value in &events {
            match value.get("type").and_then(Value::as_str) {
                Some("user/message") => {
                    let content = value
                        .pointer("/data/content")
                        .map(extract_text)
                        .unwrap_or_default();
                    if !content.trim().is_empty() {
                        messages.push(SessionMessage::from_blocks(
                            "user",
                            event_time(value),
                            vec![SessionBlock::text(content)],
                        ));
                    }
                }
                Some("assistant/message") => {
                    let content = assistant_text(value);
                    if !content.trim().is_empty() {
                        messages.push(SessionMessage::from_blocks(
                            "assistant",
                            event_time(value),
                            vec![SessionBlock::text(content)],
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(messages)
    }

    pub fn delete_session(root: &Path, path: &Path, session_id: &str) -> Result<bool, String> {
        if session_id.contains(['/', '\\']) || session_id == "." || session_id == ".." {
            return Err("Invalid DSH session ID".to_string());
        }
        if path.file_name().and_then(|n| n.to_str()) != Some(SESSION_FILE_NAME) {
            return Err(format!(
                "DSH session source must be a {SESSION_FILE_NAME} file: {}",
                path.display()
            ));
        }
        let meta = parse_session(path)?;
        if meta.session_id != session_id {
            return Err(format!(
                "DSH session ID mismatch: expected {session_id}, found {}",
                meta.session_id
            ));
        }
        let root = root.canonicalize().map_err(|error| {
            format!(
                "Failed to resolve DSH sessions root {}: {error}",
                root.display()
            )
        })?;
        let source = path.canonicalize().map_err(|error| {
            format!("Failed to resolve DSH session {}: {error}", path.display())
        })?;
        let dir = source
            .parent()
            .ok_or_else(|| "DSH session has no parent directory".to_string())?;
        if !dir.starts_with(&root)
            || source
                .strip_prefix(&root)
                .map_err(|_| "Invalid DSH session path")?
                .components()
                .count()
                != 3
            || dir.file_name().and_then(|name| name.to_str()) != Some(session_id)
        {
            return Err("DSH session is outside the sessions root".to_string());
        }
        fs::remove_dir_all(dir)
            .map_err(|error| format!("Failed to delete DSH session {}: {error}", dir.display()))?;
        Ok(true)
    }

    fn event_time(value: &Value) -> Option<i64> {
        value
            .get("time")
            .or_else(|| value.get("time0"))
            .and_then(Value::as_i64)
    }

    fn assistant_text(value: &Value) -> String {
        value
            .pointer("/data/message/content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|item| item.get("text").and_then(Value::as_str).map(str::to_string))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn read_events(path: &Path) -> Result<Vec<Value>, String> {
        let (_, source) = crate::session_manager::content::resolve_under_roots(
            PROVIDER_ID,
            path,
            &session_roots(),
        )?;
        read_events_with_limit(&source, MAX_DECOMPRESSED_BYTES)
    }

    fn read_events_with_limit(path: &Path, limit: u64) -> Result<Vec<Value>, String> {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("Failed to stat DSH session {}: {error}", path.display()))?;
        if metadata.len() > MAX_SESSION_BYTES {
            return Err(format!(
                "DSH session exceeds the {MAX_SESSION_BYTES}-byte safety limit"
            ));
        }
        let file =
            File::open(path).map_err(|error| format!("Failed to open DSH session: {error}"))?;
        let decoder = zstd::stream::read::Decoder::new(file)
            .map_err(|error| format!("Failed to decompress DSH session: {error}"))?;
        let mut reader = BufReader::new(decoder.take(limit.saturating_add(1)));
        let mut events = Vec::new();
        let mut bytes = 0;
        let mut line = String::new();
        loop {
            line.clear();
            let count = reader
                .read_line(&mut line)
                .map_err(|error| format!("Failed to read DSH session: {error}"))?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > limit {
                return Err("DSH session exceeds the decompressed safety limit".into());
            }
            if line.trim().is_empty() {
                continue;
            }
            if events.len() >= MAX_EVENTS {
                return Err(format!(
                    "DSH session exceeds the {MAX_EVENTS}-event safety limit"
                ));
            }
            events.push(
                serde_json::from_str::<Value>(&line)
                    .map_err(|_| "DSH session contains an invalid JSON record")?,
            );
        }
        Ok(events)
    }

    fn parse_session(path: &Path) -> Result<SessionMeta, String> {
        let source = path.canonicalize().map_err(|error| {
            format!("Failed to resolve DSH session {}: {error}", path.display())
        })?;
        let events = read_events(&source)?;
        let header = events
            .iter()
            .find(|value| value.get("type").and_then(Value::as_str) == Some("session"))
            .ok_or_else(|| "DSH session has no header".to_string())?;
        let id = header
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "DSH session header has no id".to_string())?
            .to_string();
        let cwd = header
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string);
        let created_at = header.get("createdAt").and_then(Value::as_i64);

        let mut title = events
            .iter()
            .rev()
            .find(|value| value.get("type").and_then(Value::as_str) == Some("session/title"))
            .and_then(|value| value.pointer("/data/title"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut first_user: Option<String> = None;
        let mut last_text: Option<String> = None;
        let mut last_active_at = created_at;
        for value in &events {
            if let Some(time) = event_time(value) {
                last_active_at = Some(last_active_at.map_or(time, |current| current.max(time)));
            }
            match value.get("type").and_then(Value::as_str) {
                Some("user/message") => {
                    let content = value
                        .pointer("/data/content")
                        .map(extract_text)
                        .unwrap_or_default();
                    if first_user.is_none() && !content.trim().is_empty() {
                        first_user = Some(content.clone());
                    }
                    if !content.trim().is_empty() {
                        last_text = Some(content);
                    }
                }
                Some("assistant/message") => {
                    let text = assistant_text(value);
                    if !text.trim().is_empty() {
                        last_text = Some(text);
                    }
                }
                _ => {}
            }
        }
        if title.is_none() {
            title = first_user
                .as_deref()
                .map(|message| truncate_summary(message, TITLE_MAX_CHARS))
                .filter(|message| !message.is_empty())
                .or_else(|| cwd.as_deref().and_then(path_basename));
        }
        let summary = last_text
            .as_deref()
            .map(|message| truncate_summary(message, 160))
            .filter(|message| !message.is_empty());
        Ok(SessionMeta {
            provider_id: PROVIDER_ID.to_string(),
            session_id: id,
            title,
            summary,
            project_dir: cwd.filter(|value| !value.trim().is_empty()),
            created_at,
            last_active_at,
            source_path: source.to_str().map(str::to_string),
            resume_command: None,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serial_test::serial;

        fn write_session(home: &Path, project: &str, id: &str, events: &[Value]) -> PathBuf {
            let dir = home.join("sessions").join(project).join(id);
            fs::create_dir_all(&dir).unwrap();
            let payload = events
                .iter()
                .map(|event| serde_json::to_string(event).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let compressed = zstd::stream::encode_all(payload.as_bytes(), 3).unwrap();
            let file = dir.join(SESSION_FILE_NAME);
            fs::write(&file, compressed).unwrap();
            file
        }

        fn fixture_events() -> Vec<Value> {
            vec![
                serde_json::json!({"type":"session","version":0,"id":"session-abc","createdAt":1787104521333_i64,"cwd":"/Users/test/project"}),
                serde_json::json!({"type":"user/message","seq":8,"time":1787220655606_i64,"data":{"role":"user","content":[{"type":"text","text":"hello dsh"}]}}),
                serde_json::json!({"type":"session/title","seq":11,"time":1787220655607_i64,"data":{"title":"Greeting"}}),
                serde_json::json!({"type":"text-chunks","seq0":69,"time0":1787220662295_i64,"data":{"turn":1,"step":1,"index":1,"texts":["stream"," delta"]}}),
                serde_json::json!({"type":"assistant/message","seq":352,"time":1787220668317_i64,"data":{"turn":1,"step":1,"message":{"role":"assistant","content":[{"type":"reasoning","text":"thinking"},{"type":"text","text":"hi there"}]}}}),
            ]
        }

        fn with_temp_home(test: impl FnOnce(&Path)) {
            let directory = tempfile::tempdir().unwrap();
            let previous = std::env::var_os("DSH_HOME");
            std::env::set_var("DSH_HOME", directory.path());
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test(directory.path())));
            match previous {
                Some(value) => std::env::set_var("DSH_HOME", value),
                None => std::env::remove_var("DSH_HOME"),
            }
            result.unwrap();
        }

        #[test]
        #[serial]
        fn scan_reads_zstd_sessions_with_titles() {
            with_temp_home(|home| {
                let file = write_session(
                    home,
                    "--Users-test-project--",
                    "session-abc",
                    &fixture_events(),
                );
                let sessions = scan_sessions();
                assert_eq!(sessions.len(), 1);
                let meta = &sessions[0];
                assert_eq!(meta.provider_id, "dsh");
                assert_eq!(meta.session_id, "session-abc");
                assert_eq!(meta.title.as_deref(), Some("Greeting"));
                assert_eq!(meta.project_dir.as_deref(), Some("/Users/test/project"));
                assert_eq!(meta.created_at, Some(1787104521333));
                assert_eq!(meta.last_active_at, Some(1787220668317));
                assert_eq!(
                    meta.source_path.as_deref(),
                    file.canonicalize().unwrap().to_str()
                );
            });
        }

        #[test]
        #[serial]
        fn load_messages_skips_stream_deltas_and_reasoning() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                let messages = load_messages(&file).unwrap();
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].role, "user");
                assert_eq!(messages[0].content, "hello dsh");
                assert_eq!(messages[0].ts, Some(1787220655606));
                assert_eq!(messages[1].role, "assistant");
                assert_eq!(messages[1].content, "hi there");
                let serialized = serde_json::to_value(&messages[1]).unwrap();
                assert_eq!(serialized["blocks"][0]["type"], "text");
                assert_eq!(serialized["blocks"][0]["text"], "hi there");
                assert!(serialized.get("content").is_none());
                let transcript =
                    crate::session_manager::load_transcript(PROVIDER_ID, file.to_str().unwrap())
                        .unwrap();
                assert!(!transcript.cached);
            });
        }

        #[test]
        #[serial]
        fn corrupt_sessions_are_skipped_not_fatal() {
            with_temp_home(|home| {
                let dir = home.join("sessions").join("proj").join("session-bad");
                fs::create_dir_all(&dir).unwrap();
                fs::write(dir.join(SESSION_FILE_NAME), b"not zstd at all").unwrap();
                write_session(home, "proj", "session-abc", &fixture_events());
                let sessions = scan_sessions();
                assert_eq!(sessions.len(), 1);
            });
        }

        #[test]
        #[serial]
        fn rescan_reuses_the_cache_and_drops_deleted_sessions() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                let first = scan_sessions();
                assert_eq!(first.len(), 1);
                // 第二次扫描命中解析缓存，结果保持一致
                let second = scan_sessions();
                assert_eq!(second.len(), 1);
                assert_eq!(second[0].session_id, first[0].session_id);
                // 删除后按本轮文件列表重建缓存，不再返回已删会话
                delete_session(&home.join("sessions"), &file, "session-abc").unwrap();
                assert!(scan_sessions().is_empty());
            });
        }

        #[test]
        #[serial]
        fn delete_removes_the_session_directory() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                let root = home.join("sessions");
                let error = delete_session(&root, &file, "wrong-id").expect_err("id mismatch");
                assert!(error.contains("mismatch"), "unexpected error: {error}");
                assert!(file.exists());
                let deleted = delete_session(&root, &file, "session-abc").unwrap();
                assert!(deleted);
                assert!(!file.exists());
                assert!(!file.parent().unwrap().exists());
            });
        }

        #[test]
        #[serial]
        fn delete_accepts_an_unnormalized_root() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                let root = home.join("sessions").join("proj").join("..");
                let deleted = delete_session(&root, &file, "session-abc").unwrap();
                assert!(deleted);
                assert!(!file.parent().unwrap().exists());
            });
        }

        #[test]
        #[serial]
        fn outside_native_session_sources_are_refused() {
            with_temp_home(|home| {
                fs::create_dir_all(home.join("sessions")).unwrap();
                let file = home.join("outside.jsonl.zstd");
                fs::write(
                    &file,
                    zstd::stream::encode_all(b"{}".as_slice(), 1).unwrap(),
                )
                .unwrap();
                assert!(load_messages(&file)
                    .unwrap_err()
                    .contains("outside provider roots"));
                assert!(file.exists());
            });
        }

        #[test]
        #[serial]
        fn a_decompressed_limit_never_returns_a_partial_success() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                assert!(read_events_with_limit(&file, 32)
                    .unwrap_err()
                    .contains("decompressed safety limit"));
            });
        }

        #[test]
        #[serial]
        fn a_corrupt_json_record_never_returns_a_partial_transcript() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                let mut bytes =
                    zstd::stream::decode_all(fs::read(&file).unwrap().as_slice()).unwrap();
                bytes.extend_from_slice(b"\n{broken-json\n");
                fs::write(
                    &file,
                    zstd::stream::encode_all(bytes.as_slice(), 1).unwrap(),
                )
                .unwrap();
                assert!(load_messages(&file)
                    .unwrap_err()
                    .contains("invalid JSON record"));
                assert!(scan_sessions().is_empty());
            });
        }

        #[test]
        #[serial]
        fn deletion_requires_the_native_session_directory_shape() {
            with_temp_home(|home| {
                let file = write_session(home, "proj", "session-abc", &fixture_events());
                let misplaced = home.join("sessions").join(SESSION_FILE_NAME);
                fs::copy(&file, &misplaced).unwrap();
                assert!(delete_session(&home.join("sessions"), &misplaced, "session-abc").is_err());
                assert!(misplaced.exists());
                assert!(file.exists());
            });
        }
    }
}
