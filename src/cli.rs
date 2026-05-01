use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use std::process::ExitCode;

use crate::compare::CompareDimension;
use crate::store::store_path;
use crate::types::{ReportKind, Source, SourceLabel};

mod workflows;

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
        Commands::Daily(a) => workflows::run_report(ReportKind::Daily, a),
        Commands::Monthly(a) => workflows::run_report(ReportKind::Monthly, a),
        Commands::Session(a) => workflows::run_report(ReportKind::Session, a),
        Commands::Repo(a) => workflows::run_repo(a),
        Commands::Doctor(a) => workflows::run_doctor(a),
        Commands::Compare(a) => workflows::run_compare(a),
        Commands::ExportDb => {
            println!("{}", store_path().display());
            Ok(())
        }
        Commands::Cursor(a) => workflows::run_cursor(a),
        Commands::Ui => workflows::run_ui(),
    }
}

pub fn main_exit() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
