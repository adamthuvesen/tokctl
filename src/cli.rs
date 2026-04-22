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
    daily_in_memory, filter_by_date, filter_by_repo, monthly_in_memory, repo_in_memory,
    resolve_repos, session_in_memory,
};
use crate::paths::{default_claude_roots, default_codex_roots, resolve_roots, ResolveInput};
use crate::pricing;
use crate::render::{
    render_json, render_repo_json, render_repo_table, render_table, render_warnings,
};
use crate::store::queries::{
    daily_report, monthly_report, repo_report, resolve_repo_filter, session_report, QueryFilter,
    RepoFilterSpec,
};
use crate::store::{open_store, store_path};
use crate::types::{AggregateRow, ReportKind, Source, SourceLabel, UsageEvent};

#[derive(Debug, Parser)]
#[command(
    name = "tokctl",
    version,
    about = "Token usage and cost report for Claude and Codex."
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
    /// Aggregate by repo. Pass a repo name to drill down into its sessions.
    Repo(RepoArgs),
    /// Print the absolute path of the cache DB (does not create it).
    ExportDb,
    /// Launch the interactive terminal UI (read-only against the cache).
    Ui,
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
    /// filter to a repo (display name, path prefix, or `(no-repo)`)
    #[arg(long)]
    repo: Option<String>,
    /// override the implicit grouping (day | month | session | repo)
    #[arg(long = "group-by")]
    group_by: Option<String>,
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

#[derive(Debug, clap::Args)]
struct RepoArgs {
    /// Optional repo name (display name, path prefix, or `(no-repo)`). When
    /// provided, shows sessions belonging to that repo instead of the
    /// repo-level summary.
    name: Option<String>,
    #[arg(long, default_value = "all")]
    source: String,
    #[arg(long)]
    since: Option<String>,
    #[arg(long)]
    until: Option<String>,
    #[arg(long)]
    json: bool,
    #[arg(long = "claude-dir")]
    claude_dir: Option<String>,
    #[arg(long = "codex-dir")]
    codex_dir: Option<String>,
    #[arg(long)]
    rebuild: bool,
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

/// The effective grouping axis after resolving any `--group-by` override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupBy {
    Day,
    Month,
    Session,
    Repo,
}

impl GroupBy {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "day" => Ok(GroupBy::Day),
            "month" => Ok(GroupBy::Month),
            "session" => Ok(GroupBy::Session),
            "repo" => Ok(GroupBy::Repo),
            other => anyhow::bail!("--group-by must be day | month | session | repo, got {other}"),
        }
    }
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Some(n) = cli.threads {
        if n == 0 {
            anyhow::bail!("--threads must be >= 1, got {n}");
        }
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global();
    }

    match cli.command {
        Commands::Daily(a) => run_report(ReportKind::Daily, a),
        Commands::Monthly(a) => run_report(ReportKind::Monthly, a),
        Commands::Session(a) => run_report(ReportKind::Session, a),
        Commands::Repo(a) => run_repo(a),
        Commands::ExportDb => {
            println!("{}", store_path().display());
            Ok(())
        }
        Commands::Ui => crate::tui::run(),
    }
}

fn run_report(kind: ReportKind, args: ReportArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }

    // Resolve --group-by override. `repo` escalates to the dedicated path.
    let group = match args.group_by.as_deref() {
        None => default_group_for(kind),
        Some(s) => GroupBy::parse(s)?,
    };

    if args.no_cache {
        run_report_no_cache(group, args)
    } else {
        run_report_cached(group, args)
    }
}

fn default_group_for(kind: ReportKind) -> GroupBy {
    match kind {
        ReportKind::Daily => GroupBy::Day,
        ReportKind::Monthly => GroupBy::Month,
        ReportKind::Session => GroupBy::Session,
    }
}

fn resolve_all_roots(
    claude_dir: Option<&str>,
    codex_dir: Option<&str>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let env = |k: &str| std::env::var(k).ok();
    let claude = resolve_roots(ResolveInput {
        flag: claude_dir,
        tokctl_env: env("TOKCTL_CLAUDE_DIR").as_deref(),
        tool_env: env("CLAUDE_CONFIG_DIR").as_deref(),
        tool_env_suffix: Some("projects"),
        defaults: default_claude_roots(),
    });
    let codex = resolve_roots(ResolveInput {
        flag: codex_dir,
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

fn run_report_cached(group: GroupBy, args: ReportArgs) -> Result<()> {
    let source = SourceArg::parse(&args.source)?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;

    let cache_path = store_path();
    if args.rebuild {
        std::fs::remove_file(&cache_path).ok();
    }

    let (claude_roots, codex_roots) =
        resolve_all_roots(args.claude_dir.as_deref(), args.codex_dir.as_deref());

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

    let repo_spec = match args.repo.as_deref() {
        Some(name) => Some(resolve_repo_filter(&conn, name)?),
        None => None,
    };

    let filter = QueryFilter {
        source: source.to_filter(),
        since_ms: since.map(|t| t.timestamp_millis()),
        until_ms: until.map(|t| t.timestamp_millis()),
        repo: repo_spec,
    };

    match group {
        GroupBy::Day => {
            let rows = daily_report(&conn, filter)?;
            emit(&rows, ReportKind::Daily, source, args.json);
        }
        GroupBy::Month => {
            let rows = monthly_report(&conn, filter)?;
            emit(&rows, ReportKind::Monthly, source, args.json);
        }
        GroupBy::Session => {
            let rows = session_report(&conn, filter)?;
            emit(&rows, ReportKind::Session, source, args.json);
        }
        GroupBy::Repo => {
            let rows = repo_report(&conn, filter)?;
            emit_repo(&rows, args.json);
        }
    }

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

fn run_report_no_cache(group: GroupBy, args: ReportArgs) -> Result<()> {
    let source = SourceArg::parse(&args.source)?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;

    let (claude_roots, codex_roots) =
        resolve_all_roots(args.claude_dir.as_deref(), args.codex_dir.as_deref());
    let (events, skipped_lines) = gather_events_no_cache(source, &claude_roots, &codex_roots)?;
    let filtered = filter_by_date(&events, since, until);
    let resolved = resolve_repos(&filtered);

    // Resolve --repo against the in-memory resolved set.
    let repo_spec = args.repo.as_deref().map(resolve_repo_filter_in_memory);

    let mut unknown: HashSet<String> = HashSet::new();
    match group {
        GroupBy::Day => {
            let filtered = filter_by_repo(&resolved, &repo_spec);
            let rows = daily_in_memory(&filtered, source.label(), &mut unknown);
            emit(&rows, ReportKind::Daily, source, args.json);
        }
        GroupBy::Month => {
            let filtered = filter_by_repo(&resolved, &repo_spec);
            let rows = monthly_in_memory(&filtered, source.label(), &mut unknown);
            emit(&rows, ReportKind::Monthly, source, args.json);
        }
        GroupBy::Session => {
            let filtered = filter_by_repo(&resolved, &repo_spec);
            let rows = session_in_memory(&filtered, &mut unknown);
            emit(&rows, ReportKind::Session, source, args.json);
        }
        GroupBy::Repo => {
            let rows = repo_in_memory(&resolved, &repo_spec, &mut unknown);
            emit_repo(&rows, args.json);
        }
    }

    for w in render_warnings(&unknown, skipped_lines) {
        eprintln!("{w}");
    }
    Ok(())
}

/// Simpler no-DB repo spec resolver. Display names are all we can match on
/// without a store — keeps `--no-cache` deterministic. Path-prefix and
/// no-repo sentinels are preserved.
fn resolve_repo_filter_in_memory(name: &str) -> RepoFilterSpec {
    if name == crate::repo::RepoIdentity::NO_REPO_DISPLAY {
        RepoFilterSpec::NoRepo
    } else if name.starts_with('/') {
        RepoFilterSpec::KeyPrefix(name.to_owned())
    } else {
        RepoFilterSpec::DisplayName(name.to_owned())
    }
}

fn gather_events_no_cache(
    source: SourceArg,
    claude_roots: &[PathBuf],
    codex_roots: &[PathBuf],
) -> Result<(Vec<UsageEvent>, usize)> {
    let mut events: Vec<UsageEvent> = Vec::new();
    let mut skipped_lines = 0usize;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let discover_opts = DiscoverOpts {
        safety_window_ms: 60 * 60 * 1000,
        now_ms,
    };
    let empty_manifest = std::collections::HashMap::<PathBuf, crate::store::FileManifestRow>::new();

    if source.include_claude() {
        let d = discover_claude(claude_roots, &empty_manifest, discover_opts);
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
                if let Some(k) = &id {
                    if !seen_ids.insert(k.clone()) {
                        continue;
                    }
                }
                events.push(ev);
            }
        }
    }
    if source.include_codex() {
        let d = discover_codex(codex_roots, &empty_manifest, discover_opts);
        for f in &d.files {
            let r = ingest_codex_range(&f.path, 0, f.size)?;
            skipped_lines += r.skipped_lines;
            events.extend(r.events);
        }
    }
    Ok((events, skipped_lines))
}

fn run_repo(args: RepoArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }
    let source = SourceArg::parse(&args.source)?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    let (claude_roots, codex_roots) =
        resolve_all_roots(args.claude_dir.as_deref(), args.codex_dir.as_deref());

    if args.no_cache {
        let (events, skipped_lines) = gather_events_no_cache(source, &claude_roots, &codex_roots)?;
        let filtered = filter_by_date(&events, since, until);
        let resolved = resolve_repos(&filtered);
        let mut unknown: HashSet<String> = HashSet::new();
        match args.name.as_deref() {
            None => {
                let rows = repo_in_memory(&resolved, &None, &mut unknown);
                emit_repo(&rows, args.json);
            }
            Some(name) => {
                let spec = Some(resolve_repo_filter_in_memory(name));
                let only = filter_by_repo(&resolved, &spec);
                let rows = session_in_memory(&only, &mut unknown);
                emit(&rows, ReportKind::Session, source, args.json);
            }
        }
        for w in render_warnings(&unknown, skipped_lines) {
            eprintln!("{w}");
        }
        return Ok(());
    }

    let cache_path = store_path();
    if args.rebuild {
        std::fs::remove_file(&cache_path).ok();
    }
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

    let mut filter = QueryFilter {
        source: source.to_filter(),
        since_ms: since.map(|t| t.timestamp_millis()),
        until_ms: until.map(|t| t.timestamp_millis()),
        repo: None,
    };

    match args.name.as_deref() {
        None => {
            let rows = repo_report(&conn, filter)?;
            emit_repo(&rows, args.json);
        }
        Some(name) => {
            filter.repo = Some(resolve_repo_filter(&conn, name)?);
            let rows = session_report(&conn, filter)?;
            emit(&rows, ReportKind::Session, source, args.json);
        }
    }

    let mut unknown = stats.unknown_models.clone();
    collect_unknown_from_db(&conn, source.to_filter(), &mut unknown);
    for w in render_warnings(&unknown, stats.skipped_lines) {
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

fn emit_repo(rows: &[crate::store::queries::RepoAggregateRow], as_json: bool) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let body = if as_json {
        render_repo_json(rows)
    } else {
        render_repo_table(rows)
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
