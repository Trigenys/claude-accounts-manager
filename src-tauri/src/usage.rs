//! Token usage from Claude Code transcripts (`<configDir>/projects/**/*.jsonl`).
//! 5-hour blocks follow ccusage's rules; percentages are estimates against a manual cap
//! or the profile's historical peak, never the official quota.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, Duration, DurationRound, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub ts: DateTime<Utc>,
    /// input + output + cache creation (cache reads are cheap and excluded).
    pub tokens: u64,
    pub key: String,
    /// Model family ("opus", "sonnet", "fable", "haiku"…), empty when unknown.
    pub model: String,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

pub fn parse_line(line: &str) -> Option<Entry> {
    let l: Line = serde_json::from_str(line).ok()?;
    if l.kind.as_deref() != Some("assistant") {
        return None;
    }
    let msg = l.message?;
    let u = msg.usage?;
    let ts = DateTime::parse_from_rfc3339(l.timestamp.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    let key = format!(
        "{}:{}",
        msg.id.unwrap_or_default(),
        l.request_id.unwrap_or_default()
    );
    let model = msg.model.as_deref().map(model_family).unwrap_or_default();
    Some(Entry {
        ts,
        tokens: u.input_tokens + u.output_tokens + u.cache_creation_input_tokens,
        key,
        model,
    })
}

/// `claude-opus-5-5` → `opus`, `claude-3-5-sonnet-2024…` → `sonnet`; synthetic/unknown ids → "".
pub fn model_family(id: &str) -> String {
    id.split('-')
        .filter(|p| {
            *p != "claude" && !p.is_empty() && !p.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .next()
        .filter(|p| p.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

pub fn parse_file(content: &str) -> Vec<Entry> {
    content.lines().filter_map(parse_line).collect()
}

/// Per-file parse cache keyed by (size, mtime) so refreshes only re-read changed transcripts.
#[derive(Default)]
pub struct UsageCache {
    files: Mutex<HashMap<PathBuf, (u64, SystemTime, Vec<Entry>)>>,
}

impl UsageCache {
    pub fn entries(&self, config_dir: &Path) -> Vec<Entry> {
        let root = config_dir.join("projects");
        let mut out = Vec::new();
        let mut cache = self.files.lock().unwrap_or_else(|p| p.into_inner());
        for f in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if f.path().extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = f.metadata() else { continue };
            let (len, mtime) = (
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            );
            let path = f.path().to_path_buf();
            let fresh = matches!(cache.get(&path), Some((l, m, _)) if *l == len && *m == mtime);
            if !fresh {
                let parsed = fs::read_to_string(&path)
                    .map(|c| parse_file(&c))
                    .unwrap_or_default();
                cache.insert(path.clone(), (len, mtime, parsed));
            }
            out.extend(cache[&path].2.iter().cloned());
        }
        dedup_sorted(out)
    }
}

pub fn dedup_sorted(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by_key(|e| e.ts);
    let mut seen = HashSet::new();
    entries.retain(|e| e.key == ":" || seen.insert(e.key.clone()));
    entries
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub tokens: u64,
}

/// ccusage rule: a block starts at the hour floor of its first entry and lasts 5 h;
/// a new block starts when an entry falls past the block end or after a > 5 h gap.
pub fn blocks(entries: &[Entry]) -> Vec<Block> {
    let five = Duration::hours(5);
    let mut out: Vec<Block> = Vec::new();
    let mut last: Option<DateTime<Utc>> = None;
    for e in entries {
        let new_block = match (out.last(), last) {
            (Some(b), Some(l)) => e.ts >= b.end || e.ts - l > five,
            _ => true,
        };
        if new_block {
            let start = e.ts.duration_trunc(Duration::hours(1)).unwrap_or(e.ts);
            out.push(Block {
                start,
                end: start + five,
                tokens: 0,
            });
        }
        if let Some(b) = out.last_mut() {
            b.tokens += e.tokens;
        }
        last = Some(e.ts);
    }
    out
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub tokens: u64,
    pub cap: Option<u64>,
    pub pct: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub account_id: String,
    pub five_hour: Window,
    pub weekly: Window,
    /// "official" | "manual" | "peak" | "none"
    pub cap_source: &'static str,
    /// When Claude Code last reported the official quotas (only with `cap_source == "official"`).
    pub official_at: Option<DateTime<Utc>>,
    /// Official limits scoped to a model or surface (e.g. Fable), when Claude Code reported them.
    pub scoped: Vec<ScopedUsage>,
    /// Last 7 days by model family, largest first.
    pub weekly_by_model: Vec<ModelTokens>,
    pub last_activity: Option<DateTime<Utc>>,
    pub has_transcripts: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScopedUsage {
    pub label: String,
    pub pct: f64,
    pub resets_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelTokens {
    pub model: String,
    pub tokens: u64,
}

/// Past windows needed before the historical peak is trusted as a cap.
const MIN_PAST_BLOCKS: usize = 3;

fn pct(tokens: u64, cap: Option<u64>) -> Option<f64> {
    cap.filter(|c| *c > 0)
        .map(|c| (tokens as f64 / c as f64 * 100.0).min(100.0))
}

pub fn summarize(
    account_id: &str,
    entries: &[Entry],
    now: DateTime<Utc>,
    manual_5h: Option<u64>,
    manual_week: Option<u64>,
) -> UsageSummary {
    let bl = blocks(entries);
    let active = bl.last().filter(|b| now >= b.start && now < b.end);
    // Peaks come from finished windows only: comparing the current window to itself always yields 100 %.
    let past_blocks = &bl[..bl.len() - usize::from(active.is_some())];
    let peak_5h = (past_blocks.len() >= MIN_PAST_BLOCKS)
        .then(|| past_blocks.iter().map(|b| b.tokens).max())
        .flatten()
        .filter(|p| *p > 0);

    let week_start = now - Duration::days(7);
    let weekly_tokens: u64 = entries
        .iter()
        .filter(|e| e.ts > week_start && e.ts <= now)
        .map(|e| e.tokens)
        .sum();
    // Busiest rolling 7-day window that ended before the current one (same shape as `weekly_tokens`).
    let past: Vec<&Entry> = entries.iter().filter(|e| e.ts <= week_start).collect();
    let (mut lo, mut sum, mut best) = (0usize, 0u64, 0u64);
    for e in &past {
        sum += e.tokens;
        while past[lo].ts <= e.ts - Duration::days(7) {
            sum -= past[lo].tokens;
            lo += 1;
        }
        best = best.max(sum);
    }
    let peak_week = Some(best).filter(|p| *p > 0);

    let mut by_model: HashMap<&str, u64> = HashMap::new();
    for e in entries.iter().filter(|e| e.ts > week_start && e.ts <= now) {
        *by_model.entry(e.model.as_str()).or_default() += e.tokens;
    }
    let mut weekly_by_model: Vec<ModelTokens> = by_model
        .into_iter()
        .filter(|(_, t)| *t > 0)
        .map(|(m, t)| ModelTokens {
            model: m.to_string(),
            tokens: t,
        })
        .collect();
    weekly_by_model.sort_by(|a, b| b.tokens.cmp(&a.tokens).then(a.model.cmp(&b.model)));

    let cap_5h = manual_5h.or(peak_5h);
    let cap_week = manual_week.or(peak_week);
    let cap_source = if manual_5h.is_some() || manual_week.is_some() {
        "manual"
    } else if peak_5h.is_some() || peak_week.is_some() {
        "peak"
    } else {
        "none"
    };
    let tokens_5h = active.map(|b| b.tokens).unwrap_or(0);
    let oldest_in_week = entries
        .iter()
        .find(|e| e.ts > week_start)
        .map(|e| e.ts + Duration::days(7));

    UsageSummary {
        account_id: account_id.to_string(),
        five_hour: Window {
            tokens: tokens_5h,
            cap: cap_5h,
            pct: pct(tokens_5h, cap_5h),
            resets_at: active.map(|b| b.end),
        },
        weekly: Window {
            tokens: weekly_tokens,
            cap: cap_week,
            pct: pct(weekly_tokens, cap_week),
            resets_at: oldest_in_week,
        },
        cap_source,
        official_at: None,
        scoped: Vec::new(),
        weekly_by_model,
        last_activity: entries.last().map(|e| e.ts),
        has_transcripts: !entries.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/sample.jsonl");

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn parses_assistant_usage_and_skips_noise() {
        let entries = dedup_sorted(parse_file(FIXTURE));
        // 5 assistant lines, one duplicate (same message id + request id), plus noise/truncated lines.
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].tokens, 2 + 365 + 16029);
    }

    #[test]
    fn splits_blocks_on_end_and_gap() {
        let entries = dedup_sorted(parse_file(FIXTURE));
        let b = blocks(&entries);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].start, t("2026-09-23T21:00:00Z"));
        assert_eq!(b[0].end, t("2026-09-24T02:00:00Z"));
        assert_eq!(b[0].tokens, 16396 + 1000);
    }

    #[test]
    fn summary_uses_active_block_and_peak_cap() {
        let entries = dedup_sorted(parse_file(FIXTURE));
        // Only two finished blocks before the active one: not enough history for a cap.
        let s = summarize("a", &entries, t("2026-09-25T10:30:00Z"), None, None);
        assert_eq!(s.five_hour.tokens, 500);
        assert_eq!(s.five_hour.cap, None);
        assert_eq!(s.five_hour.pct, None);
        // Once all three are finished, the peak becomes the cap.
        let later = summarize("a", &entries, t("2026-09-26T10:30:00Z"), None, None);
        assert_eq!(later.five_hour.cap, Some(17396));
        assert_eq!(later.cap_source, "peak");
        let idle = summarize("a", &entries, t("2026-09-26T10:30:00Z"), Some(1000), None);
        assert_eq!(idle.five_hour.tokens, 0);
        assert_eq!(idle.five_hour.pct, Some(0.0));
        assert_eq!(idle.cap_source, "manual");
    }

    #[test]
    fn empty_profile_has_no_cap() {
        let s = summarize("a", &[], Utc::now(), None, None);
        assert_eq!(s.cap_source, "none");
        assert_eq!(s.five_hour.pct, None);
        assert!(!s.has_transcripts);
    }

    fn e(ts: &str, tokens: u64, model: &str) -> Entry {
        Entry {
            ts: t(ts),
            tokens,
            key: format!("{ts}{model}"),
            model: model.into(),
        }
    }

    #[test]
    fn current_window_never_counts_as_its_own_peak() {
        // A single, still-active block used to report 100 %.
        let s = summarize(
            "a",
            &[e("2026-09-25T10:00:00Z", 19_200, "opus")],
            t("2026-09-25T10:30:00Z"),
            None,
            None,
        );
        assert_eq!(s.five_hour.pct, None);
        assert_eq!(s.weekly.pct, None);
        assert_eq!(s.cap_source, "none");
    }

    #[test]
    fn weekly_peak_comes_from_past_weeks() {
        let entries = [
            e("2026-09-10T10:00:00Z", 1_000, "opus"),
            e("2026-09-25T10:00:00Z", 250, "fable"),
        ];
        let s = summarize("a", &entries, t("2026-09-25T12:00:00Z"), None, None);
        assert_eq!(s.weekly.cap, Some(1_000));
        assert_eq!(s.weekly.pct, Some(25.0));
        // Two days straddling a calendar-week edge still form one rolling week.
        let spread = [
            e("2026-09-09T10:00:00Z", 600, "opus"),
            e("2026-09-11T10:00:00Z", 600, "opus"),
            e("2026-09-25T10:00:00Z", 600, "opus"),
        ];
        let r = summarize("a", &spread, t("2026-09-25T12:00:00Z"), None, None);
        assert_eq!(r.weekly.cap, Some(1_200));
        assert_eq!(r.weekly.pct, Some(50.0));
    }

    #[test]
    fn splits_week_by_model_family() {
        assert_eq!(model_family("claude-fable-5-1"), "fable");
        assert_eq!(model_family("claude-3-5-sonnet-20241022"), "sonnet");
        assert_eq!(model_family("<synthetic>"), "");
        let entries = [
            e("2026-09-24T10:00:00Z", 300, "fable"),
            e("2026-09-24T11:00:00Z", 500, "opus"),
            e("2026-09-25T09:00:00Z", 200, "fable"),
            e("2026-09-01T09:00:00Z", 9_000, "fable"),
        ];
        let s = summarize("a", &entries, t("2026-09-25T12:00:00Z"), None, None);
        assert_eq!(
            s.weekly_by_model,
            vec![
                ModelTokens {
                    model: "fable".into(),
                    tokens: 500
                },
                ModelTokens {
                    model: "opus".into(),
                    tokens: 500
                }
            ]
        );
    }
}
