default:
    @just --list

# Fast lane: local, pre-commit, and the first CI step.
check: fmt lint test

# Slow lane: CI runs it after `check`.
slow: test-minimal build

# Drive the real binary: no args = drives for changed files; also `all`, `list`, `control`, or drive names.
verify *args:
    .agents/skills/verify-tokctl/verify {{args}}

# Auto-fixers: format in place.
fix:
    cargo fmt --all

fmt:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo clippy --locked --all-targets --no-default-features -- -D warnings

test *filter:
    cargo test --locked --all-features {{filter}}

test-minimal:
    cargo test --locked --no-default-features

build:
    cargo build --locked --release

verify-cli:
    cargo test --locked --all-features --test cli_behavior --test report_parity

verify-ui:
    cargo test --locked --all-features tui::
    cargo test --locked --all-features --test tui_terminal

# Requires an explicit SQLite workload. Empty caches and failed queries fail.
bench cache:
    cargo run --locked --release --example bench_refresh -- {{quote(cache)}}

bench-parsers:
    cargo bench --locked --bench parse
