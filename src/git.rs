use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::finding::{Action, Category, Finding};
use crate::util::{days_since_unix, dir_usage, git};

struct Worktree {
    path: PathBuf,
    branch: Option<String>,
    prunable: Option<String>,
    locked: bool,
}

fn list_worktrees(repo: &Path) -> Vec<Worktree> {
    let Some(out) = git(repo, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    out.split("\n\n")
        .skip(1) // the main worktree
        .filter_map(|block| {
            let mut wt = Worktree {
                path: PathBuf::new(),
                branch: None,
                prunable: None,
                locked: false,
            };
            for line in block.lines() {
                let (key, value) = line.split_once(' ').unwrap_or((line, ""));
                match key {
                    "worktree" => wt.path = PathBuf::from(value),
                    "branch" => wt.branch = Some(value.trim_start_matches("refs/heads/").into()),
                    "prunable" => wt.prunable = Some(value.to_string()),
                    "locked" => wt.locked = true,
                    "bare" => return None,
                    _ => {}
                }
            }
            (!wt.path.as_os_str().is_empty()).then_some(wt)
        })
        .collect()
}

fn default_remote_ref(repo: &Path) -> Option<String> {
    git(
        repo,
        &["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"],
    )
    .or_else(|| {
        ["origin/master", "origin/main"]
            .into_iter()
            .find(|r| git(repo, &["rev-parse", "--verify", "-q", r]).is_some())
            .map(String::from)
    })
}

/// Worktrees that are missing on disk, or clean and merged, orphaned, or idle.
/// Removing a worktree keeps its branch, so only uncommitted work could be lost,
/// and `git worktree remove` refuses to run when there is any.
pub fn worktree_findings(repo: &Path, idle_days: u64) -> Vec<Finding> {
    let default_ref = default_remote_ref(repo);
    list_worktrees(repo)
        .into_iter()
        .filter(|wt| !wt.locked)
        .filter_map(|wt| {
            if let Some(reason) = &wt.prunable {
                return Some(Finding {
                    category: Category::Worktree,
                    label: "missing worktree".into(),
                    path: Some(wt.path.clone()),
                    detail: Some(format!("{reason}; prunes the record in {}", repo.display())),
                    bytes: 0,
                    age_days: None,
                    action: Action::run("git", &["worktree", "prune"], Some(repo.to_path_buf())),
                });
            }
            let status = git(&wt.path, &["status", "--porcelain"])?;
            if !status.is_empty() {
                return None;
            }
            let age = git(&wt.path, &["log", "-1", "--format=%ct"])
                .and_then(|s| s.parse().ok())
                .map(days_since_unix);
            let merged = default_ref.as_deref().is_some_and(|r| {
                git(&wt.path, &["merge-base", "--is-ancestor", "HEAD", r]).is_some()
            });
            let gone = wt.branch.as_deref().is_some_and(|b| {
                git(
                    repo,
                    &[
                        "for-each-ref",
                        "--format=%(upstream:track)",
                        &format!("refs/heads/{b}"),
                    ],
                )
                .is_some_and(|t| t == "[gone]")
            });
            let reason = if merged {
                format!("merged into {}", default_ref.as_deref().unwrap_or_default())
            } else if gone {
                "upstream branch was deleted".to_string()
            } else if age.is_some_and(|a| a >= idle_days) {
                format!("no commits in {} days", age.unwrap_or_default())
            } else {
                return None;
            };
            let branch = wt.branch.as_deref().unwrap_or("detached HEAD");
            Some(Finding {
                category: Category::Worktree,
                label: "worktree".into(),
                path: Some(wt.path.clone()),
                detail: Some(format!("{branch}: {reason}; branch is kept")),
                bytes: dir_usage(&wt.path).bytes,
                age_days: age,
                action: Action::run(
                    "git",
                    &["worktree", "remove", &wt.path.to_string_lossy()],
                    Some(repo.to_path_buf()),
                ),
            })
        })
        .collect()
}

/// Repositories with many loose objects, many packs, or garbage files.
pub fn gc_finding(repo: &Path) -> Option<Finding> {
    let out = git(repo, &["count-objects", "-v"])?;
    let stats: HashMap<&str, u64> = out
        .lines()
        .filter_map(|l| l.split_once(": "))
        .filter_map(|(k, v)| Some((k, v.parse().ok()?)))
        .collect();
    let stat = |k: &str| stats.get(k).copied().unwrap_or(0);
    let (loose, loose_kib, packs) = (stat("count"), stat("size"), stat("packs"));
    let (garbage, garbage_kib) = (stat("garbage"), stat("size-garbage"));
    if loose < 1000 && packs < 10 && garbage == 0 {
        return None;
    }
    let mut detail = format!("{loose} loose objects, {packs} packs");
    if garbage > 0 {
        detail.push_str(&format!(", {garbage} garbage files"));
    }
    Some(Finding {
        category: Category::GitGc,
        label: "git gc".into(),
        path: Some(repo.to_path_buf()),
        detail: Some(detail),
        bytes: (loose_kib + garbage_kib) * 1024,
        age_days: None,
        action: Action::run("git", &["gc"], Some(repo.to_path_buf())),
    })
}
