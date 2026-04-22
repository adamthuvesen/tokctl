use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ResolvedRoots {
    pub roots: Vec<PathBuf>,
    #[allow(dead_code)] // Retained for diagnostics — "was this user-supplied?"
    pub user_supplied: bool,
}

pub struct ResolveInput<'a> {
    pub flag: Option<&'a str>,
    pub tokctl_env: Option<&'a str>,
    pub tool_env: Option<&'a str>,
    pub tool_env_suffix: Option<&'a str>,
    pub defaults: Vec<PathBuf>,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn expand(p: &str) -> PathBuf {
    let trimmed = p.trim();
    if trimmed == "~" {
        home_dir()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home_dir().join(rest)
    } else {
        PathBuf::from(trimmed)
    }
}

fn split_csv(value: &str) -> Vec<PathBuf> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(expand)
        .collect()
}

pub fn resolve_roots(input: ResolveInput<'_>) -> ResolvedRoots {
    if let Some(flag) = input.flag.filter(|s| !s.trim().is_empty()) {
        return ResolvedRoots {
            roots: split_csv(flag),
            user_supplied: true,
        };
    }
    if let Some(env) = input.tokctl_env.filter(|s| !s.trim().is_empty()) {
        return ResolvedRoots {
            roots: split_csv(env),
            user_supplied: true,
        };
    }
    if let Some(env) = input.tool_env.filter(|s| !s.trim().is_empty()) {
        let parts = split_csv(env);
        let roots = match input.tool_env_suffix {
            Some(suffix) => parts.into_iter().map(|p| p.join(suffix)).collect(),
            None => parts,
        };
        return ResolvedRoots {
            roots,
            user_supplied: true,
        };
    }
    ResolvedRoots {
        roots: input.defaults,
        user_supplied: false,
    }
}

pub fn default_claude_roots() -> Vec<PathBuf> {
    let h = home_dir();
    vec![
        h.join(".claude").join("projects"),
        h.join(".config").join("claude").join("projects"),
    ]
}

pub fn default_codex_roots() -> Vec<PathBuf> {
    vec![home_dir().join(".codex").join("sessions")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_wins_over_env() {
        let r = resolve_roots(ResolveInput {
            flag: Some("/tmp/flag"),
            tokctl_env: Some("/tmp/env"),
            tool_env: Some("/tmp/tool"),
            tool_env_suffix: Some("x"),
            defaults: vec![PathBuf::from("/tmp/default")],
        });
        assert!(r.user_supplied);
        assert_eq!(r.roots, vec![PathBuf::from("/tmp/flag")]);
    }

    #[test]
    fn env_used_when_no_flag() {
        let r = resolve_roots(ResolveInput {
            flag: None,
            tokctl_env: Some("/tmp/env1,/tmp/env2"),
            tool_env: None,
            tool_env_suffix: None,
            defaults: vec![],
        });
        assert!(r.user_supplied);
        assert_eq!(r.roots.len(), 2);
    }

    #[test]
    fn tool_env_appends_suffix() {
        let r = resolve_roots(ResolveInput {
            flag: None,
            tokctl_env: None,
            tool_env: Some("/tmp/claude"),
            tool_env_suffix: Some("projects"),
            defaults: vec![],
        });
        assert_eq!(r.roots, vec![PathBuf::from("/tmp/claude/projects")]);
    }

    #[test]
    fn defaults_used_when_nothing_supplied() {
        let r = resolve_roots(ResolveInput {
            flag: None,
            tokctl_env: None,
            tool_env: None,
            tool_env_suffix: None,
            defaults: vec![PathBuf::from("/tmp/d")],
        });
        assert!(!r.user_supplied);
        assert_eq!(r.roots, vec![PathBuf::from("/tmp/d")]);
    }
}
