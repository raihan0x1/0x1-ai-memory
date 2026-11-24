use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProjectIdentity {
    pub id: String,
    pub name: String,
    pub remote_url: Option<String>,
    pub root_path: String,
    pub branch: Option<String>,
}

/// Resolves a working directory into a canonical cross-worktree, cross-agent ProjectIdentity.
/// Reads `.git` directly without spawning a subprocess (<1ms).
pub fn resolve_project(cwd: &Path) -> ProjectIdentity {
    let canonical = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let (repo_root, git_dir, common_dir) = find_git_dirs(&canonical);

    let root_path = repo_root
        .clone()
        .unwrap_or_else(|| canonical.clone())
        .to_string_lossy()
        .into_owned();

    let remote_url = common_dir
        .as_ref()
        .or(git_dir.as_ref())
        .and_then(|dir| read_origin_remote(&dir.join("config")))
        .map(|u| normalize_remote_url(&u));

    let branch = git_dir.as_ref().and_then(|dir| read_head_branch(dir));

    let name = remote_url
        .as_deref()
        .and_then(|u| u.rsplit('/').next())
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            Path::new(&root_path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown-project".into());

    let identity_key = remote_url
        .as_deref()
        .map(|u| format!("remote:{u}"))
        .unwrap_or_else(|| format!("path:{root_path}"));

    let mut hasher = Sha256::new();
    hasher.update(identity_key.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let id = format!("proj_{}", &hash[..16]);

    ProjectIdentity {
        id,
        name,
        remote_url,
        root_path,
        branch,
    }
}

fn find_git_dirs(start: &Path) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let mut current = Some(start);
    while let Some(dir) = current {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return (
                Some(dir.to_path_buf()),
                Some(dot_git.clone()),
                Some(dot_git),
            );
        }
        if dot_git.is_file()
            && let Ok(content) = std::fs::read_to_string(&dot_git)
            && let Some(raw_gd) = content.trim().strip_prefix("gitdir:")
        {
            let gd_path = PathBuf::from(raw_gd.trim());
            let resolved_gd = if gd_path.is_absolute() {
                gd_path
            } else {
                dir.join(gd_path)
            };
            let common = std::fs::read_to_string(resolved_gd.join("commondir"))
                .ok()
                .map(|c| {
                    let p = PathBuf::from(c.trim());
                    if p.is_absolute() {
                        p
                    } else {
                        resolved_gd.join(p)
                    }
                })
                .and_then(|p| std::fs::canonicalize(&p).ok().or(Some(p)));
            return (
                Some(dir.to_path_buf()),
                Some(resolved_gd.clone()),
                common.or(Some(resolved_gd)),
            );
        }
        current = dir.parent();
    }
    (None, None, None)
}

fn read_origin_remote(config_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(config_path).ok()?;
    let mut in_origin = false;
    let mut any_remote = false;
    let mut fallback: Option<String> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_origin = line == r#"[remote "origin"]"#;
            any_remote = line.starts_with("[remote \"");
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == "url"
        {
            let url = v.trim().to_string();
            if in_origin && !url.is_empty() {
                return Some(url);
            }
            if any_remote && fallback.is_none() && !url.is_empty() {
                fallback = Some(url);
            }
        }
    }
    fallback
}

fn read_head_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let trimmed = head.trim();
    if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
        Some(rest.to_string())
    } else if trimmed.len() >= 8 {
        Some(trimmed[..8].to_string())
    } else {
        None
    }
}

pub fn normalize_remote_url(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for prefix in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
            break;
        }
    }
    if let Some(at_idx) = s.find('@')
        && !s[..at_idx].contains('/')
    {
        s = s[at_idx + 1..].to_string();
    }
    if let Some(colon_idx) = s.find(':') {
        let host = &s[..colon_idx];
        let rest = &s[colon_idx + 1..];
        if !host.contains('/') {
            s = format!("{host}/{rest}");
        }
    }
    s = s.trim_end_matches('/').to_string();
    if let Some(stripped) = s.strip_suffix(".git") {
        s = stripped.to_string();
    }
    s.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_ssh_and_https_remotes_identically() {
        let a = normalize_remote_url("git@github.com:User/0x1-ai-memory.git");
        let b = normalize_remote_url("https://github.com/user/0x1-ai-memory");
        let c = normalize_remote_url("ssh://git@github.com/User/0x1-ai-memory.git/");
        assert_eq!(a, "github.com/user/0x1-ai-memory");
        assert_eq!(a, b);
        assert_eq!(b, c);
    }
}
