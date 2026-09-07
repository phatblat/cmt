use std::collections::BTreeMap;

/// Lockfiles whose `[[package]]` tables carry `name`/`version` pairs.
pub const TOML_LOCKS: [&str; 3] = ["Cargo.lock", "uv.lock", "poetry.lock"];

/// `<pkg> <from> -> <to>` for every package whose single version moved
/// between `before` and `after`; added, removed, and multi-version packages
/// are skipped (the convention has no Bumps form for them).
pub fn bumps(before: &str, after: &str) -> Vec<String> {
    let before = versions_by_name(before);
    let after = versions_by_name(after);
    before
        .iter()
        .filter_map(|(name, before_versions)| {
            let after_versions = after.get(name)?;
            let ([from], [to]) = (before_versions.as_slice(), after_versions.as_slice()) else {
                return None;
            };
            (from != to).then(|| format!("{name} {from} -> {to}"))
        })
        .collect()
}

fn versions_by_name(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, version) in packages(text) {
        by_name.entry(name).or_default().push(version);
    }
    by_name
}

/// Every `name`/`version` pair in the file's `[[package]]` tables, in order.
fn packages(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_package = false;
    let mut name: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[[package]]";
            if in_package {
                name = None;
            }
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name") {
            if let Some(v) = parse_toml_string(value) {
                name = Some(v);
            }
        } else if let Some(value) = trimmed.strip_prefix("version")
            && let (Some(n), Some(v)) = (name.take(), parse_toml_string(value))
        {
            out.push((n, v));
        }
    }
    out
}

/// `= "value"`, with optional surrounding whitespace, to `value`.
fn parse_toml_string(after_key: &str) -> Option<String> {
    let value = after_key.trim_start().strip_prefix('=')?.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bumps_reports_only_single_version_moves() {
        let before = r#"
[[package]]
name = "serde"
version = "1.0.0"

[[package]]
name = "removed"
version = "0.1.0"

[[package]]
name = "multi"
version = "1.0.0"

[[package]]
name = "multi"
version = "2.0.0"
"#;
        let after = r#"
[[package]]
name = "serde"
version = "1.0.1"

[[package]]
name = "added"
version = "0.1.0"

[[package]]
name = "multi"
version = "1.0.0"

[[package]]
name = "multi"
version = "3.0.0"
"#;
        assert_eq!(bumps(before, after), vec!["serde 1.0.0 -> 1.0.1"]);
    }
}
