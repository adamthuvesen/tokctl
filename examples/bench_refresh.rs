//! Repeated TUI query timings against an explicitly selected, read-only cache.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokctl::store::Store;
use tokctl::tui::{
    data::DataCache,
    state::{AppState, TimeWindow},
};

const REPETITIONS: usize = 5;

fn verify(cache: &DataCache, expected_events: Option<u64>) -> Result<u64> {
    if let Some(error) = &cache.refresh_error {
        bail!("benchmark query failed: {}", error.display_message());
    }
    let events = cache.status.event_count;
    if events == 0 || cache.left.is_empty() {
        bail!("benchmark requires a nonempty cache and visible all-time rows");
    }
    if expected_events.is_some_and(|expected| expected != events) {
        bail!("cache changed during measurement, rerun against a stable snapshot");
    }
    Ok(events)
}

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    println!(
        "{label}: median={:.3} ms range={:.3}..{:.3} ms runs={}",
        samples[samples.len() / 2].as_secs_f64() * 1000.0,
        samples[0].as_secs_f64() * 1000.0,
        samples[samples.len() - 1].as_secs_f64() * 1000.0,
        samples.len(),
    );
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(
        args.next()
            .context("usage: cargo run --release --example bench_refresh -- /path/to/cache.db")?,
    );
    if args.next().is_some() {
        bail!("expected one cache.db path");
    }
    let store = Store::open_read_only(&path)?;
    let state = AppState {
        time_window: TimeWindow::All,
        ..Default::default()
    };
    let mut cold = Vec::with_capacity(REPETITIONS);
    let mut warm = Vec::with_capacity(REPETITIONS);
    let mut expected = None;
    for _ in 0..REPETITIONS {
        let mut cache = DataCache::default();
        let start = Instant::now();
        cache.refresh_all(&store, &state);
        cold.push(start.elapsed());
        expected = Some(verify(&cache, expected)?);
        let start = Instant::now();
        cache.refresh_all(&store, &state);
        warm.push(start.elapsed());
        verify(&cache, expected)?;
    }
    println!(
        "cache={} events={} errors=0",
        path.display(),
        expected.unwrap()
    );
    report("fresh TUI query cache", cold);
    report("populated TUI query cache", warm);
    println!("Excludes source ingestion and terminal drawing. Filesystem and SQLite page caches may be warm.");
    Ok(())
}
