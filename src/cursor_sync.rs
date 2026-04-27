use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::paths::{
    cursor_credentials_path, cursor_sync_cache_dir, tokctl_config_dir,
    tokscale_cursor_credentials_path,
};

const USAGE_CSV_ENDPOINT: &str =
    "https://cursor.com/api/dashboard/export-usage-events-csv?strategy=tokens";
const USAGE_SUMMARY_ENDPOINT: &str = "https://cursor.com/api/usage-summary";

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct ValidateSessionResult {
    pub valid: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncCursorResult {
    pub synced: bool,
    pub rows: usize,
    pub path: PathBuf,
    pub error: Option<String>,
}

pub trait CursorApi {
    fn validate_session(&self, token: &str) -> Result<()>;
    fn fetch_usage_csv(&self, token: &str) -> Result<String>;
}

pub struct BlockingCursorApi {
    client: reqwest::blocking::Client,
    usage_csv_endpoint: String,
    usage_summary_endpoint: String,
}

impl BlockingCursorApi {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .context("building Cursor HTTP client")?;
        Ok(Self {
            client,
            usage_csv_endpoint: std::env::var("TOKCTL_CURSOR_USAGE_CSV_ENDPOINT")
                .unwrap_or_else(|_| USAGE_CSV_ENDPOINT.to_owned()),
            usage_summary_endpoint: std::env::var("TOKCTL_CURSOR_USAGE_SUMMARY_ENDPOINT")
                .unwrap_or_else(|_| USAGE_SUMMARY_ENDPOINT.to_owned()),
        })
    }
}

impl CursorApi for BlockingCursorApi {
    fn validate_session(&self, token: &str) -> Result<()> {
        let response = self
            .client
            .get(&self.usage_summary_endpoint)
            .headers(build_cursor_headers(token))
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
        Ok(())
    }

    fn fetch_usage_csv(&self, token: &str) -> Result<String> {
        let response = self
            .client
            .get(&self.usage_csv_endpoint)
            .headers(build_cursor_headers(token))
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
        if !text.contains("Date") || !text.contains("Model") {
            anyhow::bail!("Cursor returned an unexpected usage CSV response");
        }
        Ok(text)
    }
}

pub fn validate_cursor_session(token: &str) -> ValidateSessionResult {
    match BlockingCursorApi::new().and_then(|api| api.validate_session(token)) {
        Ok(()) => ValidateSessionResult {
            valid: true,
            error: None,
        },
        Err(err) => ValidateSessionResult {
            valid: false,
            error: Some(err.to_string()),
        },
    }
}

pub fn validate_active_account() -> Option<ValidateSessionResult> {
    let (_, credentials) = load_active_credentials()?;
    Some(validate_cursor_session(&credentials.session_token))
}

pub fn has_configured_account() -> bool {
    load_credentials_store().is_some_and(|s| !s.accounts.is_empty())
}

pub fn list_accounts() -> Vec<AccountInfo> {
    let Some(store) = load_credentials_store() else {
        return Vec::new();
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
    out
}

pub fn load_credentials_store() -> Option<CursorCredentialsStore> {
    let tokctl_path = cursor_credentials_path();
    if tokctl_path.exists() {
        let raw = fs::read_to_string(&tokctl_path).ok()?;
        return serde_json::from_str(&raw).ok();
    }

    let tokscale_path = tokscale_cursor_credentials_path();
    if !tokscale_path.exists() {
        return None;
    }
    let raw = fs::read_to_string(&tokscale_path).ok()?;
    let store = serde_json::from_str::<CursorCredentialsStore>(&raw)
        .ok()
        .or_else(|| {
            serde_json::from_str::<CursorCredentials>(&raw)
                .ok()
                .map(single_account_store)
        })?;
    let _ = save_credentials_store(&store);
    Some(store)
}

pub fn load_active_credentials() -> Option<(String, CursorCredentials)> {
    let store = load_credentials_store()?;
    let id = store.active_account_id.clone();
    let account = store.accounts.get(&id)?.clone();
    Some((id, account))
}

pub fn save_credentials(token: &str, label: Option<&str>) -> Result<String> {
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("Cursor session token cannot be empty");
    }
    let user_id = extract_user_id_from_session_token(token);
    let account_id = derive_account_id(token);

    let mut store = load_credentials_store().unwrap_or(CursorCredentialsStore {
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
    ensure_tokctl_config_dir()?;
    let path = cursor_credentials_path();
    let contents = serde_json::to_string_pretty(store)?;
    atomic_write_file(&path, contents.as_bytes())?;
    set_private_permissions(&path)?;
    Ok(())
}

pub fn sync_active_account(target_dir: Option<&Path>) -> SyncCursorResult {
    let path = target_dir
        .map(PathBuf::from)
        .unwrap_or_else(cursor_sync_cache_dir);
    let Some((_, credentials)) = load_active_credentials() else {
        return SyncCursorResult {
            synced: false,
            rows: 0,
            path,
            error: Some("No configured Cursor account".to_owned()),
        };
    };

    let api = match BlockingCursorApi::new() {
        Ok(api) => api,
        Err(err) => {
            return SyncCursorResult {
                synced: false,
                rows: 0,
                path,
                error: Some(err.to_string()),
            };
        }
    };
    sync_active_account_with_api(&api, &credentials, &path)
}

fn sync_active_account_with_api(
    api: &impl CursorApi,
    credentials: &CursorCredentials,
    target_dir: &Path,
) -> SyncCursorResult {
    let csv_path = target_dir.join("usage.csv");
    match api.fetch_usage_csv(&credentials.session_token) {
        Ok(csv) => {
            if let Err(err) = ensure_dir(target_dir)
                .and_then(|_| atomic_write_file(&csv_path, csv.as_bytes()))
                .and_then(|_| set_private_permissions(&csv_path))
            {
                return SyncCursorResult {
                    synced: false,
                    rows: 0,
                    path: csv_path,
                    error: Some(err.to_string()),
                };
            }
            SyncCursorResult {
                synced: true,
                rows: count_csv_rows(&csv),
                path: csv_path,
                error: None,
            }
        }
        Err(err) => SyncCursorResult {
            synced: false,
            rows: 0,
            path: csv_path,
            error: Some(err.to_string()),
        },
    }
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

fn build_cursor_headers(session_token: &str) -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );
    if let Ok(cookie) = format!("WorkosCursorSessionToken={session_token}").parse() {
        headers.insert("Cookie", cookie);
    }
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
    headers
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

fn count_csv_rows(csv: &str) -> usize {
    csv.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .count()
}

fn ensure_tokctl_config_dir() -> Result<()> {
    ensure_dir(&tokctl_config_dir())
}

fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))
}

fn atomic_write_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid path for {}", path.display()))?;
    ensure_dir(parent)?;
    let temp_path = parent.join(format!(
        ".tmp-{}-{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("cursor"),
        std::process::id()
    ));
    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(contents)?;
        file.sync_all().ok();
    }
    if fs::rename(&temp_path, path).is_err() {
        fs::copy(&temp_path, path)?;
        fs::remove_file(&temp_path).ok();
    }
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
    use tempfile::tempdir;

    struct FakeApi {
        validate_ok: bool,
        fetch_csv: Option<String>,
        fetch_error: Option<String>,
    }

    impl CursorApi for FakeApi {
        fn validate_session(&self, _token: &str) -> Result<()> {
            if self.validate_ok {
                Ok(())
            } else {
                anyhow::bail!("unauthorized")
            }
        }

        fn fetch_usage_csv(&self, _token: &str) -> Result<String> {
            if let Some(error) = &self.fetch_error {
                anyhow::bail!("{error}");
            }
            Ok(self.fetch_csv.clone().unwrap_or_default())
        }
    }

    #[test]
    fn single_account_store_roundtrips() {
        let account = CursorCredentials {
            session_token: "abc".into(),
            user_id: Some("user-1".into()),
            created_at: "2026-04-24T00:00:00Z".into(),
            expires_at: None,
            label: Some("main".into()),
        };
        let store = single_account_store(account);
        assert_eq!(store.version, 1);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.active_account_id, "user-1");
    }

    #[test]
    fn validate_result_reports_invalid() {
        let api = FakeApi {
            validate_ok: false,
            fetch_csv: None,
            fetch_error: None,
        };
        let err = api.validate_session("token").unwrap_err();
        assert!(err.to_string().contains("unauthorized"));
    }

    #[test]
    fn sync_writes_csv() {
        let dir = tempdir().unwrap();
        let api = FakeApi {
            validate_ok: true,
            fetch_csv: Some("Date,Model\n2026-04-24,gpt-5-codex\n".into()),
            fetch_error: None,
        };
        let credentials = CursorCredentials {
            session_token: "token".into(),
            user_id: None,
            created_at: Utc::now().to_rfc3339(),
            expires_at: None,
            label: None,
        };
        let result = sync_active_account_with_api(&api, &credentials, dir.path());
        assert!(result.synced);
        assert_eq!(result.rows, 1);
        assert!(dir.path().join("usage.csv").exists());
    }

    #[test]
    fn sync_failure_preserves_existing_cache() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("usage.csv");
        fs::write(&existing, "Date,Model\n2026-04-20,old\n").unwrap();
        let api = FakeApi {
            validate_ok: true,
            fetch_csv: None,
            fetch_error: Some("boom".into()),
        };
        let credentials = CursorCredentials {
            session_token: "token".into(),
            user_id: None,
            created_at: Utc::now().to_rfc3339(),
            expires_at: None,
            label: None,
        };
        let result = sync_active_account_with_api(&api, &credentials, dir.path());
        assert!(!result.synced);
        let contents = fs::read_to_string(&existing).unwrap();
        assert!(contents.contains("old"));
    }
}
