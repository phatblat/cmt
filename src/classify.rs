use crate::bumps;
use crate::change::{Change, Status};
use crate::message::CommitType;
use crate::scope;
use crate::subject;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Todo,
    Plan,
    Changelog,
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

const CI_PREFIXES: [&str; 5] = [
    ".github/workflows/",
    ".github/actions/",
    ".circleci/",
    ".buildkite/",
    ".woodpecker/",
];
const GITHUB_CI_FILES: [&str; 5] = [
    ".github/dependabot.yml",
    ".github/renovate.json",
    ".github/renovate.json5",
    ".github/labeler.yml",
    ".github/release.yml",
];
const CI_FILES: [&str; 11] = [
    ".gitlab-ci.yml",
    ".travis.yml",
    "azure-pipelines.yml",
    "Jenkinsfile",
    ".drone.yml",
    "bitbucket-pipelines.yml",
    "cloudbuild.yaml",
    "codecov.yml",
    ".codecov.yml",
    "renovate.json",
    "renovate.json5",
];
const LOCKFILES: [&str; 20] = [
    "Cargo.lock",
    "package-lock.json",
    "bun.lock",
    "bun.lockb",
    "yarn.lock",
    "pnpm-lock.yaml",
    "uv.lock",
    "poetry.lock",
    "go.sum",
    "Gemfile.lock",
    "composer.lock",
    "Package.resolved",
    "Podfile.lock",
    "Pipfile.lock",
    "flake.lock",
    "mix.lock",
    "pubspec.lock",
    "deno.lock",
    "gradle.lockfile",
    "go.work.sum",
];
const MANIFESTS: [&str; 20] = [
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "Gemfile",
    "composer.json",
    "Package.swift",
    "Podfile",
    "Pipfile",
    "setup.py",
    "setup.cfg",
    "flake.nix",
    "mix.exs",
    "pubspec.yaml",
    "deno.json",
    "deno.jsonc",
    "go.work",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
];
const TEST_DIRS: [&str; 6] = [
    "tests",
    "test",
    "__tests__",
    "spec",
    "testdata",
    "__snapshots__",
];
const DOC_EXTS: [&str; 4] = ["md", "mdx", "rst", "txt"];
const BUILD_FILES: [&str; 40] = [
    "mise.toml",
    ".tool-versions",
    "justfile",
    "Justfile",
    "Makefile",
    "build.rs",
    ".editorconfig",
    "rustfmt.toml",
    "clippy.toml",
    "CMakeLists.txt",
    "meson.build",
    "settings.gradle",
    "settings.gradle.kts",
    "gradle.properties",
    "gradlew",
    "gradlew.bat",
    "Taskfile.yml",
    "Taskfile.yaml",
    ".pre-commit-config.yaml",
    "deny.toml",
    "rust-toolchain",
    "rust-toolchain.toml",
    "biome.json",
    "biome.jsonc",
    ".nvmrc",
    ".node-version",
    ".python-version",
    ".ruby-version",
    "Brewfile",
    "Brewfile.lock.json",
    "mise.lock",
    ".releaserc",
    ".releaserc.json",
    ".markdownlint.json",
    ".markdownlint.yaml",
    ".markdownlint-cli2.jsonc",
    ".swiftlint.yml",
    ".swiftformat",
    "compose.yaml",
    "compose.yml",
];
const BUILD_PREFIXES: [&str; 16] = [
    "Dockerfile",
    "tsconfig",
    ".prettierrc",
    "commitlint.config.",
    "eslint.config.",
    ".eslintrc",
    "vite.config.",
    "vitest.config.",
    "webpack.config.",
    "rollup.config.",
    "jest.config.",
    "babel.config.",
    ".babelrc",
    "docker-compose.",
    "lefthook.",
    ".lefthook.",
];
const BUILD_DIRS: [&str; 6] = [".husky", ".cargo", ".vscode", ".idea", ".zed", "gradle"];

/// First path component when the path has a directory part.
fn first_dir(path: &str) -> Option<&str> {
    path.split_once('/').map(|(dir, _)| dir)
}

/// Every path component except the basename, in order.
fn dirs(path: &str) -> impl Iterator<Item = &str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.into_iter()
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

/// The lifecycle state named by a decision's `## Status` line, per
/// conventional-docs; also the verb of the `decision:` event it produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionStatus {
    Draft,
    Proposed,
    Accepted,
    Rejected,
}

impl DecisionStatus {
    pub fn verb(self) -> &'static str {
        match self {
            DecisionStatus::Draft => "draft",
            DecisionStatus::Proposed => "propose",
            DecisionStatus::Accepted => "accept",
            DecisionStatus::Rejected => "reject",
        }
    }
}

/// The state named by the single line under `## Status`, per conventional-docs.
pub fn decision_status(text: &str) -> Option<DecisionStatus> {
    let mut in_status = false;
    for line in text.lines() {
        if line.trim_end() == "## Status" {
            in_status = true;
            continue;
        }
        if !in_status {
            continue;
        }
        if line.starts_with("## ") {
            break;
        }
        if line.contains("**draft**") {
            return Some(DecisionStatus::Draft);
        }
        if line.contains("**awaiting review**") {
            return Some(DecisionStatus::Proposed);
        }
        if line.contains("**accepted**") {
            return Some(DecisionStatus::Accepted);
        }
        if line.contains("**rejected**") {
            return Some(DecisionStatus::Rejected);
        }
    }
    None
}

/// Versions of `## [x.y.z]` headings in a Keep a Changelog file, in order;
/// `[Unreleased]` (any case) is skipped.
pub fn released_versions(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("## [")?;
            let version = rest.split(']').next()?;
            (!version.eq_ignore_ascii_case("unreleased")).then(|| version.to_string())
        })
        .collect()
}

fn is_test_name(base: &str) -> bool {
    let stem = base.rsplit_once('.').map(|(stem, _)| stem);
    let ext = extension(base);
    stem.is_some_and(|s| s.ends_with("_test") || s.ends_with(".test") || s.ends_with(".spec"))
        || (base.starts_with("test_") && ext == "py")
        || base.ends_with("Tests.swift")
        || base.ends_with("Test.java")
        || base.ends_with("Test.kt")
        || base.ends_with("Tests.cs")
        || base.ends_with("_spec.rb")
        || base == "conftest.py"
        || base == "tests.rs"
        || ext == "snap"
}

pub fn category(change: &Change) -> Category {
    let path = change.path.as_str();
    let base = basename(path);
    let ext = extension(base);
    if path == "TODO.md" {
        return Category::Todo;
    }
    if path == "PLAN.md" {
        return Category::Plan;
    }
    if path == "CHANGELOG.md" {
        return Category::Changelog;
    }
    if matches!(
        change.status,
        Status::Added | Status::Modified | Status::Renamed { .. }
    ) && decision_stem(path).is_some()
    {
        return Category::Decision;
    }
    if base.ends_with("ignore") || base == ".gitattributes" {
        return Category::Ignore;
    }
    if CI_PREFIXES.iter().any(|p| path.starts_with(p))
        || GITHUB_CI_FILES.contains(&path)
        || CI_FILES.contains(&base)
    {
        return Category::Ci;
    }
    if LOCKFILES.contains(&base) {
        return Category::Lock;
    }
    if MANIFESTS.contains(&base) || (base.starts_with("requirements") && ext == "txt") {
        return Category::Manifest;
    }
    if dirs(path).any(|d| TEST_DIRS.contains(&d)) || is_test_name(base) {
        return Category::Test;
    }
    if BUILD_FILES.contains(&base)
        || BUILD_PREFIXES.iter().any(|p| base.starts_with(p))
        || ext == "dockerfile"
        || dirs(path).any(|d| BUILD_DIRS.contains(&d))
    {
        return Category::Build;
    }
    if first_dir(path) == Some("docs") || DOC_EXTS.contains(&ext) {
        return Category::Docs;
    }
    Category::Source
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub kind: CommitType,
    pub scope: Option<String>,
    pub subject: String,
    pub changes: Vec<Change>,
    /// true only for the refactor group built from `source` changes; the
    /// commit intent flags retarget when more than one group is inferred.
    pub source: bool,
    /// `Bumps:` values inferred from a TOML lockfile diff; empty until wired.
    pub bumps: Vec<String>,
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
            source: false,
            bumps: Vec::new(),
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
    let mut changelog: Option<Change> = None;
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
        match category(change) {
            Category::Decision => decisions.push(change.clone()),
            Category::Plan => plans.push(change.clone()),
            Category::Changelog => changelog = Some(change.clone()),
            Category::Ignore => ignore.push(change.clone()),
            Category::Ci => ci.push(change.clone()),
            Category::Lock => lock.push(change.clone()),
            Category::Manifest => manifest.push(change.clone()),
            Category::Test => test.push(change.clone()),
            Category::Docs => docs.push(change.clone()),
            Category::Build => build.push(change.clone()),
            Category::Source => source.push(change.clone()),
            Category::Todo => todo.push(change.clone()),
        }
    }

    let mut groups = Vec::new();
    for change in decisions {
        let stem = decision_stem(&change.path)
            .expect("categorised as a decision")
            .to_string();
        match change.status {
            Status::Modified => {
                let before = read(&format!("HEAD:{}", change.path))
                    .as_deref()
                    .and_then(decision_status);
                let after = read(&format!(":{}", change.path))
                    .as_deref()
                    .and_then(decision_status);
                match after {
                    Some(after) if Some(after) != before => {
                        groups.push(Group {
                            kind: CommitType::Decision,
                            scope: None,
                            subject: format!("{} {stem}", after.verb()),
                            changes: vec![change],
                            source: false,
                            bumps: Vec::new(),
                        });
                    }
                    _ => docs.push(change),
                }
            }
            _ => {
                let verb = read(&format!(":{}", change.path))
                    .as_deref()
                    .and_then(decision_status)
                    .map_or("propose", DecisionStatus::verb);
                groups.push(Group {
                    kind: CommitType::Decision,
                    scope: None,
                    subject: format!("{verb} {stem}"),
                    changes: vec![change],
                    source: false,
                    bumps: Vec::new(),
                });
            }
        }
    }
    for change in plans {
        match change.status {
            Status::Modified => {
                groups.push(Group {
                    kind: CommitType::Docs,
                    scope: None,
                    subject: "update PLAN.md".into(),
                    changes: vec![change],
                    source: false,
                    bumps: Vec::new(),
                });
            }
            Status::Deleted => {
                let subject = match read("HEAD:PLAN.md").and_then(|text| find_decision_id(&text)) {
                    Some(id) => format!("done {id}"),
                    None => "done".to_string(),
                };
                groups.push(Group {
                    kind: CommitType::Plan,
                    scope: None,
                    subject,
                    changes: vec![change],
                    source: false,
                    bumps: Vec::new(),
                });
            }
            _ => {
                let subject = match read(":PLAN.md").and_then(|text| find_decision_id(&text)) {
                    Some(id) => format!("start {id}"),
                    None => "start".to_string(),
                };
                groups.push(Group {
                    kind: CommitType::Plan,
                    scope: None,
                    subject,
                    changes: vec![change],
                    source: false,
                    bumps: Vec::new(),
                });
            }
        }
    }

    let mut release: Option<Group> = None;
    if let Some(change) = changelog.clone()
        && change.status == Status::Modified
    {
        let head_versions = read("HEAD:CHANGELOG.md")
            .as_deref()
            .map(released_versions)
            .unwrap_or_default();
        let index_versions = read(":CHANGELOG.md")
            .as_deref()
            .map(released_versions)
            .unwrap_or_default();
        if let Some(version) = index_versions.iter().find(|v| !head_versions.contains(v)) {
            let mut release_changes = vec![change];
            release_changes.append(&mut manifest);
            release_changes.append(&mut lock);
            release_changes.sort_by(|a, b| a.path.cmp(&b.path));
            release = Some(Group {
                kind: CommitType::Release,
                scope: None,
                subject: format!("v{}", version.trim_start_matches('v')),
                changes: release_changes,
                source: false,
                bumps: Vec::new(),
            });
            changelog = None;
        }
    }

    groups.extend(Group::mechanical(CommitType::Ignore, None, ignore));
    if lock.is_empty() {
        build.append(&mut manifest);
        groups.extend(Group::mechanical(CommitType::Build, None, build));
    } else {
        groups.extend(Group::mechanical(CommitType::Build, None, build));
        let mut bumps: Vec<String> = lock
            .iter()
            .filter(|c| {
                c.status == Status::Modified && bumps::TOML_LOCKS.contains(&basename(&c.path))
            })
            .filter_map(|c| {
                let before = read(&format!("HEAD:{}", c.path))?;
                let after = read(&format!(":{}", c.path))?;
                Some(bumps::bumps(&before, &after))
            })
            .flatten()
            .collect();
        bumps.sort();
        bumps.dedup();
        lock.append(&mut manifest);
        lock.sort_by(|a, b| a.path.cmp(&b.path));
        if let Some(mut g) = Group::mechanical(CommitType::Deps, None, lock) {
            g.bumps = bumps;
            groups.push(g);
        }
    }
    if source.is_empty() {
        groups.extend(Group::mechanical(CommitType::Test, None, test));
        if let Some(change) = changelog.take() {
            docs.push(change);
        }
    } else {
        let source_paths: Vec<&str> = source.iter().map(|c| c.path.as_str()).collect();
        let scope = scope::scope(&source_paths);
        source.append(&mut test);
        let mut g =
            Group::mechanical(CommitType::Refactor, scope, source).expect("source is non-empty");
        g.source = true;
        if let Some(change) = changelog.take() {
            if matches!(change.status, Status::Added | Status::Modified) {
                g.changes.push(change);
            } else {
                docs.push(change);
            }
        }
        groups.push(g);
    }
    groups.extend(Group::mechanical(CommitType::Docs, None, docs));
    groups.extend(Group::mechanical(CommitType::Ci, None, ci));
    groups.extend(release);
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
            source: false,
            bumps: Vec::new(),
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
        let cases: [(&str, Status, Category); 41] = [
            ("TODO.md", Status::Modified, Category::Todo),
            ("PLAN.md", Status::Added, Category::Plan),
            ("PLAN.md", Status::Deleted, Category::Plan),
            ("PLAN.md", Status::Modified, Category::Plan),
            ("CHANGELOG.md", Status::Modified, Category::Changelog),
            (
                "docs/decisions/2026-09-06-x.md",
                Status::Added,
                Category::Decision,
            ),
            (
                "docs/decisions/2026-09-06-x.md",
                Status::Modified,
                Category::Decision,
            ),
            (
                "docs/decisions/2026-09-06-x.md",
                Status::Deleted,
                Category::Docs,
            ),
            ("docs/decisions/notes.md", Status::Added, Category::Docs),
            (".gitignore", Status::Added, Category::Ignore),
            ("web/.dockerignore", Status::Modified, Category::Ignore),
            (".gitattributes", Status::Modified, Category::Ignore),
            (".github/workflows/ci.yml", Status::Added, Category::Ci),
            (
                ".github/actions/setup/action.yml",
                Status::Added,
                Category::Ci,
            ),
            (".circleci/config.yml", Status::Added, Category::Ci),
            ("Jenkinsfile", Status::Added, Category::Ci),
            ("Cargo.lock", Status::Modified, Category::Lock),
            ("web/bun.lock", Status::Modified, Category::Lock),
            ("Gemfile.lock", Status::Modified, Category::Lock),
            ("Cargo.toml", Status::Modified, Category::Manifest),
            ("web/package.json", Status::Modified, Category::Manifest),
            ("requirements.txt", Status::Modified, Category::Manifest),
            ("tests/cli.rs", Status::Added, Category::Test),
            ("crates/x/tests/a.rs", Status::Added, Category::Test),
            ("src/__tests__/a.ts", Status::Added, Category::Test),
            ("src/parser/tests.rs", Status::Added, Category::Test),
            ("src/foo_test.go", Status::Added, Category::Test),
            ("src/foo.test.ts", Status::Added, Category::Test),
            ("src/foo.spec.ts", Status::Added, Category::Test),
            ("pkg/test_foo.py", Status::Added, Category::Test),
            ("Sources/FooTests.swift", Status::Added, Category::Test),
            ("docs/guide.md", Status::Modified, Category::Docs),
            ("README.md", Status::Modified, Category::Docs),
            ("notes.txt", Status::Modified, Category::Docs),
            ("docs/Makefile", Status::Modified, Category::Build),
            ("justfile", Status::Modified, Category::Build),
            ("CMakeLists.txt", Status::Modified, Category::Build),
            ("Dockerfile.ci", Status::Added, Category::Build),
            (".husky/pre-commit", Status::Added, Category::Build),
            (".cargo/config.toml", Status::Added, Category::Build),
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

    #[test]
    fn plan_edit_is_its_own_docs_commit() {
        let changes = [modified("PLAN.md"), modified("README.md")];
        let groups = group(&changes, &none);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, CommitType::Docs);
        assert_eq!(groups[0].subject, "update PLAN.md");
        assert_eq!(groups[1].kind, CommitType::Docs);
        assert_eq!(groups[1].subject, "update README.md");
    }

    #[test]
    fn decision_added_as_draft() {
        let index = |spec: &str| {
            assert_eq!(spec, ":docs/decisions/2026-09-06-x.md");
            Some("## Status\n\nThis is a **draft**; not final.\n".to_string())
        };
        let groups = group(
            &[with("docs/decisions/2026-09-06-x.md", Status::Added)],
            &index,
        );
        assert_eq!(groups[0].kind, CommitType::Decision);
        assert_eq!(groups[0].subject, "draft 2026-09-06-x");
    }

    #[test]
    fn decision_status_transition_is_an_event() {
        let read = |spec: &str| match spec {
            "HEAD:docs/decisions/2026-09-06-x.md" => {
                Some("## Status\n\n**awaiting review**\n".to_string())
            }
            ":docs/decisions/2026-09-06-x.md" => Some("## Status\n\n**accepted**\n".to_string()),
            _ => None,
        };
        let groups = group(
            &[with("docs/decisions/2026-09-06-x.md", Status::Modified)],
            &read,
        );
        assert_eq!(groups[0].kind, CommitType::Decision);
        assert_eq!(groups[0].subject, "accept 2026-09-06-x");
    }

    #[test]
    fn decision_edit_without_transition_is_docs() {
        let read = |spec: &str| match spec {
            "HEAD:docs/decisions/2026-09-06-x.md" | ":docs/decisions/2026-09-06-x.md" => {
                Some("## Status\n\n**accepted**\n".to_string())
            }
            _ => None,
        };
        let groups = group(
            &[with("docs/decisions/2026-09-06-x.md", Status::Modified)],
            &read,
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Docs);
    }

    #[test]
    fn release_heading_cuts_a_release() {
        let read = |spec: &str| match spec {
            "HEAD:CHANGELOG.md" => Some("## [Unreleased]\n".to_string()),
            ":CHANGELOG.md" => Some("## [Unreleased]\n\n## [0.2.0] - 2026-09-07\n".to_string()),
            _ => None,
        };
        let changes = [
            with("CHANGELOG.md", Status::Modified),
            modified("Cargo.toml"),
            modified("Cargo.lock"),
        ];
        let groups = group(&changes, &read);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Release);
        assert_eq!(groups[0].subject, "v0.2.0");
        assert_eq!(groups[0].changes.len(), 3);
    }

    #[test]
    fn bumps_are_inferred_from_toml_lock() {
        let read = |spec: &str| match spec {
            "HEAD:Cargo.lock" => {
                Some("[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n".to_string())
            }
            ":Cargo.lock" => {
                Some("[[package]]\nname = \"serde\"\nversion = \"1.0.1\"\n".to_string())
            }
            _ => None,
        };
        let groups = group(&[with("Cargo.lock", Status::Modified)], &read);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Deps);
        assert_eq!(groups[0].bumps, vec!["serde 1.0.0 -> 1.0.1"]);
    }
    #[test]
    fn changelog_rides_with_source() {
        let changes = [modified("src/a.rs"), modified("CHANGELOG.md")];
        let groups = group(&changes, &none);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Refactor);
        assert_eq!(groups[0].subject, "update src/a.rs");
        assert!(groups[0].changes.iter().any(|c| c.path == "CHANGELOG.md"));
    }

    #[test]
    fn changelog_alone_is_docs() {
        let groups = group(&[modified("CHANGELOG.md")], &none);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, CommitType::Docs);
    }
}
