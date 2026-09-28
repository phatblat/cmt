use std::str::FromStr;

use crate::change::{Change, Status};
use crate::intent;
use crate::message::CommitType;

/// One hand-labelled fixture: a diff, the type a human says it deserves, and
/// the reasoning behind that label. Decision clause 21.
#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub expected: CommitType,
    pub rationale: String,
    pub body: Option<String>,
    pub files: Vec<String>,
    pub diff: String,
}

/// Reads `key = "value"`. `Ok(None)` when the key is absent; `Err` when the
/// key is present but its value fails to parse as a JSON string — a
/// spacing or quoting mistake that would otherwise silently produce an
/// empty string.
fn field(raw: &str, key: &str) -> Result<Option<String>, String> {
    let Some(value) = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key} = ")))
        .map(str::trim)
    else {
        return Ok(None);
    };
    serde_json::from_str::<String>(value)
        .map(Some)
        .map_err(|e| format!("`{key}` is not a valid JSON string ({value:?}): {e}"))
}

/// As `field`, for a `key = ["a", "b"]` line; absent is `Ok(Vec::new())`.
fn list(raw: &str, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key} = ")))
        .map(str::trim)
    else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<String>>(value)
        .map_err(|e| format!("`{key}` is not a valid JSON array ({value:?}): {e}"))
}

impl Case {
    pub fn parse(name: &str, raw: &str) -> Result<Self, String> {
        let wrap = |e: String| format!("case `{name}`: {e}");
        let expected = field(raw, "expected").map_err(wrap)?.unwrap_or_default();
        if expected.is_empty() {
            return Err(format!(
                "case `{name}` is unlabelled: set `expected` to a commit type before scoring it"
            ));
        }
        Ok(Self {
            name: name.to_string(),
            expected: CommitType::from_str(&expected)?,
            rationale: field(raw, "rationale").map_err(wrap)?.unwrap_or_default(),
            body: field(raw, "body").map_err(wrap)?.filter(|b| !b.is_empty()),
            files: list(raw, "files").map_err(wrap)?,
            diff: field(raw, "diff").map_err(wrap)?.unwrap_or_default(),
        })
    }

    /// Every `.toml` under `dir`, sorted by name.
    pub fn load_dir(dir: &std::path::Path) -> Result<Vec<Self>, String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .into_iter()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        entries.sort();
        entries
            .iter()
            .map(|path| {
                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Self::parse(&name, &raw)
            })
            .collect()
    }

    /// Whether the fixture's files include a test path, by the same rule
    /// `intent::has_tests` applies to a real change set.
    pub fn has_tests(&self) -> bool {
        let changes: Vec<Change> = self
            .files
            .iter()
            .map(|f| Change {
                path: f.clone(),
                status: Status::Modified,
            })
            .collect();
        intent::has_tests(&changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_labelled_case() {
        let raw = r#"
expected = "feat"
rationale = "Adds a flag that did not exist."
body = "before 10ms after 4ms"
files = ["src/main.rs"]
diff = "@@ -1 +1 @@\n+flag\n"
"#;
        let case = Case::parse("new-flag", raw).expect("parse");

        assert_eq!(case.name, "new-flag");
        assert_eq!(case.expected, CommitType::Feat);
        assert_eq!(case.files, vec!["src/main.rs"]);
        assert!(case.body.is_some());
    }

    #[test]
    fn a_malformed_files_array_fails_loudly_instead_of_defaulting() {
        let raw = r#"
expected = "feat"
rationale = "r"
body = ""
files = ['src/main.rs']
diff = "@@\n"
"#;
        let err = Case::parse("bad-quotes", raw).expect_err("should reject");

        assert!(err.contains("files"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_an_unlabelled_case() {
        let raw = "expected = \"\"\nrationale = \"\"\nfiles = []\ndiff = \"\"\n";
        let err = Case::parse("blank", raw).expect_err("should reject");

        assert!(err.contains("unlabelled"), "unexpected error: {err}");
    }

    #[test]
    fn has_tests_reads_the_files_like_a_real_change_set() {
        let with_test = Case::parse(
            "x",
            r#"expected = "fix"
rationale = "r"
body = ""
files = ["src/a.rs", "tests/a.rs"]
diff = ""
"#,
        )
        .expect("parse");
        let without_test = Case::parse(
            "y",
            r#"expected = "fix"
rationale = "r"
body = ""
files = ["src/a.rs"]
diff = ""
"#,
        )
        .expect("parse");

        assert!(with_test.has_tests());
        assert!(!without_test.has_tests());
    }
}
