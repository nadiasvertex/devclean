use std::path::PathBuf;

use rayon::prelude::*;

use crate::finding::{Action, Category, Finding};
use crate::util::{dir_usage, home, on_path, output};

struct CacheSpec {
    name: &'static str,
    path: fn() -> Option<PathBuf>,
    /// Tool-native cleanup command, used when the tool is installed.
    command: Option<(&'static str, &'static [&'static str])>,
    note: Option<&'static str>,
}

fn under_home(rel: &'static str) -> Option<PathBuf> {
    Some(home().join(rel))
}

/// The per-user cache folder: `~/Library/Caches/<mac>` on macOS, otherwise
/// `$XDG_CACHE_HOME/<linux>` or `~/.cache/<linux>`.
fn user_cache(mac: &str, linux: &str) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return Some(home().join("Library/Caches").join(mac));
    }
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".cache"));
    Some(base.join(linux))
}

fn mac_only(rel: &'static str) -> Option<PathBuf> {
    cfg!(target_os = "macos").then(|| home().join(rel))
}

fn specs() -> Vec<CacheSpec> {
    vec![
        CacheSpec {
            name: "Cargo registry cache",
            path: || under_home(".cargo/registry/cache"),
            command: None,
            note: Some("crates are downloaded again on next build"),
        },
        CacheSpec {
            name: "Cargo registry sources",
            path: || under_home(".cargo/registry/src"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "npm cache",
            path: || under_home(".npm/_cacache"),
            command: Some(("npm", &["cache", "clean", "--force"])),
            note: None,
        },
        CacheSpec {
            name: "Yarn cache",
            path: || under_home("Library/Caches/Yarn"),
            command: Some(("yarn", &["cache", "clean"])),
            note: None,
        },
        CacheSpec {
            name: "pnpm store",
            path: || output("pnpm", &["store", "path"], None).map(PathBuf::from),
            command: Some(("pnpm", &["store", "prune"])),
            note: Some("prune removes only unreferenced packages"),
        },
        CacheSpec {
            name: "NuGet caches",
            path: || under_home(".nuget/packages"),
            command: Some(("dotnet", &["nuget", "locals", "all", "--clear"])),
            note: Some("also clears http-cache and temp"),
        },
        CacheSpec {
            name: "Gradle caches",
            path: || under_home(".gradle/caches"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "Maven repository",
            path: || under_home(".m2/repository"),
            command: None,
            note: Some("also holds artifacts from `mvn install`"),
        },
        CacheSpec {
            name: "Homebrew downloads",
            path: || output("brew", &["--cache"], None).map(PathBuf::from),
            command: Some(("brew", &["cleanup", "--prune=all"])),
            note: Some("also removes old installed versions"),
        },
        CacheSpec {
            name: "Go build cache",
            path: || output("go", &["env", "GOCACHE"], None).map(PathBuf::from),
            command: Some(("go", &["clean", "-cache"])),
            note: None,
        },
        CacheSpec {
            name: "Go module cache",
            path: || output("go", &["env", "GOMODCACHE"], None).map(PathBuf::from),
            command: Some(("go", &["clean", "-modcache"])),
            note: None,
        },
        CacheSpec {
            name: "pip cache",
            path: || under_home("Library/Caches/pip"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "uv cache",
            path: || under_home(".cache/uv"),
            command: Some(("uv", &["cache", "clean"])),
            note: None,
        },
        CacheSpec {
            name: "Playwright browsers",
            path: || under_home("Library/Caches/ms-playwright"),
            command: None,
            note: Some("reinstall with `npx playwright install`"),
        },
        CacheSpec {
            name: "Xcode DerivedData",
            path: || under_home("Library/Developer/Xcode/DerivedData"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "iOS simulators (unavailable)",
            path: || under_home("Library/Developer/CoreSimulator/Devices"),
            command: Some(("xcrun", &["simctl", "delete", "unavailable"])),
            note: Some("size shown is all simulators; only unavailable ones are deleted"),
        },
        CacheSpec {
            name: "Terraform plugin cache",
            path: || under_home(".terraform.d/plugin-cache"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "JetBrains caches",
            path: || under_home("Library/Caches/JetBrains"),
            command: None,
            note: Some("includes caches for old IDE versions"),
        },
        CacheSpec {
            name: "Bazel output and repository cache",
            path: || user_cache("bazel", "bazel"),
            command: None,
            note: Some("every Bazel workspace rebuilds from scratch; run `bazel shutdown` first"),
        },
        CacheSpec {
            name: "Bazelisk downloads",
            path: || user_cache("bazelisk", "bazelisk"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "ccache",
            path: || {
                output("ccache", &["-k", "cache_dir"], None)
                    .map(PathBuf::from)
                    .or_else(|| user_cache("ccache", "ccache"))
            },
            command: Some(("ccache", &["-C"])),
            note: Some("the next C and C++ builds compile everything again"),
        },
        CacheSpec {
            name: "sccache",
            path: || user_cache("Mozilla.sccache", "sccache"),
            command: None,
            note: Some("run `sccache --stop-server` first if it is running"),
        },
        CacheSpec {
            name: "JetBrains logs",
            path: || mac_only("Library/Logs/JetBrains"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "Copilot search index",
            path: || user_cache("copilot/tgrep-index", "copilot/tgrep-index"),
            command: None,
            note: Some("Copilot rebuilds the index when it next searches"),
        },
        CacheSpec {
            name: "Copilot packages",
            path: || user_cache("copilot/pkg", "copilot/pkg"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "pixi (rattler) cache",
            path: || user_cache("rattler", "rattler"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "Gradle wrapper distributions",
            path: || under_home(".gradle/wrapper/dists"),
            command: None,
            note: Some("each project downloads its Gradle version again on next build"),
        },
        CacheSpec {
            name: "Coursier cache",
            path: || user_cache("Coursier", "coursier"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "CocoaPods cache",
            path: || user_cache("CocoaPods", "CocoaPods"),
            command: Some(("pod", &["cache", "clean", "--all"])),
            note: None,
        },
        CacheSpec {
            name: "Electron downloads",
            path: || user_cache("electron", "electron"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "node-gyp headers",
            path: || user_cache("node-gyp", "node-gyp"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "npx packages",
            path: || under_home(".npm/_npx"),
            command: None,
            note: None,
        },
        CacheSpec {
            name: "SwiftPM cache",
            path: || user_cache("org.swift.swiftpm", "org.swift.swiftpm"),
            command: None,
            note: None,
        },
    ]
}

pub fn cache_findings() -> Vec<Finding> {
    specs()
        .into_par_iter()
        .filter_map(|spec| {
            let path = (spec.path)().filter(|p| p.is_dir())?;
            let action = match spec.command {
                Some((program, args)) if on_path(program) => Action::run(program, args, None),
                Some(_) | None => Action::RemoveDir(path.clone()),
            };
            Some(Finding {
                category: Category::Cache,
                label: spec.name.to_string(),
                bytes: dir_usage(&path).bytes,
                path: Some(path),
                detail: spec.note.map(String::from),
                age_days: None,
                action,
            })
        })
        .collect()
}
