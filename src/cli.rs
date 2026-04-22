use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::dates::{parse_since, parse_until};
use crate::discovery::{discover_claude, discover_codex, DiscoverOpts};
use crate::ingest::file_range::{ingest_claude_range, ingest_codex_range};
use crate::ingest::run::{run_ingest, RunIngestOptions};
use crate::legacy::in_memory::{
    daily_in_memory, filter_by_date, monthly_in_memory, session_in_memory,
};
use crate::paths::{default_claude_roots, default_codex_roots, resolve_roots, ResolveInput};
use crate::pricing;
use crate::render::{render_json, render_table, render_warnings};
use crate::store::queries::{daily_report, monthly_report, session_report, QueryFilter};
use crate::store::{open_store, store_path};
use crate::types::{AggregateRow, ReportKind, Source, SourceLabel, UsageEvent};

#[derive(Debug, Parser)]
#[command(
    name = "tokctl",
    version,
    about = "Token usage and cost report for Claude Code, Claude Desktop, Codex CLI, and Codex Desktop."
)]
struct Cli {
    /// Parallel parse threads (advanced). Default: physical core count.
    #[arg(long, global = true, hide = true)]
    threads: Option<usize>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Aggregate by date (YYYY-MM-DD, local time).
    Daily(ReportArgs),
    /// Aggregate by month (YYYY-MM, local time).
    Monthly(ReportArgs),
    /// Aggregate by session id (sorted by latest activity).
    Session(ReportArgs),
    /// Print the absolute path of the cache DB (does not create it).
    ExportDb,
}

#[derive(Debug, clap::Args)]
struct ReportArgs {
    /// claude | codex | all
    #[arg(long, default_value = "all")]
    source: String,
    /// inclusive lower bound (YYYY-MM-DD, local time)
    #[arg(long)]
    since: Option<String>,
    /// inclusive upper bound (YYYY-MM-DD, local time)
    #[arg(long)]
    until: Option<String>,
    /// emit machine-readable JSON instead of a table
    #[arg(long)]
    json: bool,
    /// one or more comma-separated Claude project roots
    #[arg(long = "claude-dir")]
    claude_dir: Option<String>,
    /// one or more comma-separated Codex session roots
    #[arg(long = "codex-dir")]
    codex_dir: Option<String>,
    /// delete the cache DB before running
    #[arg(long)]
    rebuild: bool,
    /// bypass the cache for this invocation
    #[arg(long = "no-cache")]
    no_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceArg {
    Claude,
    Codex,
    All,
}

impl SourceArg {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "claude" => Ok(SourceArg::Claude),
            "codex" => Ok(SourceArg::Codex),
            "all" => Ok(SourceArg::All),
            other => anyhow::bail!("--source must be claude | codex | all, got {other}"),
        }
    }

    fn to_filter(self) -> Option<Source> {
        match self {
            SourceArg::Claude => Some(Source::Claude),
            SourceArg::Codex => Some(Source::Codex),
            SourceArg::All => None,
        }
    }

    fn include_claude(self) -> bool {
        matches!(self, SourceArg::Claude | SourceArg::All)
    }
    fn include_codex(self) -> bool {
        matches!(self, SourceArg::Codex | SourceArg::All)
    }
    fn show_source(self) -> bool {
        matches!(self, SourceArg::All)
    }
    fn label(self) -> SourceLabel {
        match self {
            SourceArg::All => SourceLabel::All,
            SourceArg::Claude => SourceLabel::Source(Source::Claude),
            SourceArg::Codex => SourceLabel::Source(Source::Codex),
        }
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Some(n) = cli.threads {
        if n == 0 {
            anyhow::bail!("--threads must be >= 1, got {n}");
        }
        // Fails if the global pool is already built (e.g. in tests). That's
        // fine — subsequent calls are a no-op and the existing pool honours
        // the requested size.
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global();
    }

    match cli.command {
        Commands::Daily(a) => run_report(ReportKind::Daily, a),
        Commands::Monthly(a) => run_report(ReportKind::Monthly, a),
        Commands::Session(a) => run_report(ReportKind::Session, a),
        Commands::ExportDb => {
            println!("{}", store_path().display());
            Ok(())
        }
    }
}

fn run_report(kind: ReportKind, args: ReportArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }
    if args.no_cache {
        run_report_no_cache(kind, args)
    } else {
        run_report_cached(kind, args)
    }
}

fn resolve_all_roots(args: &ReportArgs) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let env = |k: &str| std::env::var(k).ok();
    let claude = resolve_roots(ResolveInput {
        flag: args.claude_dir.as_deref(),
        tokctl_env: env("TOKCTL_CLAUDE_DIR").as_deref(),
        tool_env: env("CLAUDE_CONFIG_DIR").as_deref(),
        tool_env_suffix: Some("projects"),
        defaults: default_claude_roots(),
    });
    let codex = resolve_roots(ResolveInput {
        flag: args.codex_dir.as_deref(),
        tokctl_env: env("TOKCTL_CODEX_DIR").as_deref(),
        tool_env: env("CODEX_HOME").as_deref(),
        tool_env_suffix: Some("sessions"),
        defaults: default_codex_roots(),
    });
    (existing_dirs(claude), existing_dirs(codex))
}

fn existing_dirs(resolved: crate::paths::ResolvedRoots) -> Vec<PathBuf> {
    resolved.roots.into_iter().filter(|p| p.is_dir()).collect()
}

fn run_report_cached(kind: ReportKind, args: ReportArgs) -> Result<()> {
    let source = SourceArg::parse(&args.source)?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;

    let cache_path = store_path();
    if args.rebuild {
        std::fs::remove_file(&cache_path).ok();
    }

    let (claude_roots, codex_roots) = resolve_all_roots(&args);

    let mut conn = open_store(&cache_path)
        .with_context(|| format!("opening cache at {}", cache_path.display()))?;

    let stats = run_ingest(RunIngestOptions {
        conn: &mut conn,
        claude_roots,
        codex_roots,
        include_claude: source.include_claude(),
        include_codex: source.include_codex(),
        safety_window_ms: 60 * 60 * 1000,
        now_ms: 0,
    })?;

    let filter = QueryFilter {
        source: source.to_filter(),
        since_ms: since.map(|t| t.timestamp_millis()),
        until_ms: until.map(|t| t.timestamp_millis()),
    };

    let rows = match kind {
        ReportKind::Daily => daily_report(&conn, filter)?,
        ReportKind::Monthly => monthly_report(&conn, filter)?,
        ReportKind::Session => session_report(&conn, filter)?,
    };

    emit(&rows, kind, source, args.json);

    let mut unknown = stats.unknown_models.clone();
    collect_unknown_from_db(&conn, source.to_filter(), &mut unknown);
    for w in render_warnings(&unknown, stats.skipped_lines) {
        eprintln!("{w}");
    }

    Ok(())
}

fn collect_unknown_from_db(
    conn: &Connection,
    source: Option<Source>,
    unknown: &mut HashSet<String>,
) {
    let sql = match source {
        Some(_) => "SELECT DISTINCT model FROM events WHERE source = ?1",
        None => "SELECT DISTINCT model FROM events",
    };
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return,
    };
    let iter: Result<Vec<String>, _> = match source {
        Some(s) => stmt
            .query_map([s.as_str()], |row| row.get::<_, String>(0))
            .and_then(|rs| rs.collect()),
        None => stmt
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rs| rs.collect()),
    };
    if let Ok(models) = iter {
        for m in models {
            if !pricing::has_price(&m) {
                unknown.insert(m);
            }
        }
    }
}

fn run_report_no_cache(kind: ReportKind, args: ReportArgs) -> Result<()> {
    let source = SourceArg::parse(&args.source)?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;

    let (claude_roots, codex_roots) = resolve_all_roots(&args);
    let mut events: Vec<UsageEvent> = Vec::new();
    let mut skipped_lines = 0usize;
    let mut unknown: HashSet<String> = HashSet::new();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let discover_opts = DiscoverOpts {
        safety_window_ms: 60 * 60 * 1000,
        now_ms,
    };
    let empty_manifest = std::collections::HashMap::<PathBuf, crate::store::FileManifestRow>::new();

    if source.include_claude() {
        let d = discover_claude(&claude_roots, &empty_manifest, discover_opts);
        // In-file dedup by message_id (Claude tools write the same usage twice on resume).
        let mut seen_ids: HashSet<String> = HashSet::new();
        for f in &d.files {
            let r = ingest_claude_range(&f.path, f.project.as_deref(), 0, f.size)?;
            skipped_lines += r.skipped_lines;
            for (ev, id) in r.events.into_iter().zip(
                r.message_ids
                    .into_iter()
                    .map(Some)
                    .chain(std::iter::repeat(None)),
            ) {
                // message_ids and events align 1:1 only when each event had an id.
                // Play it safe: dedup only when id is present.
                let key = id;
                if let Some(k) = &key {
                    if !seen_ids.insert(k.clone()) {
                        continue;
                    }
                }
                events.push(ev);
            }
        }
    }

    if source.include_codex() {
        let d = discover_codex(&codex_roots, &empty_manifest, discover_opts);
        for f in &d.files {
            let r = ingest_codex_range(&f.path, 0, f.size)?;
            skipped_lines += r.skipped_lines;
            events.extend(r.events);
        }
    }

    let filtered = filter_by_date(&events, since, until);
    let rows: Vec<AggregateRow> = match kind {
        ReportKind::Daily => daily_in_memory(&filtered, source.label(), &mut unknown),
        ReportKind::Monthly => monthly_in_memory(&filtered, source.label(), &mut unknown),
        ReportKind::Session => session_in_memory(&filtered, &mut unknown),
    };

    emit(&rows, kind, source, args.json);
    for w in render_warnings(&unknown, skipped_lines) {
        eprintln!("{w}");
    }
    Ok(())
}

fn emit(rows: &[AggregateRow], kind: ReportKind, source: SourceArg, as_json: bool) {
    let show_source = source.show_source();
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let body = if as_json {
        render_json(rows, kind, show_source)
    } else {
        render_table(rows, kind, show_source)
    };
    let _ = writeln!(lock, "{body}");
}

/// Thin wrapper so the process exits with the right code when Result is an Err.
pub fn main_exit() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
