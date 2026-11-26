use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::finding::{Action, Category, Finding};
use crate::util::{days_since, dir_usage, home, on_path, output};

/// Machine-wide toolchains and emulators that a newer install supersedes or that
/// nothing appears to use. `pin_files` are the `rust-toolchain` files found under
/// the scanned roots.
pub fn toolchain_findings(pin_files: &[PathBuf]) -> Vec<Finding> {
    let sources: Vec<Box<dyn Fn() -> Vec<Finding> + Sync>> = vec![
        Box::new(android_emulators),
        Box::new(android_ndks),
        Box::new(jetbrains_versions),
        Box::new(|| rustup_toolchains(pin_files)),
        Box::new(dotnet_sdks),
    ];
    sources
        .par_iter()
        .flat_map_iter(|source| source())
        .collect()
}

fn finding(label: &str, path: PathBuf, detail: String, action: Action) -> Finding {
    let usage = dir_usage(&path);
    Finding {
        category: Category::Toolchain,
        label: label.into(),
        path: Some(path),
        detail: Some(detail),
        bytes: usage.bytes,
        age_days: usage.newest.map(days_since),
        action,
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Sort key for versions such as "8.0.415", "2026.3", or "8.0.100-preview.4".
/// A pre-release sorts below the release with the same numbers.
fn version_key(version: &str) -> (Vec<u64>, bool) {
    let (base, prerelease) = match version.split_once('-') {
        Some((base, _)) => (base, true),
        None => (version, false),
    };
    let numbers = base.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    (numbers, !prerelease)
}

/// Every item except the newest version in its group, each paired with that newest
/// version.
fn superseded<T>(
    items: Vec<T>,
    group: impl Fn(&T) -> String,
    version: impl Fn(&T) -> String,
) -> Vec<(T, String)> {
    let mut groups: BTreeMap<String, Vec<T>> = BTreeMap::new();
    for item in items {
        groups.entry(group(&item)).or_default().push(item);
    }
    groups
        .into_values()
        .flat_map(|mut members| {
            members.sort_by_key(|m| version_key(&version(m)));
            let newest = members.pop().map(|m| version(&m)).unwrap_or_default();
            members.into_iter().map(move |m| (m, newest.clone()))
        })
        .collect()
}

fn android_sdk() -> PathBuf {
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .find_map(std::env::var_os)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home().join(if cfg!(target_os = "macos") {
                "Library/Android/sdk"
            } else {
                "Android/Sdk"
            })
        })
}

fn android_emulators() -> Vec<Finding> {
    let dir = std::env::var_os("ANDROID_AVD_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".android/avd"));
    subdirs(&dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "avd"))
        .map(|avd| {
            let name = avd
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let action = if on_path("avdmanager") {
                Action::run("avdmanager", &["delete", "avd", "-n", &name], None)
            } else {
                Action::RemovePaths(vec![avd.clone(), dir.join(format!("{name}.ini"))])
            };
            let detail =
                format!("{name}: disk images and snapshots; recreate it in the Device Manager");
            finding("Android emulator", avd, detail, action)
        })
        .collect()
}

fn android_ndks() -> Vec<Finding> {
    let ndks = subdirs(&android_sdk().join("ndk"));
    superseded(ndks, |_| String::new(), |p| file_name(p))
        .into_iter()
        .map(|(path, newest)| {
            let version = file_name(&path);
            let detail = format!(
                "{version}; {newest} is kept; projects that pin ndkVersion {version} need it again"
            );
            finding(
                "older Android NDK",
                path.clone(),
                detail,
                Action::RemoveDir(path),
            )
        })
        .collect()
}

/// Splits a JetBrains folder name such as "CLionNova2023.3" into product and version.
fn jetbrains_version(name: &str) -> Option<(&str, &str)> {
    let split = name.find(|c: char| c.is_ascii_digit())?;
    let (product, version) = name.split_at(split);
    let (year, minor) = version.split_once('.')?;
    let all_digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    (!product.is_empty()
        && product.chars().all(|c| c.is_ascii_alphabetic())
        && year.len() == 4
        && all_digits(year)
        && all_digits(minor))
    .then_some((product, version))
}

fn jetbrains_versions() -> Vec<Finding> {
    let bases = if cfg!(target_os = "macos") {
        vec![home().join("Library/Application Support/JetBrains")]
    } else {
        vec![
            home().join(".config/JetBrains"),
            home().join(".local/share/JetBrains"),
        ]
    };
    let dirs: Vec<PathBuf> = bases
        .iter()
        .flat_map(|base| subdirs(base))
        .filter(|p| jetbrains_version(&file_name(p)).is_some())
        .collect();
    let product_of = |p: &PathBuf| {
        let name = file_name(p);
        let product = jetbrains_version(&name).map(|(prod, _)| prod.to_string());
        format!(
            "{}/{}",
            p.parent().unwrap_or(p).display(),
            product.unwrap_or_default()
        )
    };
    let version_of = |p: &PathBuf| {
        let name = file_name(p);
        jetbrains_version(&name)
            .map(|(_, v)| v.to_string())
            .unwrap_or_default()
    };
    superseded(dirs, product_of, version_of)
        .into_iter()
        .map(|(path, newest)| {
            let name = file_name(&path);
            let detail = format!(
                "{name}: settings and plugins for that version; {newest} is kept; \
                 skip this if you still run {name}"
            );
            finding(
                "old JetBrains IDE data",
                path.clone(),
                detail,
                Action::RemoveDir(path),
            )
        })
        .collect()
}

/// The toolchain a `rust-toolchain` or `rust-toolchain.toml` file asks for.
fn parse_pin(contents: &str) -> Option<String> {
    let channel = contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_string())
    });
    if channel.is_some() || contents.contains("[toolchain]") {
        return channel.filter(|c| !c.is_empty());
    }
    contents
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(String::from)
}

fn pinned(toolchain: &str, pins: &[String]) -> bool {
    pins.iter().any(|pin| {
        toolchain == pin
            || toolchain
                .strip_prefix(pin.as_str())
                .is_some_and(|rest| rest.starts_with('-'))
    })
}

fn rustup_toolchains(pin_files: &[PathBuf]) -> Vec<Finding> {
    if !on_path("rustup") {
        return Vec::new();
    }
    let mut pins: Vec<String> = pin_files
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .filter_map(|c| parse_pin(&c))
        .collect();
    if let Some(overrides) = output("rustup", &["override", "list"], None) {
        pins.extend(
            overrides
                .lines()
                .filter_map(|l| l.split_whitespace().last())
                .filter(|t| *t != "overrides")
                .map(String::from),
        );
    }
    let Some(list) = output("rustup", &["toolchain", "list", "-v"], None) else {
        return Vec::new();
    };
    list.lines()
        .filter(|line| !line.contains("default") && !line.contains("active"))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?.to_string();
            let path = PathBuf::from(words.last()?);
            (!pinned(&name, &pins) && path.is_dir()).then_some((name, path))
        })
        .map(|(name, path)| {
            let detail = format!(
                "{name}: not the default and not pinned under the scanned roots; \
                 rustup installs it again if a project asks for it"
            );
            let action = Action::run("rustup", &["toolchain", "uninstall", &name], None);
            finding("unused Rust toolchain", path, detail, action)
        })
        .collect()
}

fn dotnet_sdks() -> Vec<Finding> {
    if !on_path("dotnet") {
        return Vec::new();
    }
    let Some(list) = output("dotnet", &["--list-sdks"], None) else {
        return Vec::new();
    };
    let sdks: Vec<(String, PathBuf)> = list
        .lines()
        .filter_map(|line| {
            let (version, dir) = line.split_once(" [")?;
            let path = PathBuf::from(dir.trim_end_matches(']')).join(version);
            Some((version.to_string(), path))
        })
        .collect();
    let major = |v: &str| v.split('.').next().unwrap_or_default().to_string();
    superseded(sdks, |(v, _)| major(v), |(v, _)| v.clone())
        .into_iter()
        .map(|((version, path), newest)| {
            let pinned_note = format!("global.json files that pin {version} need it");
            let (action, how) = if path.starts_with(home()) {
                (Action::RemoveDir(path.clone()), String::new())
            } else {
                let step = format!("sudo dotnet-core-uninstall remove --sdk {version}");
                (
                    Action::Manual(step),
                    "; needs sudo, so devclean only reports it".into(),
                )
            };
            let detail = format!(
                "{version}: {newest} is kept for .NET {}{how}; {pinned_note}",
                major(&version)
            );
            finding("older .NET SDK", path, detail, action)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_versions_numerically_with_prereleases_first() {
        let mut versions = vec!["8.0.415", "8.0.100-preview.4.23260.5", "8.0.99", "8.0.100"];
        versions.sort_by_key(|v| version_key(v));
        assert_eq!(
            versions,
            ["8.0.99", "8.0.100-preview.4.23260.5", "8.0.100", "8.0.415"]
        );
    }

    #[test]
    fn keeps_the_newest_version_in_each_group() {
        let sdks = vec!["8.0.204", "10.0.200", "8.0.415", "9.0.101", "10.0.300"];
        let major = |v: &&str| v.split('.').next().unwrap().to_string();
        let mut old = superseded(sdks, major, |v| v.to_string());
        old.sort();
        assert_eq!(
            old,
            [
                ("10.0.200", "10.0.300".to_string()),
                ("8.0.204", "8.0.415".to_string())
            ]
        );
    }

    #[test]
    fn splits_jetbrains_folder_names() {
        assert_eq!(
            jetbrains_version("CLionNova2023.3"),
            Some(("CLionNova", "2023.3"))
        );
        assert_eq!(
            jetbrains_version("IntelliJIdea2026.3"),
            Some(("IntelliJIdea", "2026.3"))
        );
        assert_eq!(jetbrains_version("Toolbox"), None);
        assert_eq!(jetbrains_version("consentOptions"), None);
        assert_eq!(jetbrains_version("Rider2026"), None);
        assert_eq!(jetbrains_version("Python3.11"), None);
    }

    #[test]
    fn reads_rust_toolchain_pins() {
        assert_eq!(
            parse_pin("[toolchain]\nchannel = \"1.100.0\"\nprofile = \"minimal\"\n"),
            Some("1.100.0".into())
        );
        assert_eq!(
            parse_pin("\nnightly-2023-05-28\n"),
            Some("nightly-2023-05-28".into())
        );
        assert_eq!(parse_pin("[toolchain]\nprofile = \"minimal\"\n"), None);
    }

    #[test]
    fn matches_pins_against_host_qualified_names() {
        let pins = vec!["1.100.0".to_string(), "nightly".to_string()];
        assert!(pinned("1.100.0-aarch64-apple-darwin", &pins));
        assert!(pinned("nightly-aarch64-apple-darwin", &pins));
        assert!(!pinned("1.100.01-aarch64-apple-darwin", &pins));
        assert!(!pinned("1.82.0-aarch64-apple-darwin", &pins));
    }
}
