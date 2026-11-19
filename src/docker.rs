use crate::finding::{Action, Category, Finding};
use crate::util::{on_path, output};

/// Parses Docker's SI size strings, such as "1.25GB (40%)" or "512kB".
fn parse_size(s: &str) -> u64 {
    let s = s.split_whitespace().next().unwrap_or("");
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (number, unit) = s.split_at(split);
    let number: f64 = number.parse().unwrap_or(0.0);
    let scale = match unit.to_ascii_uppercase().as_str() {
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => 1.0,
    };
    (number * scale) as u64
}

pub fn docker_findings() -> Vec<Finding> {
    if !on_path("docker") {
        return Vec::new();
    }
    let Some(out) = output(
        "docker",
        &["system", "df", "--format", "{{.Type}}\t{{.Reclaimable}}"],
        None,
    ) else {
        eprintln!("note: skipping Docker; the daemon is not reachable");
        return Vec::new();
    };
    let mut general = 0;
    let mut volumes = 0;
    for line in out.lines() {
        let Some((kind, reclaimable)) = line.split_once('\t') else {
            continue;
        };
        let bytes = parse_size(reclaimable);
        if kind == "Local Volumes" {
            volumes += bytes;
        } else {
            general += bytes;
        }
    }
    let mut findings = Vec::new();
    if general > 0 {
        findings.push(Finding {
            category: Category::Docker,
            label: "docker system prune".into(),
            path: None,
            detail: Some(
                "stopped containers, unused networks, dangling images, build cache; \
                 estimate includes unused tagged images, which only `-a` removes"
                    .into(),
            ),
            bytes: general,
            age_days: None,
            action: Action::run("docker", &["system", "prune", "--force"], None),
        });
    }
    if volumes > 0 {
        findings.push(Finding {
            category: Category::Docker,
            label: "docker volume prune".into(),
            path: None,
            detail: Some("unused anonymous volumes; these can hold local database data".into()),
            bytes: volumes,
            age_days: None,
            action: Action::run("docker", &["volume", "prune", "--force"], None),
        });
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn parses_docker_sizes() {
        assert_eq!(parse_size("1.5GB (40%)"), 1_500_000_000);
        assert_eq!(parse_size("512kB"), 512_000);
        assert_eq!(parse_size("0B"), 0);
        assert_eq!(parse_size("12B (100%)"), 12);
    }
}
