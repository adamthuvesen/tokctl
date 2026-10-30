use anyhow::{Context, Result};
use std::collections::HashSet;
use std::io::{self, Write};

use super::pipeline::{
    finish_cached_warnings, maybe_sync_cursor, prepare_run, query_scope, resolve_root_dirs,
    sync_cursor_if_needed, CachedRun, SourceScope,
};
use super::{
    CompareArgs, CursorArgs, CursorCommands, CursorLoginArgs, DemoArgs, DoctorArgs, GroupBy,
    RepoArgs, ReportArgs, SourceArg, UiArgs,
};
use crate::compare;
use crate::cursor_sync::{
    list_accounts, save_credentials, sync_active_account, validate_active_account,
    validate_cursor_session,
};
use crate::dates::{parse_since, parse_until};
use crate::paths::cursor_sync_cache_dir;
use crate::render::{
    render_json, render_repo_json, render_repo_table, render_table, render_warnings,
};
use crate::store::queries::{
    daily_report, monthly_report, repo_report,
    resolve_repo_filter_scoped as resolve_repo_filter_cached, session_report, QueryFilter,
};
use crate::types::{AggregateRow, ReportKind};

pub(super) fn run_doctor(args: DoctorArgs) -> Result<()> {
    let report = crate::doctor::run();
    let body = if args.json {
        crate::doctor::render_json(&report)
    } else {
        crate::doctor::render_human(&report)
    };
    writeln!(io::stdout().lock(), "{body}")?;
    if args.strict && report.status != crate::doctor::CheckSeverity::Ok {
        anyhow::bail!("doctor found {}", report.status.as_str());
    }
    Ok(())
}

pub(super) fn run_report(kind: ReportKind, args: ReportArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }

    let group = match args.group_by.as_deref() {
        None => default_group_for(kind),
        Some(s) => GroupBy::parse(s)?,
    };

    let source = SourceArg::parse(&args.source)?;
    let scope = source_scope(source);
    validate_date_range(args.since.as_deref(), args.until.as_deref())?;
    sync_cursor_if_needed(scope, args.cursor_dir.as_deref())?;
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );

    run_report_cached(group, source, scope, roots, args)
}

pub(super) fn run_compare(args: CompareArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }
    if args.top == 0 {
        anyhow::bail!("--top must be >= 1");
    }

    let source = SourceArg::parse(&args.source)?;
    let scope = source_scope(source);
    let windows = compare::resolve_windows(
        args.current.as_deref(),
        args.baseline.as_deref(),
        chrono::Utc::now(),
    )?;
    let dimensions = args.by.dimensions();
    sync_cursor_if_needed(scope, args.cursor_dir.as_deref())?;
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );

    let CachedRun { conn, stats } = prepare_run(scope, &roots, args.rebuild, args.no_cache)?;
    let mut filter = query_scope(scope, &roots);
    filter.repo = args
        .repo
        .as_deref()
        .map(|name| resolve_repo_filter_cached(&conn, name, &query_scope(scope, &roots)))
        .transpose()?;
    let report = compare::compare_from_db(&conn, windows, filter.clone(), &dimensions, args.top)?;
    let (unknown, skipped, errors) = finish_cached_warnings(&conn, &stats, filter)?;
    emit_warnings(&unknown, skipped, errors);

    let body = if args.json {
        compare::render_json(&report)
    } else {
        compare::render_human(&report)
    };
    writeln!(io::stdout().lock(), "{body}")?;
    Ok(())
}

pub(super) fn run_ui(args: UiArgs) -> Result<()> {
    crate::tui::require_terminal()?;
    if args.demo {
        let result = crate::demo::seed_demo_cache(&args.cache_dir, args.overwrite)?;
        return crate::tui::run_with_store_path(&result.cache_path, None);
    }

    maybe_sync_cursor(None)?;
    let roots = resolve_root_dirs(None, None, None);
    let scope = SourceScope { filter: None };
    let CachedRun { conn, stats } = prepare_run(scope, &roots, false, false)?;
    let (unknown, skipped, errors) =
        finish_cached_warnings(&conn, &stats, query_scope(scope, &roots))?;
    emit_warnings(&unknown, skipped, errors);
    drop(conn);
    crate::tui::run_with_store_path(
        &crate::store::store_path(),
        query_scope(scope, &roots).roots,
    )
}

pub(super) fn run_demo(args: DemoArgs) -> Result<()> {
    let result = crate::demo::seed_demo_cache(&args.cache_dir, args.overwrite)?;
    writeln!(
        io::stdout().lock(),
        "Seeded {} synthetic events across {} repos into {}",
        result.events,
        result.repos,
        result.cache_path.display()
    )?;
    writeln!(
        io::stdout().lock(),
        "Run: tokctl ui --demo --cache-dir {} --overwrite",
        result.cache_dir.display()
    )?;
    Ok(())
}

pub(super) fn run_cursor(args: CursorArgs) -> Result<()> {
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
    validate_cursor_session(&token)?;
    let account_id = save_credentials(&token, args.label.as_deref())?;
    writeln!(io::stdout().lock(), "Saved Cursor account {account_id}")?;
    Ok(())
}

fn run_cursor_status() -> Result<()> {
    let accounts = list_accounts()?;
    if accounts.is_empty() {
        writeln!(io::stdout().lock(), "No saved Cursor accounts.")?;
        return Ok(());
    }
    let validation = validate_active_account()?;
    for account in accounts {
        let marker = if account.is_active { "*" } else { " " };
        writeln!(
            io::stdout().lock(),
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
        )?;
        writeln!(io::stdout().lock(), "  created: {}", account.created_at)?;
        if account.is_active && validation.is_some() {
            writeln!(io::stdout().lock(), "  session: valid")?;
        }
    }
    Ok(())
}

fn run_cursor_sync() -> Result<()> {
    let result = sync_active_account(Some(&cursor_sync_cache_dir()))?;
    writeln!(
        io::stdout().lock(),
        "Synced Cursor usage to {} ({} rows)",
        result.path.display(),
        result.rows
    )?;
    Ok(())
}

fn prompt_cursor_token() -> Result<String> {
    eprint!("Cursor session token: ");
    io::stderr().flush()?;
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

pub(super) fn run_repo(args: RepoArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }
    let source = SourceArg::parse(&args.source)?;
    let scope = source_scope(source);
    validate_date_range(args.since.as_deref(), args.until.as_deref())?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    sync_cursor_if_needed(scope, args.cursor_dir.as_deref())?;
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );

    let CachedRun { conn, stats } = prepare_run(scope, &roots, args.rebuild, args.no_cache)?;
    let mut filter = QueryFilter {
        ..query_scope(scope, &roots)
    };
    filter.since_ms = since.map(|t| t.timestamp_millis());
    filter.until_ms = until.map(|t| t.timestamp_millis());

    match args.name.as_deref() {
        None => {
            let rows = repo_report(&conn, filter)?;
            emit_repo(&rows, args.json)?;
        }
        Some(name) => {
            filter.repo = Some(resolve_repo_filter_cached(
                &conn,
                name,
                &query_scope(scope, &roots),
            )?);
            let rows = session_report(&conn, filter)?;
            emit(&rows, ReportKind::Session, source, args.json)?;
        }
    }

    let (unknown, skipped, errors) =
        finish_cached_warnings(&conn, &stats, query_scope(scope, &roots))?;
    emit_warnings(&unknown, skipped, errors);
    Ok(())
}

fn source_scope(source: SourceArg) -> SourceScope {
    SourceScope {
        filter: source.to_filter(),
    }
}

fn default_group_for(kind: ReportKind) -> GroupBy {
    match kind {
        ReportKind::Daily => GroupBy::Day,
        ReportKind::Monthly => GroupBy::Month,
        ReportKind::Session => GroupBy::Session,
    }
}

fn run_report_cached(
    group: GroupBy,
    source: SourceArg,
    scope: SourceScope,
    roots: super::pipeline::RootDirs,
    args: ReportArgs,
) -> Result<()> {
    validate_date_range(args.since.as_deref(), args.until.as_deref())?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    let CachedRun { conn, stats } = prepare_run(scope, &roots, args.rebuild, args.no_cache)?;

    let repo_spec = match args.repo.as_deref() {
        Some(name) => Some(resolve_repo_filter_cached(
            &conn,
            name,
            &query_scope(scope, &roots),
        )?),
        None => None,
    };

    let filter = QueryFilter {
        since_ms: since.map(|t| t.timestamp_millis()),
        until_ms: until.map(|t| t.timestamp_millis()),
        repo: repo_spec,
        ..query_scope(scope, &roots)
    };

    match group {
        GroupBy::Day => {
            let rows = daily_report(&conn, filter)?;
            emit(&rows, ReportKind::Daily, source, args.json)?;
        }
        GroupBy::Month => {
            let rows = monthly_report(&conn, filter)?;
            emit(&rows, ReportKind::Monthly, source, args.json)?;
        }
        GroupBy::Session => {
            let rows = session_report(&conn, filter)?;
            emit(&rows, ReportKind::Session, source, args.json)?;
        }
        GroupBy::Repo => {
            let rows = repo_report(&conn, filter)?;
            emit_repo(&rows, args.json)?;
        }
    }

    let (unknown, skipped, errors) =
        finish_cached_warnings(&conn, &stats, query_scope(scope, &roots))?;
    emit_warnings(&unknown, skipped, errors);
    Ok(())
}

fn emit(rows: &[AggregateRow], kind: ReportKind, source: SourceArg, as_json: bool) -> Result<()> {
    let show_source = source.show_source();
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let body = if as_json {
        render_json(rows, kind, show_source)
    } else {
        render_table(rows, kind, show_source)
    };
    writeln!(lock, "{body}")?;
    Ok(())
}

fn emit_repo(rows: &[crate::store::queries::RepoAggregateRow], as_json: bool) -> Result<()> {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let body = if as_json {
        render_repo_json(rows)
    } else {
        render_repo_table(rows)
    };
    writeln!(lock, "{body}")?;
    Ok(())
}

fn emit_warnings(unknown: &HashSet<String>, skipped_lines: usize, file_errors: usize) {
    for warning in render_warnings(unknown, skipped_lines, file_errors) {
        eprintln!("{warning}");
    }
}

fn validate_date_range(since: Option<&str>, until: Option<&str>) -> Result<()> {
    let since = parse_since(since)?;
    let until = parse_until(until)?;
    if let (Some(since), Some(until)) = (since, until) {
        anyhow::ensure!(since < until, "--until must be on or after --since");
    }
    Ok(())
}
