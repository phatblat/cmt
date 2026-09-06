use crate::change::{Change, Status};
use crate::message::CommitType;
use crate::scope;
use crate::subject;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Todo,
    Plan,
    Decision,
    Ignore,
    Ci,
    Lock,
    Manifest,
    Test,
    Docs,
    Build,
    Source,
}

const CI_FILES: [&str; 4] = [
    ".gitlab-ci.yml",
    ".travis.yml",
    "azure-pipelines.yml",
    "Jenkinsfile",
];
const LOCKFILES: [&str; 9] = [
    "Cargo.lock",
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "yarn.lock",
    "pnpm-lock.yaml",
    "uv.lock",
    "poetry.lock",
    "go.sum",
];
const MANIFESTS: [&str; 4] = ["Cargo.toml", "package.json", "pyproject.toml", "go.mod"];
const TEST_DIRS: [&str; 4] = ["tests", "test", "__tests__", "spec"];
const DOC_EXTS: [&str; 4] = ["md", "mdx", "rst", "txt"];
const BUILD_FILES: [&str; 11] = [
    "mise.toml",
    ".tool-versions",
    "justfile",
    "Justfile",
    "Makefile",
    "build.rs",
    ".editorconfig",
    "rustfmt.toml",
    "clippy.toml",
    "commitlint.config.js",
    "commitlint.config.mjs",
];

/// First path component when the path has a directory part.
fn first_dir(path: &str) -> Option<&str> {
    path.split_once('/').map(|(dir, _)| dir)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn extension(base: &str) -> &str {
    base.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")
}

/// `YYYY-MM-DD-slug` with a kebab-case slug.
fn is_decision_id(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
    b.len() > 11
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'-'
        && b[11..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The id of a decision record at `docs/decisions/<id>.md`.
pub fn decision_stem(path: &str) -> Option<&str> {
    let stem = path.strip_prefix("docs/decisions/")?.strip_suffix(".md")?;
    is_decision_id(stem).then_some(stem)
}

/// First `docs/decisions/<id>.md` reference in free text.
pub fn find_decision_id(text: &str) -> Option<String> {
    text.match_indices("docs/decisions/")
        .find_map(|(at, prefix)| {
            let rest = &text[at + prefix.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
                .unwrap_or(rest.len());
            let id = &rest[..end];
            (rest[end..].starts_with(".md") && is_decision_id(id)).then(|| id.to_string())
        })
}

fn is_test_name(base: &str) -> bool {
    let stem = base.rsplit_once('.').map(|(stem, _)| stem);
    stem.is_some_and(|s| s.ends_with("_test") || s.ends_with(".test") || s.ends_with(".spec"))
        || (base.starts_with("test_") && extension(base) == "py")
        || base.ends_with("Tests.swift")
}

pub fn category(change: &Change) -> Category {
    let path = change.path.as_str();
    let base = basename(path);
    let ext = extension(base);
    let dir = first_dir(path);
    if path == "TODO.md" {
        return Category::Todo;
    }
    if path == "PLAN.md" && matches!(change.status, Status::Added | Status::Deleted) {
        return Category::Plan;
    }
    if change.status == Status::Added && decision_stem(path).is_some() {
        return Category::Decision;
    }
    if base.ends_with("ignore") || base == ".gitattributes" {
        return Category::Ignore;
    }
    if path.starts_with(".github/workflows/")
        || path.starts_with(".circleci/")
        || CI_FILES.contains(&base)
    {
        return Category::Ci;
    }
    if LOCKFILES.contains(&base) {
        return Category::Lock;
    }
    if MANIFESTS.contains(&base) {
        return Category::Manifest;
    }
    if dir.is_some_and(|d| TEST_DIRS.contains(&d)) || is_test_name(base) {
        return Category::Test;
    }
    if dir == Some("docs") || DOC_EXTS.contains(&ext) {
        return Category::Docs;
    }
    if BUILD_FILES.contains(&base)
        || base.starts_with("Dockerfile")
        || base.starts_with("tsconfig")
        || base.starts_with(".prettierrc")
        || ext == "dockerfile"
        || dir == Some(".husky")
    {
        return Category::Build;
    }
    Category::Source
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub kind: CommitType,
    pub scope: Option<String>,
    pub subject: String,
    pub changes: Vec<Change>,
}

impl Group {
    fn mechanical(kind: CommitType, scope: Option<String>, changes: Vec<Change>) -> Option<Self> {
        if changes.is_empty() {
            return None;
        }
        let subject = subject::subject(&changes);
        Some(Self {
            kind,
            scope,
            subject,
            changes,
        })
    }
}

/// Split changes into one commit per logical group, in an order that keeps
/// the tree building after each commit: spec artifacts, then config and
/// dependencies, then source (with its tests), then docs and CI, then the
/// session todo. `read` resolves `HEAD:<path>` / `:<path>` for `plan:` ids.
pub fn group(changes: &[Change], read: &dyn Fn(&str) -> Option<String>) -> Vec<Group> {
    let mut decisions = Vec::new();
    let mut plans = Vec::new();
    let mut ignore = Vec::new();
    let mut ci = Vec::new();
    let mut lock = Vec::new();
    let mut manifest = Vec::new();
    let mut test = Vec::new();
    let mut docs = Vec::new();
    let mut build = Vec::new();
    let mut source = Vec::new();
    let mut todo = Vec::new();
    for change in changes {
        let bucket = match category(change) {
            Category::Decision => &mut decisions,
            Category::Plan => &mut plans,
            Category::Ignore => &mut ignore,
            Category::Ci => &mut ci,
            Category::Lock => &mut lock,
            Category::Manifest => &mut manifest,
            Category::Test => &mut test,
            Category::Docs => &mut docs,
            Category::Build => &mut build,
            Category::Source => &mut source,
            Category::Todo => &mut todo,
        };
        bucket.push(change.clone());
    }

    let mut groups = Vec::new();
    for change in decisions {
        let stem = decision_stem(&change.path).expect("categorised as a decision");
        groups.push(Group {
            kind: CommitType::Decision,
            scope: None,
            subject: format!("propose {stem}"),
            changes: vec![change],
        });
    }
    for change in plans {
        let (verb, spec) = if change.status == Status::Added {
            ("start", ":PLAN.md")
        } else {
            ("done", "HEAD:PLAN.md")
        };
        let subject = match read(spec).and_then(|text| find_decision_id(&text)) {
            Some(id) => format!("{verb} {id}"),
            None => verb.to_string(),
        };
        groups.push(Group {
            kind: CommitType::Plan,
            scope: None,
            subject,
            changes: vec![change],
        });
    }
    groups.extend(Group::mechanical(CommitType::Ignore, None, ignore));
    if lock.is_empty() {
        build.append(&mut manifest);
        groups.extend(Group::mechanical(CommitType::Build, None, build));
    } else {
        groups.extend(Group::mechanical(CommitType::Build, None, build));
        lock.append(&mut manifest);
        lock.sort_by(|a, b| a.path.cmp(&b.path));
        groups.extend(Group::mechanical(CommitType::Deps, None, lock));
    }
    if source.is_empty() {
        groups.extend(Group::mechanical(CommitType::Test, None, test));
    } else {
        let source_paths: Vec<&str> = source.iter().map(|c| c.path.as_str()).collect();
        let scope = scope::scope(&source_paths);
        source.append(&mut test);
        groups.extend(Group::mechanical(CommitType::Refactor, scope, source));
    }
    groups.extend(Group::mechanical(CommitType::Docs, None, docs));
    groups.extend(Group::mechanical(CommitType::Ci, None, ci));
    for change in todo {
        let subject = if change.status == Status::Deleted {
            "clear"
        } else {
            "sync"
        };
        groups.push(Group {
            kind: CommitType::Todo,
            scope: None,
            subject: subject.into(),
            changes: vec![change],
        });
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modified(path: &str) -> Change {
        Change {
            path: path.into(),
            status: Status::Modified,
        }
    }

    fn with(path: &str, status: Status) -> Change {
        Change {
            path: path.into(),
            status,
        }
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn category_per_cascade_row() {
        let cases: [(&str, Status, Category); 30] = [
            ("TODO.md", Status::Modified, Category::Todo),
            ("PLAN.md", Status::Added, Category::Plan),
            ("PLAN.md", Status::Deleted, Category::Plan),
            ("PLAN.md", Status::Modified, Category::Docs),
            (
                "docs/decisions/2026-09-06-x.md",
                Status::Added,
                Category::Decision,
            ),
            (
                "docs/decisions/2026-09-06-x.md",
                Status::Modified,
                Category::Docs,
            ),
            ("docs/decisions/notes.md", Status::Added, Category::Docs),
            (".gitignore", Status::Added, Category::Ignore),
            ("web/.dockerignore", Status::Modified, Category::Ignore),
            (".gitattributes", Status::Modified, Category::Ignore),
            (".github/workflows/ci.yml", Status::Added, Category::Ci),
            (".circleci/config.yml", Status::Added, Category::Ci),
            ("Jenkinsfile", Status::Added, Category::Ci),
            ("Cargo.lock", Status::Modified, Category::Lock),
            ("web/bun.lock", Status::Modified, Category::Lock),
            ("Cargo.toml", Status::Modified, Category::Manifest),
            ("web/package.json", Status::Modified, Category::Manifest),
            ("tests/cli.rs", Status::Added, Category::Test),
            ("src/foo_test.go", Status::Added, Category::Test),
            ("src/foo.test.ts", Status::Added, Category::Test),
            ("src/foo.spec.ts", Status::Added, Category::Test),
            ("pkg/test_foo.py", Status::Added, Category::Test),
            ("Sources/FooTests.swift", Status::Added, Category::Test),
            ("docs/guide.md", Status::Modified, Category::Docs),
            ("README.md", Status::Modified, Category::Docs),
            ("notes.txt", Status::Modified, Category::Docs),
            ("justfile", Status::Modified, Category::Build),
            ("Dockerfile.ci", Status::Added, Category::Build),
            (".husky/pre-commit", Status::Added, Category::Build),
            ("src/main.rs", Status::Modified, Category::Source),
        ];
        for (path, status, expected) in cases {
            assert_eq!(category(&with(path, status)), expected, "{path}");
        }
    }

    #[test]
    fn full_mix_splits_into_ordered_groups() {
        let changes = [
            ".gitignore",
            "Cargo.toml",
            "Cargo.lock",
            "justfile",
            "src/a.rs",
            "tests/a.rs",
            "README.md",
            ".github/workflows/ci.yml",
            "TODO.md",
        ]
        .map(modified);
        let groups = group(&changes, &none);
        let kinds: Vec<CommitType> = groups.iter().map(|g| g.kind).collect();
        assert_eq!(
            kinds,
            [
                CommitType::Ignore,
                CommitType::Build,
                CommitType::Deps,
                CommitType::Refactor,
                CommitType::Docs,
                CommitType::Ci,
                CommitType::Todo,
            ]
        );
        let paths = |g: &Group| g.changes.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&groups[1]), ["justfile"]);
        assert_eq!(paths(&groups[2]), ["Cargo.lock", "Cargo.toml"]);
        assert_eq!(paths(&groups[3]), ["src/a.rs", "tests/a.rs"]);
        assert_eq!(groups[3].subject, "update 2 files");
        assert_eq!(groups[6].subject, "sync");
    }

    #[test]
    fn manifest_without_lockfile_is_build() {
        let groups = group(&["Cargo.toml", "justfile"].map(modified), &none);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Build);
        assert_eq!(groups[0].changes.len(), 2);
    }

    #[test]
    fn tests_alone_are_a_test_commit() {
        let groups = group(&[modified("tests/a.rs")], &none);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Test);
    }

    #[test]
    fn tests_ride_with_source_and_scope_ignores_them() {
        let changes =
            ["src/parser/a.rs", "src/parser/b.rs", "tests/p.rs"].map(|p| with(p, Status::Added));
        let groups = group(&changes, &none);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Refactor);
        assert_eq!(groups[0].scope, Some("parser".into()));
        assert_eq!(groups[0].subject, "add 3 files");
    }

    #[test]
    fn plan_done_reads_id_from_head() {
        let head = |spec: &str| {
            assert_eq!(spec, "HEAD:PLAN.md");
            Some("see docs/decisions/2026-09-06-x.md for the spec".to_string())
        };
        let groups = group(&[with("PLAN.md", Status::Deleted)], &head);
        assert_eq!(groups[0].kind, CommitType::Plan);
        assert_eq!(groups[0].subject, "done 2026-09-06-x");
    }

    #[test]
    fn plan_without_decision_link_has_bare_subject() {
        let groups = group(&[with("PLAN.md", Status::Added)], &none);
        assert_eq!(groups[0].subject, "start");
    }

    #[test]
    fn todo_deleted_is_clear() {
        let groups = group(&[with("TODO.md", Status::Deleted)], &none);
        assert_eq!(groups[0].kind, CommitType::Todo);
        assert_eq!(groups[0].subject, "clear");
    }

    #[test]
    fn decision_added_is_proposed() {
        let groups = group(
            &[with("docs/decisions/2026-09-06-split-it.md", Status::Added)],
            &none,
        );
        assert_eq!(groups[0].kind, CommitType::Decision);
        assert_eq!(groups[0].subject, "propose 2026-09-06-split-it");
    }
}
