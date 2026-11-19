use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, ValueEnum)]
pub enum Category {
    Build,
    Worktree,
    GitGc,
    Cache,
    Docker,
}

impl Category {
    pub fn title(self) -> &'static str {
        match self {
            Category::Build => "Build output and dependency folders",
            Category::Worktree => "Git worktrees",
            Category::GitGc => "Git repositories that can be compacted",
            Category::Cache => "Global tool caches",
            Category::Docker => "Docker",
        }
    }

    pub fn is_global(self) -> bool {
        matches!(self, Category::Cache | Category::Docker)
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    RemoveDir(PathBuf),
    Run {
        program: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
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
        }
    }

    pub fn execute(&self) -> Result<()> {
        match self {
            Action::RemoveDir(path) => {
                if !path.exists() {
                    return Ok(());
                }
                std::fs::remove_dir_all(path)
                    .with_context(|| format!("removing {}", path.display()))
            }
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
        }
    }
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
