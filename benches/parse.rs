use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use tokctl::sources::{parse_claude_line, parse_codex_line, CodexCtx};

const CLAUDE_LINE: &str = r#"{"type":"assistant","timestamp":"2026-04-18T09:00:05.000Z","sessionId":"sess-a","message":{"id":"m1","model":"claude-sonnet-4-6","role":"assistant","usage":{"input_tokens":100,"output_tokens":40,"cache_read_input_tokens":500,"cache_creation_input_tokens":200}}}"#;

const CODEX_SEQUENCE: &[&str] = &[
    r#"{"timestamp":"2026-04-20T12:49:04.848Z","type":"session_meta","payload":{"id":"sess-x","cwd":"/Users/dev/repo","originator":"Codex Desktop"}}"#,
    r#"{"timestamp":"2026-04-20T12:49:05.000Z","type":"turn_context","payload":{"model":"gpt-5.4","cwd":"/Users/dev/repo"}}"#,
    r#"{"timestamp":"2026-04-20T12:49:10.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":60,"reasoning_output_tokens":10,"total_tokens":320},"total_token_usage":{"input_tokens":200,"cached_input_tokens":50,"output_tokens":60,"reasoning_output_tokens":10,"total_tokens":320}}}}"#,
];

fn bench_claude_parse_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("claude_parse_line");
    group.throughput(Throughput::Bytes(CLAUDE_LINE.len() as u64));
    group.bench_function("assistant", |b| {
        b.iter(|| {
            let r = parse_claude_line(black_box(CLAUDE_LINE), black_box(Some("/proj")));
            black_box(r);
        })
    });
    group.finish();
}

fn bench_codex_parse_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("codex_parse_line");
    let total_bytes: usize = CODEX_SEQUENCE.iter().map(|s| s.len()).sum();
    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.bench_function("session+turn+token", |b| {
        b.iter(|| {
            let mut ctx = CodexCtx::default();
            for line in CODEX_SEQUENCE {
                let r = parse_codex_line(black_box(line), &mut ctx);
                black_box(r);
            }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_claude_parse_line, bench_codex_parse_line);
criterion_main!(benches);
