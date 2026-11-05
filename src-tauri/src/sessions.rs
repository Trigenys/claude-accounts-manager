//! Claude Code conversations of a profile (`<config_dir>/projects/<encoded cwd>/<id>.jsonl`).
//!
//! Transcripts are untrusted user files: only file metadata, a bounded head and a bounded tail are
//! read, parsed as JSON and never executed. Message bodies beyond the first prompt are ignored.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::error::{AppError, AppResult};

/// Bytes read from the start of a transcript (cwd and first prompt live in the first ~10 KiB).
const HEAD_BYTES: u64 = 64 * 1024;
/// Bytes read from the end (Claude Code re-appends titles as the conversation goes on).
const TAIL_BYTES: u64 = 64 * 1024;
/// Upper bound on transcripts whose metadata is inspected, whatever the profile holds.
const MAX_SCANNED: usize = 5_000;
pub const DEFAULT_LIMIT: usize = 50;
pub const MAX_LIMIT: usize = 200;
const TEXT_CHARS: usize = 120;

/// A Claude Code session id, validated as a canonical UUID: safe to put on a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(String);

impl SessionId {
    pub fn parse(s: &str) -> AppResult<Self> {
        let invalid = || AppError::new("invalid_session", "Identifiant de session invalide");
        // Only the hyphenated form Claude Code uses for file names (no braces, urn or simple form).
        if s.len() != 36 {
            return Err(invalid());
        }
        let u = uuid::Uuid::try_parse(s).map_err(|_| invalid())?;
        Ok(Self(u.hyphenated().to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    /// Title set with /rename, else the generated one.
    pub title: Option<String>,
    /// First prompt typed by the user, truncated.
    pub first_prompt: Option<String>,
    /// Project folder the session was started in.
    pub cwd: Option<String>,
    pub last_activity: Option<String>,
    pub size_bytes: u64,
    /// The project folder still exists, so `claude --resume` can run there.
    pub resumable: bool,
}

#[derive(Debug, Default, PartialEq)]
struct Meta {
    custom_title: Option<String>,
    ai_title: Option<String>,
    first_prompt: Option<String>,
    last_prompt: Option<String>,
    cwd: Option<String>,
}

struct Candidate {
    id: SessionId,
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
}

fn plain_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// Regular `<uuid>.jsonl` files of one project folder (links are skipped).
fn transcripts_in(dir: &Path, out: &mut Vec<Candidate>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if out.len() >= MAX_SCANNED {
            return;
        }
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| SessionId::parse(s).ok())
        else {
            continue;
        };
        let Ok(m) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !m.is_file() {
            continue;
        }
        out.push(Candidate {
            id,
            path,
            modified: m.modified().ok(),
            len: m.len(),
        });
    }
}

fn project_dirs(config_dir: &Path) -> Vec<PathBuf> {
    let root = config_dir.join("projects");
    if !plain_dir(&root) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| plain_dir(p))
        .collect()
}

/// Most recent sessions of a profile, newest first. A profile without transcripts yields `[]`.
pub fn list(config_dir: &Path, limit: usize) -> Vec<SessionInfo> {
    let mut found = Vec::new();
    for dir in project_dirs(config_dir) {
        if found.len() >= MAX_SCANNED {
            break;
        }
        transcripts_in(&dir, &mut found);
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.0.cmp(&b.id.0)));
    found.truncate(limit.clamp(1, MAX_LIMIT));
    found.into_iter().map(describe).collect()
}

fn describe(c: Candidate) -> SessionInfo {
    let meta = read_meta(&c.path, c.len);
    let resumable = meta.cwd.as_deref().is_some_and(|d| resume_dir(d).is_ok());
    SessionInfo {
        id: c.id.0,
        title: meta.custom_title.or(meta.ai_title),
        first_prompt: meta.first_prompt.or(meta.last_prompt),
        cwd: meta.cwd,
        last_activity: c
            .modified
            .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        size_bytes: c.len,
        resumable,
    }
}

/// Folder a session must be resumed in, re-read from its own transcript (never from the UI).
pub fn resume_cwd(config_dir: &Path, id: &SessionId) -> AppResult<PathBuf> {
    let file = format!("{}.jsonl", id.as_str());
    let path = project_dirs(config_dir)
        .into_iter()
        .map(|d| d.join(&file))
        .find(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_file()))
        .ok_or_else(|| {
            AppError::new(
                "session_not_found",
                "Cette conversation n'existe plus dans le profil",
            )
        })?;
    let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let cwd = read_meta(&path, len).cwd.ok_or_else(|| {
        AppError::new(
            "session_no_cwd",
            "Dossier de projet inconnu pour cette conversation",
        )
    })?;
    resume_dir(&cwd)
}

/// The recorded folder is untrusted: it must be an existing absolute directory, and `;` is refused
/// because Windows Terminal reads it as a command separator in `wt -d`.
fn resume_dir(cwd: &str) -> AppResult<PathBuf> {
    let p = Path::new(cwd);
    if cwd.contains(';') || cwd.chars().any(char::is_control) || !p.is_absolute() {
        return Err(AppError::new(
            "session_bad_cwd",
            "Le dossier de cette conversation ne peut pas être ouvert",
        ));
    }
    if !p.is_dir() {
        return Err(AppError::new(
            "session_cwd_missing",
            format!("Le dossier {cwd} n'existe plus"),
        ));
    }
    Ok(p.to_path_buf())
}

fn read_meta(path: &Path, len: u64) -> Meta {
    let mut meta = Meta::default();
    let Ok(mut f) = File::open(path) else {
        return meta;
    };
    let mut head = Vec::new();
    if f.by_ref().take(HEAD_BYTES).read_to_end(&mut head).is_err() {
        return meta;
    }
    scan(&head, false, len > head.len() as u64, &mut meta);
    if len > HEAD_BYTES {
        // One byte early, so the first (always dropped) fragment is empty when a line starts there.
        let start = len.saturating_sub(TAIL_BYTES).max(HEAD_BYTES) - 1;
        let mut tail = Vec::new();
        if f.seek(SeekFrom::Start(start)).is_ok()
            && f.take(TAIL_BYTES + 1).read_to_end(&mut tail).is_ok()
        {
            scan(&tail, true, false, &mut meta);
        }
    }
    meta
}

/// Parses complete lines of a chunk; a line cut by the chunk boundary is dropped.
fn scan(chunk: &[u8], cut_start: bool, cut_end: bool, meta: &mut Meta) {
    let mut lines: Vec<&[u8]> = chunk.split(|b| *b == b'\n').collect();
    if cut_end {
        lines.pop();
    }
    let skip = usize::from(cut_start);
    for line in lines.into_iter().skip(skip) {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(line) {
            absorb(&v, meta);
        }
    }
}

fn absorb(v: &Value, meta: &mut Meta) {
    let s = |k: &str| v.get(k).and_then(Value::as_str).and_then(clean);
    match v.get("type").and_then(Value::as_str) {
        // Later entries win: titles are re-appended when renamed.
        Some("custom-title") => meta.custom_title = s("customTitle").or(meta.custom_title.take()),
        Some("ai-title") => meta.ai_title = s("aiTitle").or(meta.ai_title.take()),
        Some("summary") => meta.ai_title = meta.ai_title.take().or(s("summary")),
        Some("last-prompt") => meta.last_prompt = s("lastPrompt").or(meta.last_prompt.take()),
        Some("user")
            if meta.first_prompt.is_none()
                && v.get("isMeta").and_then(Value::as_bool) != Some(true) =>
        {
            meta.first_prompt = v.pointer("/message/content").and_then(prompt_text);
        }
        _ => {}
    }
    if meta.cwd.is_none() {
        meta.cwd = v
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
    }
}

/// Text typed by the user; command wrappers and tool results (`<…>` or non-text blocks) are skipped.
fn prompt_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.as_str(),
        Value::Array(blocks) => blocks
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))?
            .get("text")?
            .as_str()?,
        _ => return None,
    };
    if text.trim_start().starts_with('<') {
        return None;
    }
    clean(text)
}

/// Single-line, length-bounded display text.
fn clean(s: &str) -> Option<String> {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.is_empty() {
        return None;
    }
    if one_line.chars().count() <= TEXT_CHARS {
        return Some(one_line);
    }
    let mut cut: String = one_line.chars().take(TEXT_CHARS - 1).collect();
    cut.push('…');
    Some(cut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const A: &str = "0e416f3e-5ea8-4e2a-b52c-919125b2c55e";
    const B: &str = "fc5b4117-b756-4f9d-9296-2838dc784d9c";

    fn write(dir: &Path, project: &str, id: &str, lines: &[String]) -> PathBuf {
        let d = dir.join("projects").join(project);
        fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{id}.jsonl"));
        fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    fn user(text: &str, cwd: &Path) -> String {
        serde_json::json!({"type":"user","cwd":cwd,"message":{"role":"user","content":text}})
            .to_string()
    }

    #[test]
    fn session_ids_are_strict_uuids() {
        assert_eq!(SessionId::parse(A).unwrap().as_str(), A);
        let upper = A.to_uppercase();
        assert_eq!(SessionId::parse(&upper).unwrap().as_str(), A);
        for bad in [
            "",
            "abc",
            "0e416f3e5ea84e2ab52c919125b2c55e",
            "{0e416f3e-5ea8-4e2a-b52c-919125b2c55}",
            "0e416f3e-5ea8-4e2a-b52c-919125b2c55e; calc",
            "0e416f3e-5ea8-4e2a-b52c-919125b2c5&e",
            "../../../../../../../../windows/x.ps",
        ] {
            assert!(SessionId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lists_newest_first_with_titles_prompts_and_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        write(
            tmp.path(),
            "C--work",
            A,
            &[
                r#"{"type":"mode","mode":"normal"}"#.into(),
                serde_json::json!({"type":"user","isMeta":true,"cwd":cwd,"message":{"content":"meta"}}).to_string(),
                user("<command-name>/clear</command-name>", &cwd),
                user("  analyse   ma\nram ", &cwd),
                r#"{"type":"ai-title","aiTitle":"Analyse RAM auto"}"#.into(),
            ],
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(
            tmp.path(),
            "C--other",
            B,
            &[
                r#"{"type":"custom-title","customTitle":"old"}"#.into(),
                serde_json::json!({"type":"user","cwd":"C:\\gone\\nowhere","message":{"content":[{"type":"tool_result"},{"type":"text","text":"hello"}]}}).to_string(),
                r#"{"type":"custom-title","customTitle":"b7"}"#.into(),
                r#"{"broken json"#.into(),
            ],
        );
        // Noise that must be ignored.
        write(tmp.path(), "C--work", "not-a-uuid", &["{}".into()]);
        fs::write(tmp.path().join("projects").join("stray.jsonl"), "{}").unwrap();

        let list = list(tmp.path(), 10);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, B);
        assert_eq!(list[0].title.as_deref(), Some("b7"));
        assert_eq!(list[0].first_prompt.as_deref(), Some("hello"));
        assert!(!list[0].resumable, "missing folder cannot be resumed");
        assert_eq!(list[1].title.as_deref(), Some("Analyse RAM auto"));
        assert_eq!(list[1].first_prompt.as_deref(), Some("analyse ma ram"));
        assert_eq!(list[1].cwd.as_deref(), cwd.to_str());
        assert!(list[1].resumable);
        assert!(list[1].size_bytes > 0 && list[1].last_activity.is_some());

        assert_eq!(super::list(tmp.path(), 1).len(), 1, "limit is honoured");
        assert!(super::list(&tmp.path().join("nothing"), 10).is_empty());
    }

    #[test]
    fn large_transcripts_are_read_only_at_both_ends() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_path_buf();
        let p = write(
            tmp.path(),
            "big",
            A,
            &[
                user("first question", &cwd),
                r#"{"type":"custom-title","customTitle":"early"}"#.into(),
            ],
        );
        let mut f = fs::OpenOptions::new().append(true).open(&p).unwrap();
        // A 1 MiB middle line straddles the head boundary and must not break parsing.
        let filler = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}\n",
            "x".repeat(1 << 20)
        );
        f.write_all(filler.as_bytes()).unwrap();
        f.write_all(b"{\"type\":\"custom-title\",\"customTitle\":\"renamed\"}\n")
            .unwrap();
        drop(f);

        let s = &list(tmp.path(), 5)[0];
        assert_eq!(s.title.as_deref(), Some("renamed"));
        assert_eq!(s.first_prompt.as_deref(), Some("first question"));
        assert!(s.size_bytes > 1 << 20);
    }

    #[test]
    fn long_prompts_are_truncated_on_one_line() {
        let t = clean(&"mot ".repeat(100)).unwrap();
        assert_eq!(t.chars().count(), TEXT_CHARS);
        assert!(t.ends_with('…') && !t.contains('\n'));
        assert_eq!(clean(" \n\t "), None);
    }

    #[test]
    fn resume_folder_comes_from_the_transcript_and_is_checked() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().join("proj");
        fs::create_dir_all(&cwd).unwrap();
        write(tmp.path(), "p", A, &[user("hi", &cwd)]);
        write(tmp.path(), "q", B, &[user("hi", Path::new("C:\\a;wt.exe"))]);
        let a = SessionId::parse(A).unwrap();
        assert_eq!(resume_cwd(tmp.path(), &a).unwrap(), cwd);
        let b = SessionId::parse(B).unwrap();
        assert_eq!(
            resume_cwd(tmp.path(), &b).unwrap_err().code,
            "session_bad_cwd"
        );
        let missing = SessionId::parse("00000000-0000-4000-8000-000000000000").unwrap();
        assert_eq!(
            resume_cwd(tmp.path(), &missing).unwrap_err().code,
            "session_not_found"
        );
        assert!(resume_dir("relative\\dir").is_err());
    }
}
