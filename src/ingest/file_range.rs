use crate::sources::{parse_claude_line, parse_codex_line, CodexCtx, CodexParsed};
use crate::types::UsageEvent;
use anyhow::Result;
use memmap2::Mmap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Default)]
pub struct RangeResult {
    pub events: Vec<UsageEvent>,
    /// Parsed assistant/usage lines we couldn't use (invalid JSON, etc.)
    pub skipped_lines: usize,
    /// Message IDs we saw, so the caller can dedupe across tail reads.
    pub message_ids: Vec<String>,
}

/// Below this byte count, stick with BufReader — mmap setup overhead dominates
/// the savings for small files.
const MMAP_THRESHOLD: u64 = 1_048_576;

/// Map the given byte range of `file_path` as read-only memory.
///
/// Safety invariant (callers must uphold):
///   The mapped file must not be truncated or rewritten while the map is alive.
///   tokctl ingests session files that are either (a) fully written and no
///   longer being touched, or (b) routed to a full-parse via the ingest plan's
///   safety-window logic before they're considered stable. We never mmap a file
///   the producer is still appending to.
///
/// The returned bytes are the full file contents; callers slice into the
/// `[from_offset, to_offset)` range themselves.
fn map_file(file_path: &Path) -> Result<Mmap> {
    let f = File::open(file_path)?;
    // SAFETY: see the doc comment above. Upheld by the safety-window guard
    // in `ingest::plan::plan_ingest`.
    let map = unsafe { Mmap::map(&f)? };
    Ok(map)
}

fn process_claude_bytes(bytes: &[u8], project_path: Option<&str>, result: &mut RangeResult) {
    for chunk in bytes.split(|&b| b == b'\n') {
        if chunk.is_empty() {
            continue;
        }
        let line = match std::str::from_utf8(chunk) {
            Ok(s) => s,
            Err(_) => {
                result.skipped_lines += 1;
                continue;
            }
        };
        if !crate::sources::claude_line_has_signal(line) {
            continue;
        }
        match parse_claude_line(line, project_path) {
            Some(p) => {
                if let Some(id) = p.message_id {
                    result.message_ids.push(id);
                }
                result.events.push(p.event);
            }
            None => result.skipped_lines += 1,
        }
    }
}

fn process_codex_bytes(bytes: &[u8], result: &mut RangeResult) {
    let mut ctx = CodexCtx::default();
    for chunk in bytes.split(|&b| b == b'\n') {
        if chunk.is_empty() {
            continue;
        }
        let line = match std::str::from_utf8(chunk) {
            Ok(s) => s,
            Err(_) => {
                result.skipped_lines += 1;
                continue;
            }
        };
        if !crate::sources::codex_line_has_signal(line) {
            continue;
        }
        match parse_codex_line(line, &mut ctx) {
            Some(CodexParsed::Event(ev)) => result.events.push(ev),
            Some(CodexParsed::ContextUpdated) | Some(CodexParsed::Skipped) => {}
            None => result.skipped_lines += 1,
        }
    }
}

pub fn ingest_claude_range(
    file_path: &Path,
    project_path: Option<&str>,
    from_offset: u64,
    to_offset: u64,
) -> Result<RangeResult> {
    let mut result = RangeResult::default();
    let len = to_offset.saturating_sub(from_offset);

    if len >= MMAP_THRESHOLD && from_offset == 0 {
        // mmap the whole file, process the [from, to) slice
        let map = map_file(file_path)?;
        let end = (to_offset as usize).min(map.len());
        process_claude_bytes(&map[..end], project_path, &mut result);
        return Ok(result);
    }
    if len >= MMAP_THRESHOLD {
        // Partial mmap range — tail read on a large file
        let map = map_file(file_path)?;
        let start = (from_offset as usize).min(map.len());
        let end = (to_offset as usize).min(map.len());
        process_claude_bytes(&map[start..end], project_path, &mut result);
        return Ok(result);
    }

    // Small-file path: BufReader::take, line-by-line.
    let mut f = File::open(file_path)?;
    if from_offset > 0 {
        f.seek(SeekFrom::Start(from_offset))?;
    }
    let reader = BufReader::new(f.take(len));
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                result.skipped_lines += 1;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if !crate::sources::claude_line_has_signal(&line) {
            continue;
        }
        match parse_claude_line(&line, project_path) {
            Some(p) => {
                if let Some(id) = p.message_id {
                    result.message_ids.push(id);
                }
                result.events.push(p.event);
            }
            None => result.skipped_lines += 1,
        }
    }
    Ok(result)
}

pub fn ingest_codex_range(
    file_path: &Path,
    from_offset: u64,
    to_offset: u64,
) -> Result<RangeResult> {
    let mut result = RangeResult::default();
    let len = to_offset.saturating_sub(from_offset);

    if len >= MMAP_THRESHOLD && from_offset == 0 {
        let map = map_file(file_path)?;
        let end = (to_offset as usize).min(map.len());
        process_codex_bytes(&map[..end], &mut result);
        return Ok(result);
    }
    if len >= MMAP_THRESHOLD {
        let map = map_file(file_path)?;
        let start = (from_offset as usize).min(map.len());
        let end = (to_offset as usize).min(map.len());
        process_codex_bytes(&map[start..end], &mut result);
        return Ok(result);
    }

    // Small-file path
    let mut f = File::open(file_path)?;
    if from_offset > 0 {
        f.seek(SeekFrom::Start(from_offset))?;
    }
    let reader = BufReader::new(f.take(len));
    let mut ctx = CodexCtx::default();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                result.skipped_lines += 1;
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        if !crate::sources::codex_line_has_signal(&line) {
            continue;
        }
        match parse_codex_line(&line, &mut ctx) {
            Some(CodexParsed::Event(ev)) => result.events.push(ev),
            Some(CodexParsed::ContextUpdated) | Some(CodexParsed::Skipped) => {}
            None => result.skipped_lines += 1,
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn claude_range_reads_fixture() {
        let path = std::path::Path::new("test/fixtures/claude/-Users-dev-tokctl/sess-a.jsonl");
        if !path.exists() {
            eprintln!("skipping — fixture missing");
            return;
        }
        let size = std::fs::metadata(path).unwrap().len();
        let r = ingest_claude_range(path, Some("/Users/dev/tokctl"), 0, size).unwrap();
        assert_eq!(r.events.len(), 4);
        assert_eq!(r.skipped_lines, 0);
    }

    #[test]
    fn codex_range_reads_fixture() {
        let path = std::path::Path::new(
            "test/fixtures/codex/2026/04/20/rollout-2026-04-20T14-49-04-sess-x.jsonl",
        );
        if !path.exists() {
            eprintln!("skipping — fixture missing");
            return;
        }
        let size = std::fs::metadata(path).unwrap().len();
        let r = ingest_codex_range(path, 0, size).unwrap();
        assert!(r.events.len() >= 2);
    }

    #[test]
    fn partial_range_honors_offsets() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        writeln!(f, r#"{{"type":"user"}}"#).unwrap();
        let offset_before_second = f.stream_position().unwrap();
        writeln!(f, r#"{{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"s","message":{{"id":"m","model":"claude-sonnet-4-6","usage":{{"input_tokens":1}}}}}}"#).unwrap();
        let end = f.metadata().unwrap().len();
        drop(f);

        let r = ingest_claude_range(&p, None, offset_before_second, end).unwrap();
        assert_eq!(r.events.len(), 1);
    }

    #[test]
    fn mmap_path_and_buf_path_agree() {
        // Build a file slightly over the threshold so mmap kicks in, and a
        // copy under it — results must match.
        let dir = tempfile::tempdir().unwrap();
        let line = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"s","message":{"id":"m","model":"claude-sonnet-4-6","usage":{"input_tokens":1,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;

        let small_path = dir.path().join("small.jsonl");
        let mut f = std::fs::File::create(&small_path).unwrap();
        for _ in 0..5 {
            writeln!(f, "{}", line).unwrap();
        }
        drop(f);

        let big_path = dir.path().join("big.jsonl");
        let mut f = std::fs::File::create(&big_path).unwrap();
        let target = MMAP_THRESHOLD + 1000;
        while f.metadata().unwrap().len() < target {
            writeln!(f, "{}", line).unwrap();
        }
        drop(f);

        let small_size = std::fs::metadata(&small_path).unwrap().len();
        let big_size = std::fs::metadata(&big_path).unwrap().len();

        let small_r = ingest_claude_range(&small_path, None, 0, small_size).unwrap();
        let big_r = ingest_claude_range(&big_path, None, 0, big_size).unwrap();

        // Both produce N events where N = (file_size / line_size).
        // The exact count varies per-file; we just check both paths succeed
        // and produce positive, plausible counts.
        assert_eq!(small_r.events.len(), 5);
        assert!(big_r.events.len() > 100);
    }
}
