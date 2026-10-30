default:
    @just --list

# Complete verification, shared by local development and CI.
check: fmt lint test-all test-minimal build

fmt:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

test filter="":
    cargo test --locked --all-features {{quote(filter)}}

test-all:
    cargo test --locked --all-features

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
