use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
pub enum Category {
    Build,
    Worktree,
    GitGc,
    Cache,
    Toolchain,
    Docker,
}

impl Category {
    pub fn title(self) -> &'static str {
        match self {
            Category::Build => "Build output and dependency folders",
            Category::Worktree => "Git worktrees",
            Category::GitGc => "Git repositories that can be compacted",
            Category::Cache => "Global tool caches",
            Category::Toolchain => "Superseded toolchains, SDKs, and emulators",
            Category::Docker => "Docker",
        }
    }

    pub fn is_global(self) -> bool {
        matches!(
            self,
            Category::Cache | Category::Toolchain | Category::Docker
        )
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    RemoveDir(PathBuf),
    /// Removes each file or directory in the list.
    RemovePaths(Vec<PathBuf>),
    Run {
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
    /// A step devclean reports but does not run, such as one that needs sudo.
    Manual(String),
}

impl Action {
    pub fn run(program: &str, args: &[&str], cwd: Option<PathBuf>) -> Action {
        Action::Run {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Action::RemoveDir(path) => format!("rm -rf {}", path.display()),
            Action::RemovePaths(paths) => std::iter::once("rm -rf".to_string())
                .chain(paths.iter().map(|p| p.display().to_string()))
                .collect::<Vec<_>>()
                .join(" "),
            Action::Run { program, args, cwd } => {
                let cmd = std::iter::once(program.as_str())
                    .chain(args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" ");
                match cwd {
                    Some(dir) => format!("(in {}) {cmd}", dir.display()),
                    None => cmd,
                }
            }
            Action::Manual(step) => step.clone(),
        }
    }

    pub fn execute(&self) -> Result<()> {
        match self {
            Action::RemoveDir(path) => remove(path),
            Action::RemovePaths(paths) => paths.iter().try_for_each(|p| remove(p)),
            Action::Run { program, args, cwd } => {
                let mut cmd = Command::new(program);
                cmd.args(args);
                if let Some(dir) = cwd {
                    cmd.current_dir(dir);
                }
                let status = cmd.status().with_context(|| format!("running {program}"))?;
                if !status.success() {
                    bail!("`{}` exited with {status}", self.describe());
                }
                Ok(())
            }
            Action::Manual(step) => bail!("needs a manual step: {step}"),
        }
    }

    pub fn is_manual(&self) -> bool {
        matches!(self, Action::Manual(_))
    }
}

/// Removes a file or directory tree. Some tools, such as Bazel, leave read-only
/// directories whose entries cannot be unlinked, so a failed removal restores write
/// permission on every directory and tries once more.
fn remove(path: &Path) -> Result<()> {
    let Ok(meta) = path.symlink_metadata() else {
        return Ok(());
    };
    let context = || format!("removing {}", path.display());
    if !meta.is_dir() {
        return std::fs::remove_file(path).with_context(context);
    }
    if std::fs::remove_dir_all(path).is_ok() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if entry.file_type().is_dir() {
            let mut perms = entry.metadata()?.permissions();
            perms.set_mode(perms.mode() | 0o700);
            std::fs::set_permissions(entry.path(), perms).with_context(context)?;
        }
    }
    std::fs::remove_dir_all(path).with_context(context)
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub category: Category,
    pub label: String,
    pub path: Option<PathBuf>,
    pub detail: Option<String>,
    /// Estimated reclaimable bytes.
    pub bytes: u64,
    pub age_days: Option<u64>,
    pub action: Action,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_a_tree_with_read_only_directories() {
        let root = std::env::temp_dir().join(format!("devclean-ro-{}", std::process::id()));
        let locked = root.join("external/lib");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("file"), "x").unwrap();
        for dir in [&locked, &root.join("external")] {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }

        let result = Action::RemoveDir(root.clone()).execute();

        assert!(result.is_ok(), "{result:?}");
        assert!(!root.exists());
    }
}
