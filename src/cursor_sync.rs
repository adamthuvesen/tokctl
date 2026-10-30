use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::paths::{
    cursor_credentials_path, cursor_sync_cache_dir, tokscale_cursor_credentials_path,
};

const USAGE_CSV_ENDPOINT: &str =
    "https://cursor.com/api/dashboard/export-usage-events-csv?strategy=tokens";
const USAGE_SUMMARY_ENDPOINT: &str = "https://cursor.com/api/usage-summary";

#[derive(Clone, Serialize, Deserialize)]
pub struct CursorCredentials {
    #[serde(rename = "sessionToken")]
    pub session_token: String,
    #[serde(rename = "userId", skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CursorCredentialsStore {
    pub version: i32,
    #[serde(rename = "activeAccountId")]
    pub active_account_id: String,
    pub accounts: HashMap<String, CursorCredentials>,
}

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub id: String,
    pub label: Option<String>,
    pub user_id: Option<String>,
    pub created_at: String,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct SyncCursorResult {
    pub rows: usize,
    pub path: PathBuf,
}

pub trait CursorApi {
    fn validate_session(&self, token: &str) -> Result<()>;
    fn fetch_usage_csv(&self, token: &str) -> Result<String>;
}

pub struct BlockingCursorApi {
    client: reqwest::blocking::Client,
}

impl BlockingCursorApi {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .context("building Cursor HTTP client")?;
        Ok(Self { client })
    }
}

impl CursorApi for BlockingCursorApi {
    fn validate_session(&self, token: &str) -> Result<()> {
        let response = self
            .client
            .get(USAGE_SUMMARY_ENDPOINT)
            .headers(build_cursor_headers(token)?)
            .send()
            .context("validating Cursor session")?;
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            anyhow::bail!("Cursor session is invalid or expired");
        }
        if !response.status().is_success() {
            anyhow::bail!("Cursor API returned status {}", response.status());
        }
        let summary: serde_json::Value = response
            .json()
            .map_err(|_| anyhow!("Cursor returned an invalid usage summary response"))?;
        if !summary.is_object() {
            anyhow::bail!("Cursor returned an invalid usage summary response");
        }
        Ok(())
    }

    fn fetch_usage_csv(&self, token: &str) -> Result<String> {
        let response = self
            .client
            .get(USAGE_CSV_ENDPOINT)
            .headers(build_cursor_headers(token)?)
            .send()
            .context("fetching Cursor usage CSV")?;
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            anyhow::bail!("Cursor session is invalid or expired");
        }
        if !response.status().is_success() {
            anyhow::bail!("Cursor API returned status {}", response.status());
        }
        let text = response.text().context("reading Cursor usage CSV body")?;
        Ok(text)
    }
}

pub fn validate_cursor_session(token: &str) -> Result<()> {
    build_cursor_headers(token)?;
    BlockingCursorApi::new()?.validate_session(token)
}

pub fn validate_active_account() -> Result<Option<()>> {
    let Some((_, credentials)) = load_active_credentials()? else {
        return Ok(None);
    };
    validate_cursor_session(&credentials.session_token).map(Some)
}

pub fn has_configured_account() -> Result<bool> {
    Ok(load_credentials_store()?.is_some_and(|s| !s.accounts.is_empty()))
}

pub fn list_accounts() -> Result<Vec<AccountInfo>> {
    let Some(store) = load_credentials_store()? else {
        return Ok(Vec::new());
    };
    let mut out: Vec<AccountInfo> = store
        .accounts
        .iter()
        .map(|(id, account)| AccountInfo {
            id: id.clone(),
            label: account.label.clone(),
            user_id: account.user_id.clone(),
            created_at: account.created_at.clone(),
            is_active: id == &store.active_account_id,
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

pub fn load_credentials_store() -> Result<Option<CursorCredentialsStore>> {
    load_credentials_from_paths(
        &cursor_credentials_path(),
        &tokscale_cursor_credentials_path(),
    )
}

fn load_credentials_from_paths(
    primary: &Path,
    legacy: &Path,
) -> Result<Option<CursorCredentialsStore>> {
    for path in [primary, legacy] {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => anyhow::bail!("could not read Cursor credentials at {}", path.display()),
        };
        let store = serde_json::from_str::<CursorCredentialsStore>(&raw)
            .ok()
            .or_else(|| {
                serde_json::from_str::<CursorCredentials>(&raw)
                    .ok()
                    .map(single_account_store)
            })
            .ok_or_else(|| anyhow!("invalid Cursor credentials at {}", path.display()))?;
        if store.version != 1
            || (!store.accounts.is_empty()
                && !store.accounts.contains_key(&store.active_account_id))
        {
            anyhow::bail!("invalid Cursor account configuration at {}", path.display());
        }
        for account in store.accounts.values() {
            build_cursor_headers(&account.session_token)
                .context("invalid token in Cursor account configuration")?;
        }
        return Ok(Some(store));
    }
    Ok(None)
}

pub fn load_active_credentials() -> Result<Option<(String, CursorCredentials)>> {
    let Some(store) = load_credentials_store()? else {
        return Ok(None);
    };
    let id = store.active_account_id;
    Ok(store
        .accounts
        .get(&id)
        .cloned()
        .map(|account| (id, account)))
}

pub fn save_credentials(token: &str, label: Option<&str>) -> Result<String> {
    let token = token.trim();
    build_cursor_headers(token)?;
    let user_id = extract_user_id_from_session_token(token);
    let account_id = derive_account_id(token);
    let mut store = load_credentials_store()?.unwrap_or(CursorCredentialsStore {
        version: 1,
        active_account_id: account_id.clone(),
        accounts: HashMap::new(),
    });
    let credentials = CursorCredentials {
        session_token: token.to_owned(),
        user_id,
        created_at: Utc::now().to_rfc3339(),
        expires_at: None,
        label: label.map(str::to_owned),
    };
    store.accounts.insert(account_id.clone(), credentials);
    store.active_account_id = account_id.clone();
    save_credentials_store(&store)?;
    Ok(account_id)
}

pub fn save_credentials_store(store: &CursorCredentialsStore) -> Result<()> {
    let path = cursor_credentials_path();
    let contents = serde_json::to_string_pretty(store)?;
    atomic_write_file(&path, contents.as_bytes())
}

pub fn sync_active_account(target_dir: Option<&Path>) -> Result<SyncCursorResult> {
    let path = target_dir
        .map(PathBuf::from)
        .unwrap_or_else(cursor_sync_cache_dir);
    let (_, credentials) =
        load_active_credentials()?.ok_or_else(|| anyhow!("No configured Cursor account"))?;
    let api = BlockingCursorApi::new()?;
    sync_active_account_with_api(&api, &credentials, &path)
}

fn sync_active_account_with_api(
    api: &impl CursorApi,
    credentials: &CursorCredentials,
    target_dir: &Path,
) -> Result<SyncCursorResult> {
    build_cursor_headers(&credentials.session_token)?;
    let csv_path = target_dir.join("usage.csv");
    let csv = api.fetch_usage_csv(&credentials.session_token)?;
    let (_, malformed) = crate::sources::cursor::parse_cursor_reader(csv.as_bytes(), &csv_path)
        .context("validating Cursor usage CSV before replacing cache")?;
    if malformed > 0 {
        anyhow::bail!("Cursor usage CSV contains {malformed} malformed record(s)");
    }
    let rows = csv::Reader::from_reader(csv.as_bytes())
        .records()
        .try_fold(0usize, |count, record| -> Result<usize> {
            let record = record.context("reading Cursor usage CSV record")?;
            Ok(count + usize::from(record.iter().any(|field| !field.trim().is_empty())))
        })?;
    atomic_write_file(&csv_path, csv.as_bytes())?;
    Ok(SyncCursorResult {
        rows,
        path: csv_path,
    })
}

fn single_account_store(account: CursorCredentials) -> CursorCredentialsStore {
    let account_id = account
        .user_id
        .clone()
        .unwrap_or_else(|| derive_account_id(&account.session_token));
    let mut accounts = HashMap::new();
    accounts.insert(account_id.clone(), account);
    CursorCredentialsStore {
        version: 1,
        active_account_id: account_id,
        accounts,
    }
}

pub(crate) fn build_cursor_headers(session_token: &str) -> Result<reqwest::header::HeaderMap> {
    use reqwest::header::{HeaderMap, HeaderValue};

    if session_token.is_empty()
        || session_token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control() || byte == b';')
    {
        anyhow::bail!("Cursor session token is empty or contains invalid cookie characters");
    }
    let mut cookie = HeaderValue::from_str(&format!("WorkosCursorSessionToken={session_token}"))
        .map_err(|_| anyhow!("Cursor session token contains invalid header characters"))?;
    cookie.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    headers.insert("Cookie", cookie);
    headers.insert(
        "Referer",
        HeaderValue::from_static("https://www.cursor.com/settings"),
    );
    headers.insert(
        "User-Agent",
        HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        ),
    );
    Ok(headers)
}

fn extract_user_id_from_session_token(token: &str) -> Option<String> {
    for sep in ["%3A%3A", "::"] {
        if token.contains(sep) {
            let user_id = token.split(sep).next()?.trim();
            if !user_id.is_empty() {
                return Some(user_id.to_owned());
            }
        }
    }
    None
}

fn derive_account_id(token: &str) -> String {
    if let Some(user_id) = extract_user_id_from_session_token(token) {
        return user_id;
    }
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash = hasher.finalize();
    let hex = format!("{hash:x}");
    format!("anon-{}", &hex[..12])
}

fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))
}

fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid Cursor cache path"))?;
    ensure_dir(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .context("creating private Cursor temporary file")?;
    set_private_permissions(temp.path())?;
    temp.write_all(contents)
        .context("writing Cursor temporary file")?;
    temp.as_file()
        .sync_all()
        .context("syncing Cursor temporary file")?;
    temp.persist(path)
        .map_err(|err| anyhow!("replacing Cursor file atomically: {}", err.error))?;
    fs::File::open(parent)?
        .sync_all()
        .context("syncing Cursor directory")?;
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use tempfile::tempdir;

    const HEADER: &str =
        "Date,Model,Input (w/ Cache Write),Input (w/o Cache Write),Cache Read,Output Tokens,Cost\n";

    struct FakeApi {
        csv: Result<String>,
        calls: Cell<usize>,
    }

    impl CursorApi for FakeApi {
        fn validate_session(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn fetch_usage_csv(&self, _: &str) -> Result<String> {
            self.calls.set(self.calls.get() + 1);
            self.csv
                .as_ref()
                .map(Clone::clone)
                .map_err(|_| anyhow!("transport failed"))
        }
    }

    fn credentials(token: &str) -> CursorCredentials {
        CursorCredentials {
            session_token: token.to_owned(),
            user_id: None,
            created_at: "2026-04-24T00:00:00Z".into(),
            expires_at: None,
            label: None,
        }
    }

    #[test]
    fn sync_counts_records_with_multiline_fields() {
        let dir = tempdir().unwrap();
        let api = FakeApi {
            csv: Ok(format!(
                "{HEADER}2026-04-24,\"model\nname\",10,5,0,1,0.01\n2026-04-24,zero,0,0,0,0,0\n"
            )),
            calls: Cell::new(0),
        };
        let result = sync_active_account_with_api(&api, &credentials("token"), dir.path()).unwrap();
        assert_eq!(result.rows, 2);
        assert_eq!(
            fs::read_to_string(result.path).unwrap(),
            *api.csv.as_ref().unwrap()
        );
    }

    #[test]
    fn failed_responses_preserve_previous_cache() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("usage.csv");
        fs::write(&path, "previous cache").unwrap();
        for csv in [
            Err(anyhow!("transport failed")),
            Ok("<html>Date Model</html>".into()),
            Ok("Date,Model\n2026-04-24,model\n".into()),
            Ok(format!("{HEADER}bad-date,model,10,5,0,1,0.01\n")),
            Ok(format!("{HEADER}2026-04-24,model,10,invalid,0,1,0.01\n")),
        ] {
            let api = FakeApi {
                csv,
                calls: Cell::new(0),
            };
            assert!(sync_active_account_with_api(&api, &credentials("token"), dir.path()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), "previous cache");
        }
    }

    #[test]
    fn invalid_token_fails_before_transport_and_is_redacted() {
        let dir = tempdir().unwrap();
        for token in ["", "secret\r\ninjected", "secret;extra=cookie"] {
            let api = FakeApi {
                csv: Ok(String::new()),
                calls: Cell::new(0),
            };
            let err =
                sync_active_account_with_api(&api, &credentials(token), dir.path()).unwrap_err();
            assert!(!err.to_string().contains("secret"));
            assert_eq!(api.calls.get(), 0);
        }
    }

    #[test]
    fn credential_reads_support_legacy_formats_without_migration() {
        let dir = tempdir().unwrap();
        let primary = dir.path().join("primary.json");
        let legacy = dir.path().join("legacy.json");
        let account = credentials("token");
        let formats = [
            serde_json::to_string(&account).unwrap(),
            serde_json::to_string(&single_account_store(account)).unwrap(),
        ];
        for raw in formats {
            fs::write(&legacy, &raw).unwrap();
            let store = load_credentials_from_paths(&primary, &legacy)
                .unwrap()
                .unwrap();
            assert_eq!(store.accounts.len(), 1);
            assert!(!primary.exists());
            assert_eq!(fs::read_to_string(&legacy).unwrap(), raw);
        }
    }

    #[test]
    fn corrupt_primary_does_not_fall_back_or_expose_token() {
        let dir = tempdir().unwrap();
        let primary = dir.path().join("primary.json");
        let legacy = dir.path().join("legacy.json");
        fs::write(&primary, "secret malformed config").unwrap();
        fs::write(
            &legacy,
            serde_json::to_string(&credentials("token")).unwrap(),
        )
        .unwrap();
        let error = load_credentials_from_paths(&primary, &legacy)
            .err()
            .unwrap();
        assert!(!error.to_string().contains("secret"));
    }

    #[test]
    fn failed_atomic_publish_preserves_target_and_removes_temporary_file() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("usage.csv");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("preserved"), "old").unwrap();
        assert!(atomic_write_file(&target, b"new").is_err());
        assert_eq!(fs::read_to_string(target.join("preserved")).unwrap(), "old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_publishes_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let target = dir.path().join("secret.json");
        atomic_write_file(&target, b"secret").unwrap();
        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
