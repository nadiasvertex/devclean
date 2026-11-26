use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::finding::{Category, Finding};

/// One row in the clean checklist. Build folders are grouped by project so that a
/// workspace with thousands of `bin/` and `obj/` folders stays a short list.
pub struct Entry {
    pub indices: Vec<usize>,
    pub bytes: u64,
    pub checked: bool,
    pub group_root: Option<PathBuf>,
}

/// The nearest git work tree (repo or linked worktree) at or above the path, else the
/// parent folder.
pub fn project_root(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|dir| dir.join(".git").exists())
        .or_else(|| path.parent())
        .unwrap_or(path)
        .to_path_buf()
}

/// Build-output groups come first, largest first, and start checked. Every other
/// finding is its own row and starts unchecked.
pub fn entries(findings: &[Finding], root_of: impl Fn(&Path) -> PathBuf) -> Vec<Entry> {
    let mut groups: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    let mut singles = Vec::new();
    for (i, f) in findings.iter().enumerate() {
        match (&f.category, &f.path) {
            (Category::Build, Some(path)) => groups.entry(root_of(path)).or_default().push(i),
            _ => singles.push(i),
        }
    }
    let size = |indices: &[usize]| indices.iter().map(|&i| findings[i].bytes).sum();
    let mut out: Vec<Entry> = groups
        .into_iter()
        .map(|(root, indices)| Entry {
            bytes: size(&indices),
            indices,
            checked: true,
            group_root: Some(root),
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.bytes));
    out.extend(singles.into_iter().map(|i| Entry {
        indices: vec![i],
        bytes: findings[i].bytes,
        checked: false,
        group_root: None,
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finding::Action;

    fn finding(category: Category, path: &str, bytes: u64) -> Finding {
        Finding {
            category,
            label: String::new(),
            path: Some(PathBuf::from(path)),
            detail: None,
            bytes,
            age_days: None,
            action: Action::RemoveDir(PathBuf::from(path)),
        }
    }

    #[test]
    fn groups_build_folders_by_project_and_checks_them() {
        let findings = vec![
            finding(Category::Build, "/r/a/src/x/bin", 10),
            finding(Category::Worktree, "/r/a/wt", 500),
            finding(Category::Build, "/r/a/src/y/obj", 20),
            finding(Category::Build, "/r/b/bin", 100),
        ];
        let root_of = |p: &Path| p.iter().take(4).collect::<PathBuf>();
        let rows = entries(&findings, root_of);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].group_root.as_deref(), Some(Path::new("/r/b/bin")));
        assert_eq!((rows[0].bytes, rows[0].checked), (100, true));
        assert_eq!(rows[1].group_root.as_deref(), Some(Path::new("/r/a/src")));
        assert_eq!(
            (rows[1].indices.clone(), rows[1].bytes, rows[1].checked),
            (vec![0, 2], 30, true)
        );
        assert_eq!((rows[2].indices.clone(), rows[2].checked), (vec![1], false));
    }
}
