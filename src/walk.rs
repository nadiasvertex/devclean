use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rayon::prelude::*;
use walkdir::WalkDir;

use crate::finding::{Action, Category, Finding};
use crate::pick::project_root;
use crate::util::{days_since, dir_usage, home};

#[derive(Default)]
pub struct WalkResult {
    pub build_dirs: Vec<(PathBuf, &'static str)>,
    pub repos: Vec<PathBuf>,
    /// LLVM `*.profraw` coverage files outside build folders.
    pub coverage_files: Vec<PathBuf>,
    /// `rust-toolchain` and `rust-toolchain.toml` files.
    pub toolchain_files: Vec<PathBuf>,
}

/// Walks the roots once, collecting build-output candidates, main git repositories,
/// coverage files, and Rust toolchain pins. Does not descend into `.git` or into any
/// directory it classifies as build output.
pub fn walk(roots: &[PathBuf]) -> WalkResult {
    let mut found = WalkResult::default();
    for root in roots {
        let mut it = WalkDir::new(root).follow_links(false).into_iter();
        while let Some(entry) = it.next() {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().is_file() {
                if name == "rust-toolchain" || name == "rust-toolchain.toml" {
                    found.toolchain_files.push(entry.into_path());
                } else if name.ends_with(".profraw") {
                    found.coverage_files.push(entry.into_path());
                }
                continue;
            }
            if !entry.file_type().is_dir() {
                continue;
            }
            if name == ".git" {
                if let Some(parent) = entry.path().parent() {
                    found.repos.push(parent.to_path_buf());
                }
                it.skip_current_dir();
            } else if let Some(kind) = classify(entry.path(), &name) {
                found.build_dirs.push((entry.path().to_path_buf(), kind));
                it.skip_current_dir();
            }
        }
    }
    found
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

    let inside = |file: &str| path.join(file).exists();
    // An in-source CMake build puts CMakeCache.txt next to CMakeLists.txt; that folder
    // is the project itself, not build output.
    if inside("CMakeCache.txt") && !inside("CMakeLists.txt") && !inside(".git") {
        return Some("CMake build");
    }
    let mm = || has("mmpackage.json");
    let c_project = || has("compile_commands.json") || has(".clangd") || has("CMakeLists.txt");

    let kind = match name {
        ".build" | "dist" if mm() => "mm build output",
        ".mm" if mm() => "mm packages",
        ".cache" if c_project() && parent != home() => "clangd and IDE index",
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

/// True unless the path sits in a git work tree and is not ignored there,
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

/// True when git tracks the file, so removing it would change the work tree.
fn tracked(path: &Path) -> bool {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return true;
    };
    Command::new("git")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(name)
        .current_dir(parent)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// One finding per project for its untracked `*.profraw` files, which test runs built
/// with coverage leave in whatever directory they ran from.
pub fn coverage_findings(files: Vec<PathBuf>) -> Vec<Finding> {
    let mut by_project: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for file in files
        .into_par_iter()
        .filter(|f| !tracked(f))
        .collect::<Vec<_>>()
    {
        by_project
            .entry(project_root(&file))
            .or_default()
            .push(file);
    }
    by_project
        .into_iter()
        .map(|(root, files)| {
            let usage: Vec<_> = files.iter().map(|f| dir_usage(f)).collect();
            let n = files.len();
            Finding {
                category: Category::Build,
                label: "LLVM coverage files".into(),
                path: Some(root),
                detail: Some(format!(
                    "{n} *.profraw file{}",
                    if n == 1 { "" } else { "s" }
                )),
                bytes: usage.iter().map(|u| u.bytes).sum(),
                age_days: usage.iter().filter_map(|u| u.newest).max().map(days_since),
                action: Action::RemovePaths(files),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("devclean-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn kind(path: &Path) -> Option<&'static str> {
        classify(path, &path.file_name().unwrap().to_string_lossy())
    }

    #[test]
    fn classifies_mm_output_next_to_mmpackage_json() {
        let root = scratch("mm");
        touch(&root.join("proj/mmpackage.json"));
        for dir in [".build", ".mm", "dist"] {
            std::fs::create_dir_all(root.join("proj").join(dir)).unwrap();
        }
        std::fs::create_dir_all(root.join("other/.build")).unwrap();

        assert_eq!(kind(&root.join("proj/.build")), Some("mm build output"));
        assert_eq!(kind(&root.join("proj/dist")), Some("mm build output"));
        assert_eq!(kind(&root.join("proj/.mm")), Some("mm packages"));
        assert_eq!(kind(&root.join("other/.build")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn classifies_any_cmake_build_tree_but_not_an_in_source_build() {
        let root = scratch("cmake");
        touch(&root.join("proj/CMakeLists.txt"));
        touch(&root.join("proj/.build/debug/CMakeCache.txt"));
        touch(&root.join("insrc/CMakeLists.txt"));
        touch(&root.join("insrc/CMakeCache.txt"));

        assert_eq!(kind(&root.join("proj/.build/debug")), Some("CMake build"));
        assert_eq!(kind(&root.join("proj/.build")), None);
        assert_eq!(kind(&root.join("insrc")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn classifies_clangd_cache_only_in_a_c_project() {
        let root = scratch("clangd");
        touch(&root.join("proj/compile_commands.json"));
        std::fs::create_dir_all(root.join("proj/.cache/clangd")).unwrap();
        std::fs::create_dir_all(root.join("plain/.cache")).unwrap();

        assert_eq!(
            kind(&root.join("proj/.cache")),
            Some("clangd and IDE index")
        );
        assert_eq!(kind(&root.join("plain/.cache")), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn groups_coverage_files_by_project() {
        let root = scratch("coverage");
        std::fs::create_dir_all(root.join("a/.git")).unwrap();
        std::fs::create_dir_all(root.join("b/.git")).unwrap();
        let files = [
            root.join("a/default.profraw"),
            root.join("a/test/x/default.profraw"),
            root.join("b/default.profraw"),
        ];
        files.iter().for_each(|f| touch(f));

        let found = coverage_findings(files.to_vec());

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path.as_deref(), Some(root.join("a").as_path()));
        assert_eq!(found[0].detail.as_deref(), Some("2 *.profraw files"));
        assert_eq!(found[1].path.as_deref(), Some(root.join("b").as_path()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
