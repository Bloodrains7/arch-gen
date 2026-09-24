//! Git for the project folder and for any repository the user points at.
//!
//! Reads (status, tags and branches, a range of commits for release notes)
//! never change the repository: no optional locks, no fsmonitor, no prompts.
//! The two writes, commit and push, run only on an explicit click, only for the
//! folder of the open project, and a commit never contains anything outside that
//! folder — not even files the user staged elsewhere in the same repository.
use crate::project;
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

/// A release rarely has more; the rest is reported as truncated, never silently dropped.
const MAX_COMMITS: usize = 2000;
const MAX_BODY: usize = 16 * 1024;
const MAX_MESSAGE: usize = 16 * 1024;
const MAX_REFS: usize = 1000;
const MAX_CHANGES: usize = 500;
const PUSH_TIMEOUT: Duration = Duration::from_secs(180);

type Result<T> = std::result::Result<T, String>;

const NO_REMOTE: &str = "This repository has no remote to push to. Add one with git remote add origin <url> and push once with git push -u origin <branch>.";
const NO_UPSTREAM: &str = "This branch has no upstream yet. Push it once from a terminal with git push -u origin <branch>; later pushes work from ArchGen.";

fn command(dir: &Path) -> Command {
    let mut command = project::git(dir);
    command
        .args(["-c", "core.fsmonitor=false", "-c", "log.showSignature=false", "-c", "color.ui=false"])
        // Never wait for a password on an invisible terminal; English messages for the checks below.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .stdin(Stdio::null());
    command
}

fn friendly(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    if text.contains("not a git repository") {
        return "This folder is not in a Git repository. Run git init there, or choose another folder.".into();
    }
    if text.contains("dubious ownership") {
        return "Git refuses this repository because it belongs to another user (safe.directory). Nothing was changed.".into();
    }
    if text.contains("Please tell me who you are") || text.contains("unable to auto-detect email address") {
        return "Git does not know who you are. Set it once with git config --global user.name \"Your Name\" and git config --global user.email you@example.com.".into();
    }
    if text.contains("No configured push destination") {
        return NO_REMOTE.into();
    }
    if text.contains("has no upstream branch") || text.contains("no upstream configured") {
        return NO_UPSTREAM.into();
    }
    if text.contains("could not read Username") || text.contains("Authentication failed") || text.contains("Permission denied") {
        return "The remote asked for credentials that Git could not supply without a prompt. Sign in once with your Git credential manager or SSH agent, then push again.".into();
    }
    let short: String = text.chars().take(600).collect();
    if short.is_empty() {
        "Git failed without a message.".into()
    } else {
        format!("Git failed: {short}")
    }
}

fn output(mut command: Command) -> Result<Output> {
    command.output().map_err(project::git_error)
}

fn run(command: Command) -> Result<String> {
    let output = output(command)?;
    if !output.status.success() {
        return Err(friendly(&output.stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn folder(path: &str) -> Result<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() || !path.is_dir() {
        return Err("Choose an existing folder.".into());
    }
    Ok(path)
}

fn toplevel(dir: &Path) -> Result<String> {
    let mut c = command(dir);
    c.args(["rev-parse", "--show-toplevel"]);
    Ok(run(c)?.trim().to_string())
}

fn branch(dir: &Path) -> Option<String> {
    let mut c = command(dir);
    c.args(["symbolic-ref", "--quiet", "--short", "HEAD"]);
    run(c).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Anything `git rev-parse` understands (`v1.2`, `main`, `HEAD~3`, a hash), but
/// never an option, a range or something that spans lines.
fn valid_revision(revision: &str) -> Result<&str> {
    let bad = revision.is_empty()
        || revision.len() > 255
        || revision.starts_with('-')
        || revision.contains("..")
        || revision.chars().any(|c| c.is_whitespace() || c.is_control());
    if bad {
        Err(format!("\"{}\" is not a Git revision.", revision.chars().take(80).collect::<String>()))
    } else {
        Ok(revision)
    }
}

/// Full commit hash of a revision, so later commands only ever see hex.
fn resolve(dir: &Path, revision: &str) -> Result<String> {
    let revision = valid_revision(revision)?;
    let mut c = command(dir);
    c.args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(format!("{revision}^{{commit}}"));
    let output = output(c)?;
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || hash.len() < 40 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        let error = friendly(&output.stderr);
        return Err(if error.contains("not in a Git repository") {
            error
        } else {
            format!("Git does not know the revision \"{revision}\".")
        });
    }
    Ok(hash)
}

/// A folder inside the repository for a monorepo's release notes: relative, no `..`.
fn valid_subpath(subpath: &str) -> Result<String> {
    let trimmed = subpath.trim().trim_matches(['/', '\\']).replace('\\', "/");
    let bad = trimmed.len() > 400
        || trimmed.starts_with(':')
        || trimmed.chars().any(|c| c.is_control())
        || trimmed.split('/').any(|part| part == ".." || part.contains(':'));
    if bad {
        return Err("The folder filter must be a relative folder inside the repository, without \"..\".".into());
    }
    Ok(trimmed)
}

// ── Tags and branches ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRef {
    name: String,
    kind: &'static str,
    commit: String,
    date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRefs {
    root: String,
    branch: Option<String>,
    head: Option<String>,
    /// Newest first, tags before branches only by date.
    refs: Vec<GitRef>,
}

fn refs(dir: &Path) -> Result<GitRefs> {
    let root = toplevel(dir)?;
    let mut head = command(dir);
    head.args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
    let head = run(head).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut c = command(dir);
    c.args([
        "for-each-ref",
        "--sort=-creatordate",
        &format!("--count={}", MAX_REFS),
        "--format=%(refname)%1f%(objectname)%1f%(*objectname)%1f%(creatordate:iso-strict)",
        "refs/tags",
        "refs/heads",
        "refs/remotes",
    ]);
    let refs = run(c)?
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\u{1f}');
            let full = parts.next()?;
            let object = parts.next()?;
            let peeled = parts.next()?;
            let date = parts.next().unwrap_or_default();
            let (kind, name) = if let Some(name) = full.strip_prefix("refs/tags/") {
                ("tag", name)
            } else if let Some(name) = full.strip_prefix("refs/heads/") {
                ("branch", name)
            } else {
                ("remote", full.strip_prefix("refs/remotes/")?)
            };
            if name.ends_with("/HEAD") {
                return None;
            }
            Some(GitRef {
                name: name.into(),
                kind,
                commit: if peeled.is_empty() { object } else { peeled }.into(),
                date: date.into(),
            })
        })
        .collect();
    Ok(GitRefs { root, branch: branch(dir), head, refs })
}

#[tauri::command]
pub async fn git_list_refs(path: String) -> Result<GitRefs> {
    tokio::task::spawn_blocking(move || refs(folder(&path)?))
        .await
        .map_err(|e| e.to_string())?
}

// ── Commits of a range, for release notes ────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    hash: String,
    short_hash: String,
    parents: usize,
    author: String,
    date: String,
    subject: String,
    body: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRange {
    root: String,
    from: Option<String>,
    to: String,
    /// Newest first, exactly as `git log` lists them.
    commits: Vec<GitCommit>,
    /// More than `MAX_COMMITS` commits: only the newest ones are listed.
    truncated: bool,
}

fn parse_log(text: &str) -> Vec<GitCommit> {
    text.split('\u{1e}')
        .map(|record| record.trim_start_matches(['\n', '\r']))
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let mut parts = record.splitn(6, '\u{1f}');
            let hash = parts.next()?.to_string();
            let parents = parts.next()?.split_whitespace().count();
            let author = parts.next()?.to_string();
            let date = parts.next()?.to_string();
            let subject = parts.next()?.to_string();
            let body: String = parts.next().unwrap_or_default().trim_end().chars().take(MAX_BODY).collect();
            Some(GitCommit { short_hash: hash.chars().take(7).collect(), hash, parents, author, date, subject, body })
        })
        .collect()
}

fn range(dir: &Path, from: Option<&str>, to: &str, subpath: Option<&str>, merges: bool) -> Result<GitRange> {
    let root = toplevel(dir)?;
    let to = resolve(dir, to)?;
    let from = match from.map(str::trim).filter(|f| !f.is_empty()) {
        Some(from) => Some(resolve(dir, from)?),
        None => None,
    };
    let subpath = match subpath.map(str::trim).filter(|s| !s.is_empty()) {
        Some(subpath) => Some(valid_subpath(subpath)?),
        None => None,
    };
    let mut c = command(dir);
    c.args(["log", "--no-color", &format!("-n{}", MAX_COMMITS + 1)])
        .arg("--format=%H%x1f%P%x1f%aN%x1f%aI%x1f%s%x1f%b%x1e");
    if !merges {
        c.arg("--no-merges");
    }
    c.arg(&to);
    if let Some(from) = &from {
        c.arg(format!("^{from}"));
    }
    c.arg("--");
    if let Some(subpath) = &subpath {
        // Relative to the repository root, wherever inside it `dir` is.
        c.arg(format!("{root}/{subpath}"));
    }
    let mut commits = parse_log(&run(c)?);
    let truncated = commits.len() > MAX_COMMITS;
    commits.truncate(MAX_COMMITS);
    Ok(GitRange { root, from, to, commits, truncated })
}

#[tauri::command]
pub async fn git_log_range(
    path: String,
    from: Option<String>,
    to: String,
    subpath: Option<String>,
    include_merges: Option<bool>,
) -> Result<GitRange> {
    tokio::task::spawn_blocking(move || {
        range(folder(&path)?, from.as_deref(), &to, subpath.as_deref(), include_merges.unwrap_or(false))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Status of the project folder ─────────────────────────────────────────────

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitChange {
    /// Relative to the project folder, with `/`.
    path: String,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    root: String,
    branch: Option<String>,
    head: Option<String>,
    upstream: Option<String>,
    ahead: u64,
    behind: u64,
    /// Changes inside the project folder only.
    changes: Vec<GitChange>,
    truncated: bool,
}

fn change_kind(xy: &str) -> &'static str {
    let mut codes = xy.chars().filter(|c| *c != '.');
    match codes.next() {
        Some('A') | Some('C') => "added",
        Some('D') => "deleted",
        Some('R') => "renamed",
        _ => "modified",
    }
}

/// `git status --porcelain=v2 --branch -z`; paths in it are relative to the repository root.
fn parse_status(text: &str, prefix: &str, root: String) -> GitStatus {
    let mut status = GitStatus {
        root,
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        changes: Vec::new(),
        truncated: false,
    };
    let mut tokens = text.split('\0');
    while let Some(token) = tokens.next() {
        let (path, kind) = if let Some(header) = token.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => status.head = Some(value.into()),
                "branch.head" if value != "(detached)" => status.branch = Some(value.into()),
                "branch.upstream" => status.upstream = Some(value.into()),
                "branch.ab" => {
                    for part in value.split_whitespace() {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        } else if let Some(rest) = token.strip_prefix("1 ") {
            let fields: Vec<&str> = rest.splitn(8, ' ').collect();
            (fields.get(7).copied(), fields.first().map(|xy| change_kind(xy)))
        } else if let Some(rest) = token.strip_prefix("2 ") {
            let _original = tokens.next();
            (rest.splitn(9, ' ').nth(8), Some("renamed"))
        } else if let Some(rest) = token.strip_prefix("u ") {
            (rest.splitn(10, ' ').nth(9), Some("conflicted"))
        } else if let Some(rest) = token.strip_prefix("? ") {
            (Some(rest), Some("untracked"))
        } else {
            continue;
        };
        let (Some(path), Some(kind)) = (path, kind) else { continue };
        let Some(path) = path.strip_prefix(prefix) else { continue };
        if status.changes.len() == MAX_CHANGES {
            status.truncated = true;
            continue;
        }
        status.changes.push(GitChange { path: path.into(), status: kind });
    }
    status
}

fn prefix(dir: &Path) -> Result<String> {
    let mut c = command(dir);
    c.args(["rev-parse", "--show-prefix"]);
    Ok(run(c)?.trim_end_matches(['\n', '\r']).to_string())
}

fn status(dir: &Path) -> Result<GitStatus> {
    let root = toplevel(dir)?;
    let prefix = prefix(dir)?;
    let mut c = command(dir);
    c.args(["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all", "--", "."]);
    Ok(parse_status(&run(c)?, &prefix, root))
}

#[tauri::command]
pub async fn git_project_status(path: String, project_id: String) -> Result<GitStatus> {
    tokio::task::spawn_blocking(move || {
        let dir = folder(&path)?;
        project::ensure_project(dir, &project_id)?;
        status(dir)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Commit and push the project folder ───────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitted {
    commit: String,
    summary: String,
    files: usize,
}

fn valid_message(message: &str) -> Result<String> {
    let message = message.replace("\r\n", "\n");
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first.".into());
    }
    if message.len() > MAX_MESSAGE || message.contains('\0') {
        return Err("The commit message is too long or contains invalid characters.".into());
    }
    Ok(format!("{message}\n"))
}

/// `expected_fingerprint` is what the app last saved or opened: the commit must
/// contain exactly that, not an edit made meanwhile by another editor or a pull.
fn commit(dir: &Path, project_id: &str, expected_fingerprint: &str, message: &str) -> Result<GitCommitted> {
    let message = valid_message(message)?;
    project::ensure_project(dir, project_id)?;
    if project::disk_fingerprint(dir, project_id)? != expected_fingerprint {
        return Err("The project folder changed on disk since ArchGen last saved or opened it. Open it again and review it before committing.".into());
    }
    let mut add = command(dir);
    add.args(["add", "--all", "--", "."]);
    run(add)?;
    let mut staged = command(dir);
    staged.args(["diff", "--cached", "--name-only", "-z", "--", "."]);
    let files = run(staged)?.split('\0').filter(|f| !f.is_empty()).count();
    if files == 0 {
        return Err("Nothing to commit: the project folder has no changes since its last commit.".into());
    }
    // `-- .` commits only the project folder, whatever else is staged in the repository.
    let mut c = command(dir);
    c.args(["commit", "--quiet", "--file=-", "--", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().map_err(project::git_error)?;
    child
        .stdin
        .take()
        .ok_or("Git input is unavailable.")?
        .write_all(message.as_bytes())
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(friendly(&output.stderr));
    }
    let mut head = command(dir);
    head.args(["rev-parse", "HEAD"]);
    Ok(GitCommitted {
        commit: run(head)?.trim().to_string(),
        summary: message.lines().next().unwrap_or_default().to_string(),
        files,
    })
}

#[tauri::command]
pub async fn git_commit_project(
    path: String,
    project_id: String,
    expected_fingerprint: String,
    message: String,
) -> Result<GitCommitted> {
    tokio::task::spawn_blocking(move || commit(folder(&path)?, &project_id, &expected_fingerprint, &message))
        .await
        .map_err(|e| e.to_string())?
}

fn config(dir: &Path, key: &str) -> Option<String> {
    let mut c = command(dir);
    c.args(["config", "--get", key]);
    run(c).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The current branch to exactly its upstream branch, whatever `push.default` says;
/// never a force push, never another branch, bounded in time.
fn push(dir: &Path, project_id: &str, timeout: Duration) -> Result<String> {
    project::ensure_project(dir, project_id)?;
    let branch = branch(dir).ok_or("A detached HEAD cannot be pushed. Check out a branch first.")?;
    let (Some(remote), Some(merge)) = (config(dir, &format!("branch.{branch}.remote")), config(dir, &format!("branch.{branch}.merge"))) else {
        let mut remotes = command(dir);
        remotes.arg("remote");
        return Err(if run(remotes)?.trim().is_empty() { NO_REMOTE } else { NO_UPSTREAM }.into());
    };
    if remote == "." || remote.starts_with('-') || !merge.starts_with("refs/heads/") || merge.chars().any(|c| c.is_whitespace()) {
        return Err(format!("The upstream of {branch} is not a remote branch ({remote} {merge}); push it from a terminal."));
    }
    let mut c = command(dir);
    c.args(["push", "--porcelain", &remote, &format!("HEAD:{merge}")]).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(project::git_error)?;
    let mut stdout = child.stdout.take().ok_or("Git output is unavailable.")?;
    let mut stderr = child.stderr.take().ok_or("Git output is unavailable.")?;
    let out = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stdout.read_to_end(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        text
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Push did not finish within {} seconds and was stopped. Check the network and the remote.", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        let text = String::from_utf8_lossy(&stdout);
        if text.contains("[rejected]") || text.contains("non-fast-forward") {
            return Err("The remote has commits you do not have yet. Pull or merge them first (outside ArchGen), then push again. Nothing was overwritten.".into());
        }
        return Err(friendly(&stderr));
    }
    Ok(format!("Pushed {branch} to {remote}."))
}

#[tauri::command]
pub async fn git_push_project(path: String, project_id: String) -> Result<String> {
    tokio::task::spawn_blocking(move || push(folder(&path)?, &project_id, PUSH_TIMEOUT))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Project;

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "archgen-git-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    fn sh(dir: &Path, args: &[&str]) -> String {
        let output = project::git(dir)
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", "-c", "init.defaultBranch=main"])
            .args(args)
            .env("GIT_AUTHOR_DATE", "2026-09-01T10:00:00+02:00")
            .env("GIT_COMMITTER_DATE", "2026-09-01T10:00:00+02:00")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn repo() -> TempDir {
        let dir = TempDir::new();
        sh(&dir.0, &["init", "-q"]);
        sh(&dir.0, &["config", "user.name", "Tester"]);
        sh(&dir.0, &["config", "user.email", "tester@example.invalid"]);
        sh(&dir.0, &["config", "commit.gpgsign", "false"]);
        dir
    }

    fn commit_file(dir: &Path, file: &str, text: &str, message: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        sh(dir, &["add", "--", file]);
        sh(dir, &["commit", "-q", "-m", message]);
    }

    fn fixture() -> Project {
        serde_json::from_value(serde_json::json!({"schemaVersion":1,"id":"project","name":"Docs","revision":1,"activeDocumentId":"doc","documents":[{"id":"doc","name":"Architecture","template":"arc42","language":"en","revision":1,"sections":[{"id":"s","title":"Overview","content":"Text","diagrams":[]}]}]})).unwrap()
    }

    #[test]
    fn revisions_and_folder_filters_refuse_options_ranges_and_escapes() {
        for bad in ["", "-n", "--all", "a..b", "HEAD main", "x\ny", &"a".repeat(300)] {
            assert!(valid_revision(bad).is_err(), "{bad:?}");
        }
        for good in ["v1.2.0", "main", "HEAD~3", "origin/release/2.0", "abc1234"] {
            assert!(valid_revision(good).is_ok(), "{good:?}");
        }
        assert_eq!(valid_subpath("/services/billing/").unwrap(), "services/billing");
        assert_eq!(valid_subpath("docs\\arch").unwrap(), "docs/arch");
        for bad in ["../outside", "a/../../b", ":(exclude)x", "C:/x"] {
            assert!(valid_subpath(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn log_records_keep_multiline_bodies_and_unicode() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{1f}p1 p2\u{1f}Ján\u{1f}2026-09-01T10:00:00+02:00\u{1f}feat(api): pridaj\u{1f}Body line\n\nBREAKING CHANGE: x\n\u{1e}\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\u{1f}\u{1f}A\u{1f}2026\u{1f}fix: y\u{1f}\u{1e}\n";
        let commits = parse_log(text);
        assert_eq!(commits.len(), 2);
        assert_eq!((commits[0].parents, commits[0].author.as_str(), commits[0].short_hash.as_str()), (2, "Ján", "aaaaaaa"));
        assert_eq!(commits[0].body, "Body line\n\nBREAKING CHANGE: x");
        assert_eq!((commits[1].parents, commits[1].subject.as_str(), commits[1].body.as_str()), (0, "fix: y", ""));
    }

    #[test]
    fn status_lists_only_the_project_folder_with_branch_and_divergence() {
        let text = "# branch.oid 0123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 a b docs/arch/overview.md\0\
1 A. N... 000000 100644 100644 a b docs/arch/new file.md\0\
1 .D N... 100644 100644 000000 a b docs/arch/gone.md\0\
2 R. N... 100644 100644 100644 a b R100 docs/arch/renamed.md\0docs/arch/old.md\0\
u UU N... 100644 100644 100644 100644 a b c docs/arch/conflict.md\0\
? docs/arch/untracked.puml\0\
1 .M N... 100644 100644 100644 a b src/outside.rs\0";
        let status = parse_status(text, "docs/arch/", "/repo".into());
        assert_eq!((status.branch.as_deref(), status.upstream.as_deref(), status.ahead, status.behind), (Some("main"), Some("origin/main"), 2, 1));
        let changes: Vec<(&str, &str)> = status.changes.iter().map(|c| (c.path.as_str(), c.status)).collect();
        assert_eq!(changes, vec![
            ("overview.md", "modified"), ("new file.md", "added"), ("gone.md", "deleted"),
            ("renamed.md", "renamed"), ("conflict.md", "conflicted"), ("untracked.puml", "untracked"),
        ]);
        let initial = parse_status("# branch.oid (initial)\0# branch.head (detached)\0", "", "/r".into());
        assert_eq!((initial.head, initial.branch), (None, None));
    }

    #[test]
    fn refs_and_ranges_come_from_a_real_repository() {
        if !git_available() {
            return;
        }
        let repo = repo();
        assert!(refs(&repo.0).unwrap().head.is_none());
        commit_file(&repo.0, "a.txt", "1", "feat: first");
        sh(&repo.0, &["tag", "v1.0.0"]);
        commit_file(&repo.0, "services/billing/b.txt", "2", "fix(billing): rounding");
        commit_file(&repo.0, "c.txt", "3", "docs: readme");
        sh(&repo.0, &["tag", "-a", "v1.1.0", "-m", "Release 1.1"]);
        sh(&repo.0, &["checkout", "-q", "-b", "topic"]);
        commit_file(&repo.0, "d.txt", "4", "feat: topic work");
        sh(&repo.0, &["checkout", "-q", "main"]);
        sh(&repo.0, &["merge", "-q", "--no-ff", "-m", "Merge branch 'topic'", "topic"]);

        let listed = refs(&repo.0).unwrap();
        assert_eq!(listed.branch.as_deref(), Some("main"));
        let tag = listed.refs.iter().find(|r| r.name == "v1.1.0").unwrap();
        assert_eq!((tag.kind, tag.commit.as_str()), ("tag", sh(&repo.0, &["rev-parse", "v1.1.0^{commit}"]).as_str()));
        assert!(listed.refs.iter().any(|r| r.name == "topic" && r.kind == "branch"));

        let subjects = |range: GitRange| range.commits.into_iter().map(|c| c.subject).collect::<Vec<_>>();
        assert_eq!(subjects(range(&repo.0, Some("v1.0.0"), "v1.1.0", None, false).unwrap()), vec!["docs: readme", "fix(billing): rounding"]);
        assert_eq!(subjects(range(&repo.0, Some("v1.1.0"), "HEAD", None, false).unwrap()), vec!["feat: topic work"]);
        assert_eq!(subjects(range(&repo.0, Some("v1.1.0"), "HEAD", None, true).unwrap()), vec!["Merge branch 'topic'", "feat: topic work"]);
        assert_eq!(subjects(range(&repo.0, None, "v1.0.0", None, false).unwrap()), vec!["feat: first"]);
        assert_eq!(subjects(range(&repo.0, None, "HEAD", Some("services/billing"), false).unwrap()), vec!["fix(billing): rounding"]);
        // Works from a subfolder too: the filter is relative to the repository root.
        assert_eq!(subjects(range(&repo.0.join("services"), None, "HEAD", Some("services/billing"), false).unwrap()).len(), 1);
        assert!(range(&repo.0, Some("v9"), "HEAD", None, false).unwrap_err().contains("does not know"));
        assert!(range(&repo.0, Some("--output=/tmp/x"), "HEAD", None, false).unwrap_err().contains("not a Git revision"));

        let plain = TempDir::new();
        assert!(refs(&plain.0).unwrap_err().contains("not in a Git repository"));
    }

    #[test]
    fn commit_contains_only_the_saved_project_folder_and_push_reaches_the_upstream() {
        if !git_available() {
            return;
        }
        let repo = repo();
        commit_file(&repo.0, "README.md", "readme", "chore: init");
        let root = repo.0.join("docs/architecture");
        let fingerprint = project::save_for_tests(&root, &fixture());
        // Something the user staged elsewhere must stay out of the project commit.
        std::fs::write(repo.0.join("unrelated.txt"), "staged elsewhere").unwrap();
        sh(&repo.0, &["add", "unrelated.txt"]);

        assert!(commit(&root, "project", &fingerprint, "  ").unwrap_err().contains("commit message"));
        assert!(commit(&root, "other", &fingerprint, "docs").is_err());
        assert!(commit(&root, "project", "stale", "docs").unwrap_err().contains("changed on disk"));

        let before = status(&root).unwrap();
        assert!(before.changes.iter().all(|c| c.status == "untracked"), "{:?}", before.changes);
        assert!(before.changes.iter().any(|c| c.path == "archgen.json"));
        assert!(!before.changes.iter().any(|c| c.path.contains("unrelated")));

        let done = commit(&root, "project", &fingerprint, "-docs: architecture\n\nFirst version").unwrap();
        assert_eq!((done.summary.as_str(), done.files), ("-docs: architecture", 3));
        assert_eq!(sh(&repo.0, &["log", "-1", "--format=%B"]), "-docs: architecture\n\nFirst version");
        let committed = sh(&repo.0, &["show", "--name-only", "--format=", "HEAD"]);
        assert!(committed.contains("docs/architecture/archgen.json") && !committed.contains("unrelated"));
        assert_eq!(sh(&repo.0, &["diff", "--cached", "--name-only"]), "unrelated.txt");
        assert!(status(&root).unwrap().changes.is_empty());
        assert!(commit(&root, "project", &fingerprint, "again").unwrap_err().contains("Nothing to commit"));

        assert!(push(&root, "project", Duration::from_secs(30)).unwrap_err().contains("no remote"));
        let remote = TempDir::new();
        sh(&remote.0, &["init", "-q", "--bare"]);
        sh(&repo.0, &["remote", "add", "origin", remote.0.to_str().unwrap()]);
        assert!(push(&root, "project", Duration::from_secs(30)).unwrap_err().contains("no upstream"));
        sh(&repo.0, &["push", "-q", "-u", "origin", "HEAD"]);
        std::fs::write(root.join("documents/architecture/overview.md"), "# Overview\n\nChanged\n").unwrap();
        let fingerprint = project::disk_fingerprint(&root, "project").unwrap();
        commit(&root, "project", &fingerprint, "docs: second").unwrap();
        let ahead = status(&root).unwrap();
        assert_eq!((ahead.ahead, ahead.behind, ahead.upstream.as_deref()), (1, 0, Some("origin/main")));
        // push.default=matching must not push other branches: only HEAD to its upstream.
        sh(&repo.0, &["config", "push.default", "matching"]);
        sh(&repo.0, &["branch", "side"]);
        sh(&repo.0, &["push", "-q", "origin", "side"]);
        sh(&repo.0, &["checkout", "-q", "side"]);
        commit_file(&repo.0, "side.txt", "side", "side work");
        sh(&repo.0, &["checkout", "-q", "main"]);
        assert_eq!(push(&root, "project", Duration::from_secs(30)).unwrap(), "Pushed main to origin.");
        assert_ne!(sh(&remote.0, &["log", "-1", "--format=%s", "side"]), "side work");
        assert_eq!(status(&root).unwrap().ahead, 0);
        assert_eq!(sh(&remote.0, &["log", "-1", "--format=%s"]), "docs: second");
    }
}
