use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use rusqlite::Connection;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::compare::{self, CompareDimension};
use crate::cursor_sync::{
    has_configured_account, list_accounts, save_credentials, sync_active_account,
    validate_active_account, validate_cursor_session,
};
use crate::dates::{parse_since, parse_until};
use crate::discovery::{discover_claude, discover_codex, discover_cursor, DiscoverOpts};
use crate::ingest::file_range::{ingest_claude_range, ingest_codex_range, ingest_cursor_range};
use crate::ingest::run::{run_ingest, RunIngestOptions};
use crate::legacy::in_memory::{
    daily_in_memory, filter_by_date, filter_by_repo, monthly_in_memory, repo_in_memory,
    resolve_repos, session_in_memory,
};
use crate::paths::{
    cursor_sync_cache_dir, default_claude_roots, default_codex_roots, default_cursor_roots,
    resolve_roots, ResolveInput,
};
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
    about = "Token usage and cost report for Claude, Codex, and Cursor."
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
    /// Inspect local setup, cache health, and pricing coverage.
    Doctor(DoctorArgs),
    /// Compare usage across two calendar windows.
    Compare(CompareArgs),
    /// Print the absolute path of the cache DB (does not create it).
    ExportDb,
    /// Cursor account setup, sync, and status.
    Cursor(CursorArgs),
    /// Launch the interactive terminal UI (read-only against the cache).
    Ui,
}

#[derive(Debug, clap::Args)]
struct DoctorArgs {
    /// emit machine-readable JSON instead of a table
    #[arg(long)]
    json: bool,
    /// exit non-zero if any warning or error is present
    #[arg(long)]
    strict: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CompareByArg {
    Source,
    Repo,
    Model,
    Session,
    All,
}

impl CompareByArg {
    fn dimensions(self) -> Vec<CompareDimension> {
        match self {
            CompareByArg::Source => vec![CompareDimension::Source],
            CompareByArg::Repo => vec![CompareDimension::Repo],
            CompareByArg::Model => vec![CompareDimension::Model],
            CompareByArg::Session => vec![CompareDimension::Session],
            CompareByArg::All => CompareDimension::ALL.to_vec(),
        }
    }
}

#[derive(Debug, clap::Args)]
struct CompareArgs {
    /// current window preset or YYYY-MM-DD..YYYY-MM-DD range
    current: Option<String>,
    /// baseline window preset or YYYY-MM-DD..YYYY-MM-DD range
    baseline: Option<String>,
    /// claude | codex | cursor | all
    #[arg(long, default_value = "all")]
    source: String,
    /// filter to a repo (display name, path prefix, or `(no-repo)`)
    #[arg(long)]
    repo: Option<String>,
    /// source | repo | model | session | all
    #[arg(long = "by", value_enum, default_value_t = CompareByArg::All)]
    by: CompareByArg,
    /// number of positive and negative driver rows to show per breakdown
    #[arg(long, default_value_t = 5)]
    top: usize,
    /// emit machine-readable JSON instead of a table
    #[arg(long)]
    json: bool,
    /// one or more comma-separated Claude project roots
    #[arg(long = "claude-dir")]
    claude_dir: Option<String>,
    /// one or more comma-separated Codex session roots
    #[arg(long = "codex-dir")]
    codex_dir: Option<String>,
    /// one or more comma-separated Cursor usage CSV roots
    #[arg(long = "cursor-dir")]
    cursor_dir: Option<String>,
    /// delete the cache DB before running
    #[arg(long)]
    rebuild: bool,
    /// bypass the cache for this invocation
    #[arg(long = "no-cache")]
    no_cache: bool,
}

#[derive(Debug, clap::Args)]
struct CursorArgs {
    #[command(subcommand)]
    command: CursorCommands,
}

#[derive(Debug, Subcommand)]
enum CursorCommands {
    /// Validate and store a Cursor session token locally.
    Login(CursorLoginArgs),
    /// Show configured Cursor account status.
    Status,
    /// Fetch Cursor usage CSV into the local cache.
    Sync,
}

#[derive(Debug, clap::Args)]
struct CursorLoginArgs {
    /// Cursor session token. If omitted, tokctl prompts on stdin.
    #[arg(long)]
    token: Option<String>,
    /// Optional local label for the account.
    #[arg(long)]
    label: Option<String>,
}

#[derive(Debug, clap::Args)]
struct ReportArgs {
    /// claude | codex | cursor | all
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
    /// one or more comma-separated Cursor usage CSV roots
    #[arg(long = "cursor-dir")]
    cursor_dir: Option<String>,
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
    #[arg(long = "cursor-dir")]
    cursor_dir: Option<String>,
    #[arg(long)]
    rebuild: bool,
    #[arg(long = "no-cache")]
    no_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceArg {
    Claude,
    Codex,
    Cursor,
    All,
}

impl SourceArg {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "claude" => Ok(SourceArg::Claude),
            "codex" => Ok(SourceArg::Codex),
            "cursor" => Ok(SourceArg::Cursor),
            "all" => Ok(SourceArg::All),
            other => anyhow::bail!("--source must be claude | codex | cursor | all, got {other}"),
        }
    }

    fn to_filter(self) -> Option<Source> {
        match self {
            SourceArg::Claude => Some(Source::Claude),
            SourceArg::Codex => Some(Source::Codex),
            SourceArg::Cursor => Some(Source::Cursor),
            SourceArg::All => None,
        }
    }

    fn include_claude(self) -> bool {
        matches!(self, SourceArg::Claude | SourceArg::All)
    }
    fn include_codex(self) -> bool {
        matches!(self, SourceArg::Codex | SourceArg::All)
    }
    fn include_cursor(self) -> bool {
        matches!(self, SourceArg::Cursor | SourceArg::All)
    }
    fn show_source(self) -> bool {
        matches!(self, SourceArg::All)
    }
    fn label(self) -> SourceLabel {
        match self {
            SourceArg::All => SourceLabel::All,
            SourceArg::Claude => SourceLabel::Source(Source::Claude),
            SourceArg::Codex => SourceLabel::Source(Source::Codex),
            SourceArg::Cursor => SourceLabel::Source(Source::Cursor),
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
        Commands::Doctor(a) => run_doctor(a),
        Commands::Compare(a) => run_compare(a),
        Commands::ExportDb => {
            println!("{}", store_path().display());
            Ok(())
        }
        Commands::Cursor(a) => run_cursor(a),
        Commands::Ui => run_ui(),
    }
}

fn run_doctor(args: DoctorArgs) -> Result<()> {
    let report = crate::doctor::run();
    let body = if args.json {
        crate::doctor::render_json(&report)
    } else {
        crate::doctor::render_human(&report)
    };
    println!("{body}");
    if args.strict && report.status != crate::doctor::CheckSeverity::Ok {
        anyhow::bail!("doctor found {}", report.status.as_str());
    }
    Ok(())
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

fn run_compare(args: CompareArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }
    if args.top == 0 {
        anyhow::bail!("--top must be >= 1");
    }

    let source = SourceArg::parse(&args.source)?;
    let windows = compare::resolve_windows(
        args.current.as_deref(),
        args.baseline.as_deref(),
        chrono::Utc::now(),
    )?;
    let dimensions = args.by.dimensions();

    if source.include_cursor() {
        let target = cursor_sync_target_dir(args.cursor_dir.as_deref());
        maybe_sync_cursor(Some(&target));
    }

    let (claude_roots, codex_roots, cursor_roots) = resolve_all_roots(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );

    let report = if args.no_cache {
        let (events, skipped_lines) =
            gather_events_no_cache(source, &claude_roots, &codex_roots, &cursor_roots)?;
        let resolved = resolve_repos(&events);
        let repo_spec = args.repo.as_deref().map(resolve_repo_filter_in_memory);
        let mut unknown = HashSet::new();
        let report = compare::compare_from_events(
            &resolved,
            windows,
            &repo_spec,
            &dimensions,
            args.top,
            &mut unknown,
        );
        for warning in render_warnings(&unknown, skipped_lines) {
            eprintln!("{warning}");
        }
        report
    } else {
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
            cursor_roots,
            include_claude: source.include_claude(),
            include_codex: source.include_codex(),
            include_cursor: source.include_cursor(),
            safety_window_ms: 60 * 60 * 1000,
            now_ms: 0,
        })?;
        let repo_spec = match args.repo.as_deref() {
            Some(name) => Some(resolve_repo_filter(&conn, name)?),
            None => None,
        };
        let filter = QueryFilter {
            source: source.to_filter(),
            since_ms: None,
            until_ms: None,
            repo: repo_spec,
        };
        let report = compare::compare_from_db(&conn, windows, filter, &dimensions, args.top)?;
        let mut unknown = stats.unknown_models.clone();
        collect_unknown_from_db(&conn, source.to_filter(), &mut unknown);
        for warning in render_warnings(&unknown, stats.skipped_lines) {
            eprintln!("{warning}");
        }
        report
    };

    let body = if args.json {
        compare::render_json(&report)
    } else {
        compare::render_human(&report)
    };
    println!("{body}");
    Ok(())
}

fn run_ui() -> Result<()> {
    maybe_sync_cursor(None);
    let (claude_roots, codex_roots, cursor_roots) = resolve_all_roots(None, None, None);
    let cache_path = store_path();
    let mut conn = open_store(&cache_path)
        .with_context(|| format!("opening cache at {}", cache_path.display()))?;
    let stats = run_ingest(RunIngestOptions {
        conn: &mut conn,
        claude_roots,
        codex_roots,
        cursor_roots,
        include_claude: true,
        include_codex: true,
        include_cursor: true,
        safety_window_ms: 60 * 60 * 1000,
        now_ms: 0,
    })?;
    let mut unknown = stats.unknown_models.clone();
    collect_unknown_from_db(&conn, None, &mut unknown);
    for w in render_warnings(&unknown, stats.skipped_lines) {
        eprintln!("{w}");
    }
    drop(conn);
    crate::tui::run()
}

fn run_cursor(args: CursorArgs) -> Result<()> {
    match args.command {
        CursorCommands::Login(args) => run_cursor_login(args),
        CursorCommands::Status => run_cursor_status(),
        CursorCommands::Sync => run_cursor_sync(),
    }
}

fn run_cursor_login(args: CursorLoginArgs) -> Result<()> {
    let token = match args.token {
        Some(token) => token,
        None => prompt_cursor_token()?,
    };
    let result = validate_cursor_session(&token);
    if !result.valid {
        anyhow::bail!(
            "{}",
            result
                .error
                .unwrap_or_else(|| "Cursor session is invalid".to_owned())
        );
    }
    let account_id = save_credentials(&token, args.label.as_deref())?;
    println!("Saved Cursor account {account_id}");
    Ok(())
}

fn run_cursor_status() -> Result<()> {
    let accounts = list_accounts();
    if accounts.is_empty() {
        println!("No saved Cursor accounts.");
        return Ok(());
    }
    let validation = validate_active_account();
    for account in accounts {
        let marker = if account.is_active { "*" } else { " " };
        println!(
            "{marker} {}{}{}",
            account.id,
            account
                .label
                .as_deref()
                .map(|label| format!(" ({label})"))
                .unwrap_or_default(),
            account
                .user_id
                .as_deref()
                .map(|user| format!(" user:{user}"))
                .unwrap_or_default(),
        );
        println!("  created: {}", account.created_at);
        if account.is_active {
            if let Some(result) = &validation {
                if result.valid {
                    println!("  session: valid");
                } else if let Some(error) = &result.error {
                    println!("  session: invalid ({error})");
                }
            }
        }
    }
    Ok(())
}

fn run_cursor_sync() -> Result<()> {
    let result = sync_active_account(Some(&cursor_sync_cache_dir()));
    if !result.synced {
        anyhow::bail!(
            "{}",
            result
                .error
                .unwrap_or_else(|| "Cursor sync failed".to_owned())
        );
    }
    println!(
        "Synced Cursor usage to {} ({} rows)",
        result.path.display(),
        result.rows
    );
    Ok(())
}

fn prompt_cursor_token() -> Result<String> {
    eprint!("Cursor session token: ");
    io::stderr().flush().ok();
    let mut token = String::new();
    io::stdin()
        .read_line(&mut token)
        .context("reading Cursor session token")?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!("Cursor session token cannot be empty");
    }
    Ok(token)
}

fn maybe_sync_cursor(target_dir: Option<&std::path::Path>) {
    if !has_configured_account() {
        return;
    }
    let result = sync_active_account(target_dir);
    if !result.synced {
        if let Some(error) = result.error {
            eprintln!("warning: Cursor sync failed: {error}");
        }
    }
}

fn cursor_sync_target_dir(cursor_dir: Option<&str>) -> PathBuf {
    let env = |k: &str| std::env::var(k).ok();
    let resolved = resolve_roots(ResolveInput {
        flag: cursor_dir,
        tokctl_env: env("TOKCTL_CURSOR_DIR").as_deref(),
        tool_env: None,
        tool_env_suffix: None,
        defaults: default_cursor_roots(),
    });
    resolved
        .roots
        .into_iter()
        .next()
        .unwrap_or_else(cursor_sync_cache_dir)
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
    cursor_dir: Option<&str>,
) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>) {
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
    let cursor = resolve_roots(ResolveInput {
        flag: cursor_dir,
        tokctl_env: env("TOKCTL_CURSOR_DIR").as_deref(),
        tool_env: None,
        tool_env_suffix: None,
        defaults: default_cursor_roots(),
    });
    (
        existing_dirs(claude),
        existing_dirs(codex),
        existing_dirs(cursor),
    )
}

fn existing_dirs(resolved: crate::paths::ResolvedRoots) -> Vec<PathBuf> {
    resolved.roots.into_iter().filter(|p| p.is_dir()).collect()
}

fn run_report_cached(group: GroupBy, args: ReportArgs) -> Result<()> {
    let source = SourceArg::parse(&args.source)?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;

    if source.include_cursor() {
        let target = cursor_sync_target_dir(args.cursor_dir.as_deref());
        maybe_sync_cursor(Some(&target));
    }

    let cache_path = store_path();
    if args.rebuild {
        std::fs::remove_file(&cache_path).ok();
    }

    let (claude_roots, codex_roots, cursor_roots) = resolve_all_roots(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );

    let mut conn = open_store(&cache_path)
        .with_context(|| format!("opening cache at {}", cache_path.display()))?;

    let stats = run_ingest(RunIngestOptions {
        conn: &mut conn,
        claude_roots,
        codex_roots,
        cursor_roots,
        include_claude: source.include_claude(),
        include_codex: source.include_codex(),
        include_cursor: source.include_cursor(),
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
        Some(_) => "SELECT DISTINCT model, source FROM events WHERE source = ?1",
        None => "SELECT DISTINCT model, source FROM events",
    };
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return,
    };
    let iter: Result<Vec<(String, String)>, _> = match source {
        Some(s) => stmt
            .query_map([s.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rs| rs.collect()),
        None => stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .and_then(|rs| rs.collect()),
    };
    if let Ok(models) = iter {
        for (m, src) in models {
            if src == Source::Cursor.as_str() {
                continue;
            }
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

    if source.include_cursor() {
        let target = cursor_sync_target_dir(args.cursor_dir.as_deref());
        maybe_sync_cursor(Some(&target));
    }

    let (claude_roots, codex_roots, cursor_roots) = resolve_all_roots(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );
    let (events, skipped_lines) =
        gather_events_no_cache(source, &claude_roots, &codex_roots, &cursor_roots)?;
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
    cursor_roots: &[PathBuf],
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
        for f in &d.files {
            let r = ingest_claude_range(&f.path, f.project.as_deref(), 0, f.size)?;
            skipped_lines += r.skipped_lines;
            events.extend(r.events);
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
    if source.include_cursor() {
        let d = discover_cursor(cursor_roots, &empty_manifest, discover_opts);
        for f in &d.files {
            let r = ingest_cursor_range(&f.path)?;
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

    if source.include_cursor() {
        let target = cursor_sync_target_dir(args.cursor_dir.as_deref());
        maybe_sync_cursor(Some(&target));
    }

    let (claude_roots, codex_roots, cursor_roots) = resolve_all_roots(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );

    if args.no_cache {
        let (events, skipped_lines) =
            gather_events_no_cache(source, &claude_roots, &codex_roots, &cursor_roots)?;
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
        cursor_roots,
        include_claude: source.include_claude(),
        include_codex: source.include_codex(),
        include_cursor: source.include_cursor(),
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
