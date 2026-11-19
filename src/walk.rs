use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rayon::prelude::*;
use walkdir::WalkDir;

use crate::finding::{Action, Category, Finding};
use crate::util::{days_since, dir_usage};

pub struct WalkResult {
    pub build_dirs: Vec<(PathBuf, &'static str)>,
    pub repos: Vec<PathBuf>,
}

/// Walks the roots once, collecting build-output candidates and main git repositories.
/// Does not descend into `.git` or into any directory it classifies as build output.
pub fn walk(roots: &[PathBuf]) -> WalkResult {
    let mut build_dirs = Vec::new();
    let mut repos = Vec::new();
    for root in roots {
        let mut it = WalkDir::new(root).follow_links(false).into_iter();
        while let Some(entry) = it.next() {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if name == ".git" {
                if let Some(parent) = entry.path().parent() {
                    repos.push(parent.to_path_buf());
                }
                it.skip_current_dir();
            } else if let Some(kind) = classify(entry.path(), &name) {
                build_dirs.push((entry.path().to_path_buf(), kind));
                it.skip_current_dir();
            }
        }
    }
    WalkResult { build_dirs, repos }
}

fn classify(path: &Path, name: &str) -> Option<&'static str> {
    let parent = path.parent()?;
    let has = |file: &str| parent.join(file).exists();
    let has_ext = |exts: &[&str]| {
        std::fs::read_dir(parent).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| exts.iter().any(|ext| x == *ext))
            })
        })
    };
    let node = || has("package.json");
    let gradle = || {
        [
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ]
        .iter()
        .any(|f| has(f))
    };

    let kind = match name {
        "target" if has("Cargo.toml") => "Rust target",
        "target" if has("pom.xml") => "Maven target",
        "node_modules" if node() => "node_modules",
        "bin" | "obj" if has_ext(&["csproj", "fsproj", "vbproj"]) => ".NET bin/obj",
        ".venv" | "venv" | "env" if path.join("pyvenv.cfg").exists() => "Python virtualenv",
        "__pycache__" | ".pytest_cache" | ".mypy_cache" | ".ruff_cache" | ".tox" => "Python cache",
        ".next" | ".nuxt" | ".svelte-kit" | ".turbo" | ".parcel-cache" | ".angular" | ".vite"
            if node() =>
        {
            "JS framework cache"
        }
        "dist" | "build" | "out" | "coverage" if node() => "JS build output",
        ".gradle" | "build" if gradle() => "Gradle build",
        "build" if has("CMakeLists.txt") => "CMake build",
        n if n.starts_with("cmake-build-") => "CMake build",
        ".terraform" if has_ext(&["tf"]) => "Terraform providers",
        ".build" if has("Package.swift") => "Swift build",
        ".zig-cache" | "zig-cache" | "zig-out" if has("build.zig") => "Zig build",
        _ => return None,
    };
    Some(kind)
}

/// True unless the directory sits in a git work tree and is not ignored there,
/// which means it may hold tracked or deliberately kept files.
fn safe_to_remove(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let status = Command::new("git")
        .arg("check-ignore")
        .arg("-q")
        .arg(name)
        .current_dir(parent)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // 0 = ignored, 1 = not ignored, 128 = not in a repository.
    !matches!(status.map(|s| s.code()), Ok(Some(1)))
}

pub fn build_findings(dirs: Vec<(PathBuf, &'static str)>) -> Vec<Finding> {
    dirs.into_par_iter()
        .filter(|(path, _)| safe_to_remove(path))
        .map(|(path, kind)| {
            let usage = dir_usage(&path);
            Finding {
                category: Category::Build,
                label: kind.to_string(),
                path: Some(path.clone()),
                detail: None,
                bytes: usage.bytes,
                age_days: usage.newest.map(days_since),
                action: Action::RemoveDir(path),
            }
        })
        .collect()
}
