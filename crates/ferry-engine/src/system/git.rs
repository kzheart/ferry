//! 从工作目录解析当前 git 分支（只读 `.git/HEAD`，不调 `git`）。
//!
//! 供会话列表等 UI 出参附带分支名：按目录缓存，同一次进程里同一 cwd 只读一次。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static BRANCH_CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    BRANCH_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析 `dir` 所在仓库的当前分支名；非 git 仓库或读失败返回 `None`。
///
/// - `ref: refs/heads/foo` → `foo`
/// - detached HEAD → 短 SHA（前 7 位）
/// - worktree（`.git` 是文件）会跟着 `gitdir:` 走
pub fn branch_of(dir: &str) -> Option<String> {
    let key = dir.trim();
    if key.is_empty() {
        return None;
    }
    {
        let guard = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = guard.get(key) {
            return cached.clone();
        }
    }
    let resolved = resolve_branch(Path::new(key));
    let mut guard = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.insert(key.to_string(), resolved.clone());
    resolved
}

fn resolve_branch(start: &Path) -> Option<String> {
    let git_dir = find_git_dir(start)?;
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    parse_head(&head)
}

fn find_git_dir(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };
    loop {
        let marker = current.join(".git");
        if marker.is_dir() {
            return Some(marker);
        }
        if marker.is_file() {
            // worktree: `.git` 内容形如 `gitdir: /path/to/repo/.git/worktrees/name`
            let text = fs::read_to_string(&marker).ok()?;
            let line = text.lines().next()?.trim();
            let gitdir = line.strip_prefix("gitdir:")?.trim();
            let path = if Path::new(gitdir).is_absolute() {
                PathBuf::from(gitdir)
            } else {
                current.join(gitdir)
            };
            return if path.is_dir() { Some(path) } else { None };
        }
        if !current.pop() {
            return None;
        }
    }
}

fn parse_head(head: &str) -> Option<String> {
    let line = head.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(reference) = line.strip_prefix("ref:") {
        let reference = reference.trim();
        if let Some(name) = reference.strip_prefix("refs/heads/") {
            return Some(name.to_string());
        }
        // 其它 ref（如 refs/remotes/...）取最后一段，总比空白有用
        return reference
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .map(str::to_string);
    }
    // detached HEAD：裸 SHA
    let sha = line.trim();
    if sha.len() >= 7 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Some(sha[..7].to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_head_reads_branch_name() {
        assert_eq!(
            parse_head("ref: refs/heads/main\n").as_deref(),
            Some("main")
        );
        assert_eq!(
            parse_head("ref: refs/heads/feature/foo").as_deref(),
            Some("feature/foo")
        );
    }

    #[test]
    fn parse_head_shortens_detached_sha() {
        assert_eq!(
            parse_head("abcdef0123456789\n").as_deref(),
            Some("abcdef0")
        );
    }

    #[test]
    fn branch_of_reads_repo_head() {
        let root = tempfile::tempdir().unwrap();
        let git = root.path().join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/demo\n").unwrap();
        // 绕过进程缓存：用唯一路径
        let dir = root.path().join("src");
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(branch_of(&dir.to_string_lossy()).as_deref(), Some("demo"));
    }
}
