use anyhow::{Context, Result};
use std::io::{self, Write};

use super::pipeline::{
    cursor_sync_target_dir, prepare_run, query_scope, resolve_root_dirs, sync_cursor_if_needed,
    CachedRun, SourceScope,
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
use crate::render::{
    render_json, render_repo_json, render_repo_table, render_table, render_warnings,
};
use crate::store::queries::{
    daily_report, monthly_report, repo_report, report_diagnostics, report_diagnostics_for_scopes,
    resolve_repo_filter_scoped as resolve_repo_filter_cached, session_report, QueryFilter,
    ReportDiagnostics,
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
    if let Some(warning) = sync_cursor_if_needed(scope, args.cursor_dir.as_deref())? {
        eprintln!("warning: {warning}");
    }
    let roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    )?;

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
    if let Some(warning) = sync_cursor_if_needed(scope, args.cursor_dir.as_deref())? {
        eprintln!("warning: {warning}");
    }
    let mut roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    )?;

    let CachedRun { conn, stats } = prepare_run(scope, &mut roots, args.rebuild, args.no_cache)?;
    let (body, diagnostics) = conn.read_snapshot(|conn| {
        let mut filter = query_scope(scope, &roots);
        filter.repo = args
            .repo
            .as_deref()
            .map(|name| resolve_repo_filter_cached(conn, name, &filter))
            .transpose()?;
        let diagnostic_scopes = [&windows.current, &windows.baseline]
            .into_iter()
            .map(|window| {
                Ok(QueryFilter {
                    since_ms: parse_since(Some(&window.start))?.map(|date| date.timestamp_millis()),
                    until_ms: parse_until(Some(&window.end))?.map(|date| date.timestamp_millis()),
                    ..filter.clone()
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let report = compare::compare_from_db(conn, windows, filter, &dimensions, args.top)?;
        let mut diagnostics = report_diagnostics_for_scopes(conn, &diagnostic_scopes)?;
        diagnostics.current_file_errors = stats.file_errors;
        let body = if args.json {
            compare::render_json(&report)
        } else {
            compare::render_human(&report)
        };
        Ok((body, diagnostics))
    })?;
    emit_warnings(&diagnostics);
    writeln!(io::stdout().lock(), "{body}")?;
    Ok(())
}

pub(super) fn run_ui(args: UiArgs) -> Result<()> {
    crate::tui::require_terminal()?;
    if args.demo {
        let result = crate::demo::seed_demo_cache(&args.cache_dir, args.overwrite)?;
        return crate::tui::run_with_store_path(&result.cache_path, None);
    }

    let scope = SourceScope { filter: None };
    let cursor_sync_failure = sync_cursor_if_needed(scope, None)?;
    let mut roots = resolve_root_dirs(None, None, None)?;
    let CachedRun { conn, stats } = prepare_run(scope, &mut roots, false, false)?;
    drop(conn);
    crate::tui::run_with_store_path_and_warnings(
        &crate::store::store_path(),
        query_scope(scope, &roots).roots,
        stats.file_errors,
        cursor_sync_failure,
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
    let result = sync_active_account(Some(&cursor_sync_target_dir(None)))?;
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
    if let Some(warning) = sync_cursor_if_needed(scope, args.cursor_dir.as_deref())? {
        eprintln!("warning: {warning}");
    }
    let mut roots = resolve_root_dirs(
        args.claude_dir.as_deref(),
        args.codex_dir.as_deref(),
        args.cursor_dir.as_deref(),
    )?;

    let CachedRun { conn, stats } = prepare_run(scope, &mut roots, args.rebuild, args.no_cache)?;
    let (body, diagnostics) = conn.read_snapshot(|conn| {
        let mut filter = QueryFilter {
            since_ms: since.map(|date| date.timestamp_millis()),
            until_ms: until.map(|date| date.timestamp_millis()),
            ..query_scope(scope, &roots)
        };
        if let Some(name) = args.name.as_deref() {
            filter.repo = Some(resolve_repo_filter_cached(conn, name, &filter)?);
        }
        let body = if args.name.is_some() {
            let rows = session_report(conn, filter.clone())?;
            render_report(&rows, ReportKind::Session, source, args.json)
        } else {
            let rows = repo_report(conn, filter.clone())?;
            render_repos(&rows, args.json)
        };
        let mut diagnostics = report_diagnostics(conn, &filter)?;
        diagnostics.current_file_errors = stats.file_errors;
        Ok((body, diagnostics))
    })?;
    emit_warnings(&diagnostics);
    writeln!(io::stdout().lock(), "{body}")?;
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
    mut roots: super::pipeline::RootDirs,
    args: ReportArgs,
) -> Result<()> {
    validate_date_range(args.since.as_deref(), args.until.as_deref())?;
    let since = parse_since(args.since.as_deref())?;
    let until = parse_until(args.until.as_deref())?;
    let CachedRun { conn, stats } = prepare_run(scope, &mut roots, args.rebuild, args.no_cache)?;

    let (body, diagnostics) = conn.read_snapshot(|conn| {
        let mut filter = QueryFilter {
            since_ms: since.map(|date| date.timestamp_millis()),
            until_ms: until.map(|date| date.timestamp_millis()),
            ..query_scope(scope, &roots)
        };
        filter.repo = args
            .repo
            .as_deref()
            .map(|name| resolve_repo_filter_cached(conn, name, &filter))
            .transpose()?;
        let body = match group {
            GroupBy::Day => render_report(
                &daily_report(conn, filter.clone())?,
                ReportKind::Daily,
                source,
                args.json,
            ),
            GroupBy::Month => render_report(
                &monthly_report(conn, filter.clone())?,
                ReportKind::Monthly,
                source,
                args.json,
            ),
            GroupBy::Session => render_report(
                &session_report(conn, filter.clone())?,
                ReportKind::Session,
                source,
                args.json,
            ),
            GroupBy::Repo => render_repos(&repo_report(conn, filter.clone())?, args.json),
        };
        let mut diagnostics = report_diagnostics(conn, &filter)?;
        diagnostics.current_file_errors = stats.file_errors;
        Ok((body, diagnostics))
    })?;
    emit_warnings(&diagnostics);
    writeln!(io::stdout().lock(), "{body}")?;
    Ok(())
}

fn render_report(
    rows: &[AggregateRow],
    kind: ReportKind,
    source: SourceArg,
    as_json: bool,
) -> String {
    if as_json {
        render_json(rows, kind, source.show_source())
    } else {
        render_table(rows, kind, source.show_source())
    }
}

fn render_repos(rows: &[crate::store::queries::RepoAggregateRow], as_json: bool) -> String {
    if as_json {
        render_repo_json(rows)
    } else {
        render_repo_table(rows)
    }
}

fn emit_warnings(diagnostics: &ReportDiagnostics) {
    for warning in render_warnings(
        &diagnostics.unknown_models,
        diagnostics.skipped_records,
        diagnostics.current_file_errors,
    ) {
        eprintln!("{warning}");
    }
    if diagnostics.legacy_cost_rows > 0 {
        eprintln!(
            "warning: retained {} legacy cost row(s) without recoverable cost provenance; restore source roots to refresh",
            diagnostics.legacy_cost_rows
        );
    }
    if diagnostics.stale_pricing_rows > 0 {
        eprintln!(
            "warning: retained {} cost row(s) with stale pricing; restore source roots to refresh",
            diagnostics.stale_pricing_rows
        );
    }
    if diagnostics.stale_parser_files > 0 {
        eprintln!(
            "warning: retained {} file snapshot(s) from a stale parser; restore source roots to refresh",
            diagnostics.stale_parser_files
        );
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
