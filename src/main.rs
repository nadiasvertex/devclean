mod caches;
mod docker;
mod finding;
mod git;
mod util;
mod walk;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use dialoguer::{Confirm, MultiSelect};
use rayon::prelude::*;

use finding::{Category, Finding};
use util::{home, human};

#[derive(Parser)]
#[command(
    version,
    about = "Find and reclaim disk space used by development artifacts"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report what could be cleaned. Changes nothing.
    Scan(ScanArgs),
    /// Pick findings interactively and clean them.
    Clean {
        #[command(flatten)]
        scan: ScanArgs,
        /// Clean every finding without prompting.
        #[arg(long)]
        yes: bool,
        /// Show what would run without doing it.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Args)]
struct ScanArgs {
    /// Directories to search for projects and repos (default: current directory).
    roots: Vec<PathBuf>,
    /// Limit to these categories (comma-separated).
    #[arg(long, value_enum, value_delimiter = ',')]
    only: Vec<Category>,
    /// Skip machine-wide categories (tool caches and Docker).
    #[arg(long)]
    no_global: bool,
    /// Only report items untouched for at least this many days.
    /// Also sets the idle threshold for worktrees (default 30).
    #[arg(long, value_name = "DAYS")]
    older_than: Option<u64>,
    /// Hide build folders and caches smaller than this many MiB.
    #[arg(long, value_name = "MIB", default_value_t = 10)]
    min_size: u64,
    /// List every finding instead of the largest per category.
    #[arg(long)]
    all: bool,
}

const ROWS_PER_CATEGORY: usize = 15;

impl ScanArgs {
    fn wants(&self, category: Category) -> bool {
        (self.only.is_empty() || self.only.contains(&category))
            && !(self.no_global && category.is_global())
    }
}

fn scan(args: &ScanArgs) -> Vec<Finding> {
    let roots: Vec<PathBuf> = if args.roots.is_empty() {
        vec![std::env::current_dir().unwrap_or_else(|_| ".".into())]
    } else {
        args.roots
            .iter()
            .map(|r| r.canonicalize().unwrap_or_else(|_| r.clone()))
            .collect()
    };

    let needs_walk = [Category::Build, Category::Worktree, Category::GitGc]
        .into_iter()
        .any(|c| args.wants(c));
    let walked = if needs_walk {
        eprintln!(
            "Scanning {} ...",
            roots
                .iter()
                .map(|r| tilde(r))
                .collect::<Vec<_>>()
                .join(", ")
        );
        walk::walk(&roots)
    } else {
        walk::WalkResult {
            build_dirs: Vec::new(),
            repos: Vec::new(),
        }
    };

    let mut findings = Vec::new();
    if args.wants(Category::Build) {
        findings.extend(walk::build_findings(walked.build_dirs));
    }
    let idle_days = args.older_than.unwrap_or(30);
    findings.extend(
        walked
            .repos
            .par_iter()
            .flat_map_iter(|repo| {
                let mut found = Vec::new();
                if args.wants(Category::Worktree) {
                    found.extend(git::worktree_findings(repo, idle_days));
                }
                if args.wants(Category::GitGc) {
                    found.extend(git::gc_finding(repo));
                }
                found
            })
            .collect::<Vec<_>>(),
    );
    if args.wants(Category::Cache) {
        findings.extend(caches::cache_findings());
    }
    if args.wants(Category::Docker) {
        findings.extend(docker::docker_findings());
    }

    let min_bytes = args.min_size * 1024 * 1024;
    findings.retain(|f| {
        let size_ok =
            !matches!(f.category, Category::Build | Category::Cache) || f.bytes >= min_bytes;
        let age_ok = match (args.older_than, f.age_days) {
            (Some(min), Some(age)) => age >= min,
            _ => true,
        };
        size_ok && age_ok
    });
    findings.sort_by(|a, b| a.category.cmp(&b.category).then(b.bytes.cmp(&a.bytes)));
    findings
}

fn tilde(path: &Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn line(f: &Finding) -> String {
    let age = f.age_days.map(|d| format!("{d}d")).unwrap_or_default();
    let path = f.path.as_deref().map(tilde).unwrap_or_default();
    format!(
        "{:>10}  {:>5}  {:<28}  {}",
        human(f.bytes),
        age,
        f.label,
        path
    )
}

/// Sum of sizes, skipping findings whose path lies inside another finding's path.
fn total_bytes(findings: &[&Finding]) -> u64 {
    findings
        .iter()
        .filter(|f| {
            let Some(path) = &f.path else { return true };
            !findings.iter().any(|outer| {
                // `git gc` compacts a repo but does not remove its contents.
                outer.category != Category::GitGc
                    && outer
                        .path
                        .as_ref()
                        .is_some_and(|o| o != path && path.starts_with(o))
            })
        })
        .map(|f| f.bytes)
        .sum()
}

fn print_report(findings: &[Finding], all: bool) {
    if findings.is_empty() {
        println!("Nothing to clean.");
        return;
    }
    let mut categories: Vec<Category> = findings.iter().map(|f| f.category).collect();
    categories.dedup();
    for category in categories {
        let group: Vec<&Finding> = findings.iter().filter(|f| f.category == category).collect();
        let total: u64 = group.iter().map(|f| f.bytes).sum();
        println!(
            "\n{} ({} items, {})",
            category.title(),
            group.len(),
            human(total)
        );
        let shown = if all {
            group.len()
        } else {
            ROWS_PER_CATEGORY.min(group.len())
        };
        for f in &group[..shown] {
            println!("{}", line(f));
            if let Some(detail) = &f.detail {
                println!("{:>19}{detail}", "");
            }
        }
        if shown < group.len() {
            let rest: u64 = group[shown..].iter().map(|f| f.bytes).sum();
            println!(
                "{:>19}... {} more ({}); use --all to list them",
                "",
                group.len() - shown,
                human(rest)
            );
        }
    }
    let total = total_bytes(&findings.iter().collect::<Vec<_>>());
    println!(
        "\nEstimated total: {} (nested items counted once)",
        human(total)
    );
}

fn clean(findings: Vec<Finding>, yes: bool, dry_run: bool) -> Result<()> {
    if findings.is_empty() {
        println!("Nothing to clean.");
        return Ok(());
    }
    let chosen: Vec<Finding> = if yes {
        findings
    } else {
        if !std::io::stdin().is_terminal() {
            bail!("not a terminal; pass --yes to clean everything found, or run `scan` instead");
        }
        let items: Vec<String> = findings
            .iter()
            .map(|f| format!("[{:?}] {}", f.category, line(f)))
            .collect();
        let Some(picked) = MultiSelect::new()
            .with_prompt("Select items to clean (space toggles, a selects all, enter confirms)")
            .items(&items)
            .max_length(25)
            .interact_opt()?
        else {
            return Ok(());
        };
        picked.into_iter().map(|i| findings[i].clone()).collect()
    };
    if chosen.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }

    let total = total_bytes(&chosen.iter().collect::<Vec<_>>());
    if dry_run {
        for f in &chosen {
            println!("would run: {}", f.action.describe());
        }
        println!("Would reclaim about {}.", human(total));
        return Ok(());
    }
    if !yes
        && !Confirm::new()
            .with_prompt(format!(
                "Clean {} items (about {})?",
                chosen.len(),
                human(total)
            ))
            .default(false)
            .interact()?
    {
        return Ok(());
    }

    let mut reclaimed = 0;
    let mut failures = 0;
    for f in &chosen {
        println!("{}", f.action.describe());
        match f.action.execute() {
            Ok(()) => reclaimed += f.bytes,
            Err(err) => {
                failures += 1;
                eprintln!("  failed: {err:#}");
            }
        }
    }
    println!("Reclaimed about {}. {failures} failed.", human(reclaimed));
    if failures > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Scan(args) => {
            print_report(&scan(&args), args.all);
            Ok(())
        }
        Command::Clean {
            scan: args,
            yes,
            dry_run,
        } => clean(scan(&args), yes, dry_run),
    }
}
