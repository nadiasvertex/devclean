use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use walkdir::WalkDir;

pub struct Usage {
    pub bytes: u64,
    pub newest: Option<SystemTime>,
}

/// On-disk usage of a directory tree, plus the newest modification time inside it.
pub fn dir_usage(path: &Path) -> Usage {
    let mut bytes = 0;
    let mut newest: Option<SystemTime> = None;
    for entry in WalkDir::new(path).follow_links(false).into_iter().flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        bytes += meta.blocks() * 512;
        if let Ok(modified) = meta.modified() {
            newest = Some(newest.map_or(modified, |n| n.max(modified)));
        }
    }
    Usage { bytes, newest }
}

pub fn days_since(time: SystemTime) -> u64 {
    SystemTime::now()
        .duration_since(time)
        .unwrap_or_default()
        .as_secs()
        / 86_400
}

pub fn days_since_unix(secs: u64) -> u64 {
    days_since(UNIX_EPOCH + Duration::from_secs(secs))
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Runs a command and returns trimmed stdout if it exits successfully.
pub fn output(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git(repo: &Path, args: &[&str]) -> Option<String> {
    output("git", args, Some(repo))
}

pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

pub fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(Into::into).unwrap_or_default()
}
