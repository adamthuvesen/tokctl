use anyhow::{Context, Result};
use std::collections::HashSet;
use std::io::{self, Write};

use super::pipeline::{
    finish_cached_warnings, maybe_sync_cursor, prepare_cached, prepare_no_cache, resolve_root_dirs,
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
use crate::reports::in_memory::{
    daily_in_memory, filter_by_repo, monthly_in_memory, repo_in_memory,
    resolve_repo_filter as resolve_repo_filter_in_memory, session_in_memory,
};
use crate::store::queries::{
    daily_report, monthly_report, repo_report, resolve_repo_filter as resolve_repo_filter_cached,
    session_report, QueryFilter,
};
use crate::types::{AggregateRow, ReportKind};

pub(super) fn run_doctor(args: DoctorArgs) -> Result<()> {
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
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );
    sync_cursor_if_needed(scope, args.cursor_dir.as_deref());

    if args.no_cache {
        run_report_no_cache(group, source, scope, roots, args)
    } else {
        run_report_cached(group, source, scope, roots, args)
    }
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
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );
    sync_cursor_if_needed(scope, args.cursor_dir.as_deref());

    let report = if args.no_cache {
        let run = prepare_no_cache(scope, &roots)?;
        let repo_spec = match args.repo.as_deref() {
            Some(name) => Some(resolve_repo_filter_in_memory(&run.repo_catalog, name)?),
            None => None,
        };
        let mut unknown = HashSet::new();
        let report = compare::compare_from_events(
            &run.resolved,
            windows,
            &repo_spec,
            &dimensions,
            args.top,
            &mut unknown,
        );
        emit_warnings(&unknown, run.skipped_lines, run.file_errors);
        report
    } else {
        let CachedRun { conn, stats } = prepare_cached(scope, &roots, args.rebuild)?;
        let repo_spec = match args.repo.as_deref() {
            Some(name) => Some(resolve_repo_filter_cached(&conn, name)?),
            None => None,
        };
        let filter = QueryFilter {
            source: scope.filter,
            since_ms: None,
            until_ms: None,
            repo: repo_spec,
        };
        let report = compare::compare_from_db(&conn, windows, filter, &dimensions, args.top)?;
        let (unknown, skipped, errors) = finish_cached_warnings(&conn, &stats, scope.filter);
        emit_warnings(&unknown, skipped, errors);
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

pub(super) fn run_ui(args: UiArgs) -> Result<()> {
    if args.demo {
        let result = crate::demo::seed_demo_cache(&args.cache_dir, args.overwrite)?;
        // SAFETY: tokctl is still single-threaded here; the TUI reads this
        // process-local override immediately when resolving store_path().
        std::env::set_var("TOKCTL_CACHE_DIR", &result.cache_dir);
        return crate::tui::run();
    }

    maybe_sync_cursor(None);
    let roots = resolve_root_dirs(None, None, None);
    let scope = SourceScope {
        filter: None,
        include_claude: true,
        include_codex: true,
        include_cursor: true,
    };
    let CachedRun { conn, stats } = prepare_cached(scope, &roots, false)?;
    let (unknown, skipped, errors) = finish_cached_warnings(&conn, &stats, None);
    emit_warnings(&unknown, skipped, errors);
    drop(conn);
    crate::tui::run()
}

pub(super) fn run_demo(args: DemoArgs) -> Result<()> {
    let result = crate::demo::seed_demo_cache(&args.cache_dir, args.overwrite)?;
    println!(
        "Seeded {} synthetic events across {} repos into {}",
        result.events,
        result.repos,
        result.cache_path.display()
    );
    println!(
        "Run: tokctl ui --demo --cache-dir {} --overwrite",
        result.cache_dir.display()
    );
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

pub(super) fn run_repo(args: RepoArgs) -> Result<()> {
    if args.rebuild && args.no_cache {
        anyhow::bail!("--rebuild and --no-cache are mutually exclusive");
    }
    let source = SourceArg::parse(&args.source)?;
    let scope = source_scope(source);
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    );
    sync_cursor_if_needed(scope, args.cursor_dir.as_deref());

    if args.no_cache {
        let run = prepare_no_cache(scope, &roots)?.with_date_filter(since, until);
        let mut unknown = HashSet::new();
        match args.name.as_deref() {
            None => {
                let rows = repo_in_memory(&run.resolved, &None, &mut unknown);
                emit_repo(&rows, args.json);
            }
            Some(name) => {
                let spec = Some(resolve_repo_filter_in_memory(&run.repo_catalog, name)?);
                let only = filter_by_repo(&run.resolved, &spec);
                let rows = session_in_memory(&only, &mut unknown);
                emit(&rows, ReportKind::Session, source, args.json);
            }
        }
        emit_warnings(&unknown, run.skipped_lines, run.file_errors);
        return Ok(());
    }

    let CachedRun { conn, stats } = prepare_cached(scope, &roots, args.rebuild)?;
    let mut filter = QueryFilter {
        source: scope.filter,
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
            filter.repo = Some(resolve_repo_filter_cached(&conn, name)?);
            let rows = session_report(&conn, filter)?;
            emit(&rows, ReportKind::Session, source, args.json);
        }
    }

    let (unknown, skipped, errors) = finish_cached_warnings(&conn, &stats, scope.filter);
    emit_warnings(&unknown, skipped, errors);
    Ok(())
}

fn source_scope(source: SourceArg) -> SourceScope {
    SourceScope {
        filter: source.to_filter(),
        include_claude: source.include_claude(),
        include_codex: source.include_codex(),
        include_cursor: source.include_cursor(),
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
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    let CachedRun { conn, stats } = prepare_cached(scope, &roots, args.rebuild)?;

    let repo_spec = match args.repo.as_deref() {
        Some(name) => Some(resolve_repo_filter_cached(&conn, name)?),
        None => None,
    };

    let filter = QueryFilter {
        source: scope.filter,
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

    let (unknown, skipped, errors) = finish_cached_warnings(&conn, &stats, scope.filter);
    emit_warnings(&unknown, skipped, errors);
    Ok(())
}

fn run_report_no_cache(
    group: GroupBy,
    source: SourceArg,
    scope: SourceScope,
    roots: super::pipeline::RootDirs,
    args: ReportArgs,
) -> Result<()> {
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    let run = prepare_no_cache(scope, &roots)?.with_date_filter(since, until);

    let repo_spec = match args.repo.as_deref() {
        Some(name) => Some(resolve_repo_filter_in_memory(&run.repo_catalog, name)?),
        None => None,
    };

    let mut unknown = HashSet::new();
    match group {
        GroupBy::Day => {
            let filtered = filter_by_repo(&run.resolved, &repo_spec);
            let rows = daily_in_memory(&filtered, source.label(), &mut unknown);
            emit(&rows, ReportKind::Daily, source, args.json);
        }
        GroupBy::Month => {
            let filtered = filter_by_repo(&run.resolved, &repo_spec);
            let rows = monthly_in_memory(&filtered, source.label(), &mut unknown);
            emit(&rows, ReportKind::Monthly, source, args.json);
        }
        GroupBy::Session => {
            let filtered = filter_by_repo(&run.resolved, &repo_spec);
            let rows = session_in_memory(&filtered, &mut unknown);
            emit(&rows, ReportKind::Session, source, args.json);
        }
        GroupBy::Repo => {
            let rows = repo_in_memory(&run.resolved, &repo_spec, &mut unknown);
            emit_repo(&rows, args.json);
        }
    }

    emit_warnings(&unknown, run.skipped_lines, run.file_errors);
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

fn emit_warnings(unknown: &HashSet<String>, skipped_lines: usize, file_errors: usize) {
    for warning in render_warnings(unknown, skipped_lines, file_errors) {
        eprintln!("{warning}");
    }
}
