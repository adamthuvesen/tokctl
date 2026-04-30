//! End-to-end timing for `DataCache::refresh_for` against a real cache.
//!
//! Runs the same cache+state interactions the TUI does on common
//! navigation paths and prints how long each took. Intended as a manual
//! sanity check after perf changes — not a CI gate.
//!
//! Run with `cargo run --release --example bench_refresh` against the
//! default cache path (or set `TOKCTL_DB` to override).

use std::time::Instant;

use tokctl::{
    store::open_store,
    tui::{
        data::DataCache,
        state::{
            AppState, RefreshMask, Section, SourceFilter, TimeWindow, TrendGranularity,
        },
    },
};

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn time<F: FnOnce()>(label: &str, f: F) {
    let t = Instant::now();
    f();
    println!("  {:<48} {:>7.2} ms", label, ms(t.elapsed()));
}

fn main() -> anyhow::Result<()> {
    let path = std::env::var_os("TOKCTL_DB")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(tokctl::store::store_path);
    println!("cache: {}", path.display());
    let conn = open_store(&path)?;

    let mut state = AppState {
        time_window: TimeWindow::All,
        ..AppState::default()
    };
    let mut cache = DataCache::default();

    println!("\n--- cold load (everything) ---");
    time("refresh_all (Days, all-time)", || {
        cache.refresh_all(&conn, &state);
    });

    println!("\n--- section switches with memoization ---");
    state.current_section = Section::Models;
    time("Days -> Models  (cold left)", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                ..Default::default()
            },
        );
    });

    state.current_section = Section::Provider;
    time("Models -> Provider (cold left)", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                ..Default::default()
            },
        );
    });

    state.current_section = Section::Days;
    time("Provider -> Days (memo hit)", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                ..Default::default()
            },
        );
    });

    state.current_section = Section::Models;
    time("Days -> Models  (memo hit)", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                ..Default::default()
            },
        );
    });

    println!("\n--- window / source / granularity ---");
    state.time_window = TimeWindow::Today;
    time("SetWindow(Today)  -> left+trend+sparkline", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                trend: true,
                sparkline: true,
                ..Default::default()
            },
        );
    });

    state.time_window = TimeWindow::All;
    time("SetWindow(All)  (memo hit on memos pop'd)", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                trend: true,
                sparkline: true,
                ..Default::default()
            },
        );
    });

    state.source_filter = SourceFilter::Claude;
    time("SetSource(Claude) -> cold per-key memo", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                trend: true,
                sparkline: true,
                ..Default::default()
            },
        );
    });

    state.source_filter = SourceFilter::All;
    state.trend_granularity = TrendGranularity::Daily;
    time("SetGranularity(Daily) -> left+trend cold", || {
        cache.refresh_for(
            &conn,
            &state,
            RefreshMask {
                left: true,
                trend: true,
                ..Default::default()
            },
        );
    });

    println!("\n--- toggle expand (no refresh) ---");
    time("ToggleExpand: empty mask", || {
        cache.refresh_for(&conn, &state, RefreshMask::default());
    });

    println!("\n--- manual refresh (clears memos, full reload) ---");
    cache.clear_memos();
    time("Refresh: cold, all slices", || {
        cache.refresh_all(&conn, &state);
    });

    println!("\nrows resident:  left={} trend={} sparkline={} sessions={} events={}",
        cache.left.len(), cache.trend.len(), cache.sparkline.len(),
        cache.sessions.len(), cache.events.len()
    );
    println!("status: {} events, mtime_ns={:?}", cache.status.event_count, cache.status.mtime_ns);

    Ok(())
}
