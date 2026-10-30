use crate::sources::{
    parse_claude_line_classified, parse_codex_line, parse_cursor_reader, ClaudeParseResult,
    CodexCtx, CodexParsed,
};
use crate::types::UsageEvent;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

#[derive(Debug, Default)]
pub struct ParseResult {
    pub events: Vec<UsageEvent>,
    pub skipped_lines: usize,
}

fn same_snapshot(left: &Metadata, right: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if left.dev() != right.dev()
            || left.ino() != right.ino()
            || left.ctime() != right.ctime()
            || left.ctime_nsec() != right.ctime_nsec()
        {
            return false;
        }
    }
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

fn read_snapshot<T>(
    path: &Path,
    expected: Option<&crate::discovery::DiscoveredFile>,
    parse: impl FnOnce(&mut File) -> Result<T>,
) -> Result<T> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let before = file.metadata()?;
    anyhow::ensure!(before.is_file(), "{} is not a regular file", path.display());
    anyhow::ensure!(
        expected.is_none_or(|expected| expected.matches_metadata(&before)),
        "{} changed since discovery",
        path.display()
    );
    let result = parse(&mut file).with_context(|| format!("reading {}", path.display()))?;
    let after = file.metadata()?;
    let current =
        std::fs::metadata(path).with_context(|| format!("checking {}", path.display()))?;
    anyhow::ensure!(
        same_snapshot(&before, &after) && same_snapshot(&after, &current),
        "{} changed while reading",
        path.display()
    );
    Ok(result)
}

pub fn parse_discovered_file(file: &crate::discovery::DiscoveredFile) -> Result<ParseResult> {
    read_snapshot(&file.path, Some(file), |reader| match file.source {
        crate::types::Source::Claude => parse_jsonl(reader, file.project.as_deref(), false),
        crate::types::Source::Codex => parse_jsonl(reader, None, true),
        crate::types::Source::Cursor => {
            let (events, skipped_lines) = parse_cursor_reader(reader, &file.path)?;
            Ok(ParseResult {
                events,
                skipped_lines,
            })
        }
    })
}

fn parse_jsonl(reader: impl Read, project: Option<&str>, codex: bool) -> Result<ParseResult> {
    let mut result = ParseResult::default();
    let mut seen_ids = HashSet::new();
    let mut context = CodexCtx::default();
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        let line = match std::str::from_utf8(&bytes) {
            Ok(line) => line.trim(),
            Err(_) => {
                result.skipped_lines += 1;
                continue;
            }
        };
        if line.is_empty() {
            continue;
        }
        let event = if codex {
            match parse_codex_line(line, &mut context) {
                Some(CodexParsed::Event(event)) => Some(event),
                Some(CodexParsed::ContextUpdated | CodexParsed::Skipped) => None,
                None => {
                    result.skipped_lines += 1;
                    None
                }
            }
        } else {
            match parse_claude_line_classified(line, project) {
                ClaudeParseResult::Event(parsed) => {
                    if parsed.message_id.is_some_and(|id| !seen_ids.insert(id)) {
                        continue;
                    }
                    Some(parsed.event)
                }
                ClaudeParseResult::Skipped => None,
                ClaudeParseResult::Malformed => {
                    result.skipped_lines += 1;
                    None
                }
            }
        };
        if let Some(event) = event {
            if event.validate().is_err() {
                result.skipped_lines += 1;
            } else {
                result.events.push(event);
            }
        }
    }
    Ok(result)
}

pub fn parse_claude_file(path: &Path, project: Option<&str>) -> Result<ParseResult> {
    read_snapshot(path, None, |file| parse_jsonl(file, project, false))
}

pub fn parse_codex_file(path: &Path) -> Result<ParseResult> {
    read_snapshot(path, None, |file| parse_jsonl(file, None, true))
}

pub fn parse_cursor_file(path: &Path) -> Result<ParseResult> {
    read_snapshot(path, None, |file| {
        let (events, skipped_lines) = parse_cursor_reader(file, path)?;
        Ok(ParseResult {
            events,
            skipped_lines,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn claude_range_dedupes_repeated_message_ids() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("dup.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        let line = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"s","message":{"id":"m","model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":500,"cache_creation_input_tokens":200}}}"#;
        writeln!(f, "{}", line).unwrap();
        writeln!(f, "{}", line).unwrap();
        drop(f);

        let r = parse_claude_file(&p, None).unwrap();
        assert_eq!(r.events.len(), 1);
        let ev = &r.events[0];
        assert_eq!(ev.input_tokens, 100);
        assert_eq!(ev.output_tokens, 40);
        assert_eq!(ev.cache_read_tokens, 500);
        assert_eq!(ev.cache_write_tokens, 200);
    }

    #[test]
    fn large_buffered_files_parse() {
        let dir = tempfile::tempdir().unwrap();

        let small_path = dir.path().join("small.jsonl");
        let mut f = std::fs::File::create(&small_path).unwrap();
        for i in 0..5 {
            let line = format!(
                r#"{{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"s","message":{{"id":"m-{i}","model":"claude-sonnet-4-6","usage":{{"input_tokens":1,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            );
            writeln!(f, "{}", line).unwrap();
        }
        drop(f);

        let big_path = dir.path().join("big.jsonl");
        let mut f = std::fs::File::create(&big_path).unwrap();
        let target = 1_048_576 + 1000;
        let mut i = 0usize;
        while f.metadata().unwrap().len() < target {
            let line = format!(
                r#"{{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"s","message":{{"id":"m-{i}","model":"claude-sonnet-4-6","usage":{{"input_tokens":1,"output_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            );
            writeln!(f, "{}", line).unwrap();
            i += 1;
        }
        drop(f);

        let small_r = parse_claude_file(&small_path, None).unwrap();
        let big_r = parse_claude_file(&big_path, None).unwrap();

        // Both produce N events where N = (file_size / line_size).
        // The exact count varies per-file; we just check both paths succeed
        // and produce positive, plausible counts.
        assert_eq!(small_r.events.len(), 5);
        assert!(big_r.events.len() > 100);
    }
}
