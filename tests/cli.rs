use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

struct Repo {
    dir: TempDir,
}

impl Repo {
    fn unborn() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Self { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    /// One `seed` commit containing `src/lib.rs`, `README.md`, `Cargo.lock`.
    fn seeded() -> Self {
        let repo = Self::unborn();
        repo.write("src/lib.rs", "pub fn f() {}\n");
        repo.write("README.md", "# seed\n");
        repo.write("Cargo.lock", "# lock\n");
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", "seed"]);
        repo
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t");
        cmd
    }

    fn git(&self, args: &[&str]) -> String {
        let out = self.command("git").args(args).output().expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn cmt(&self, args: &[&str]) -> Output {
        self.command(env!("CARGO_BIN_EXE_cmt"))
            .args(args)
            .output()
            .expect("run cmt")
    }

    fn subjects(&self) -> String {
        self.git(&["log", "--reverse", "--format=%s"])
    }

    /// The stored commit message, byte for byte (`%B` appends its own newline).
    fn last_body(&self) -> String {
        let object = self.git(&["cat-file", "commit", "HEAD"]);
        object
            .split_once("\n\n")
            .expect("commit object")
            .1
            .to_string()
    }

    fn head(&self) -> Option<String> {
        let out = self
            .command("git")
            .args(["rev-parse", "-q", "--verify", "HEAD"])
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8(out.stdout).unwrap())
    }

    fn status(&self) -> String {
        self.git(&["status", "--porcelain"])
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

#[test]
fn dry_run_prints_without_committing() {
    let repo = Repo::unborn();
    repo.write(".gitignore", "/target\n");
    let out = repo.cmt(&["--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "ignore: add .gitignore\n");
    assert_eq!(repo.head(), None);
}

#[test]
fn initial_commits_are_split_per_group() {
    let repo = Repo::unborn();
    repo.write(".gitignore", "/target\n");
    repo.write("src/main.rs", "fn main() {}\n");
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        repo.subjects(),
        "ignore: add .gitignore\nrefactor: add src/main.rs\n"
    );
    assert_eq!(repo.status(), "");
}

#[test]
fn source_and_docs_become_two_commits() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    repo.write("README.md", "# seed\n\nmore\n");
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "refactor: update src/lib.rs\n\ndocs: update README.md\n"
    );
    assert_eq!(
        repo.subjects(),
        "seed\nrefactor: update src/lib.rs\ndocs: update README.md\n"
    );
    assert_eq!(repo.status(), "");
}

#[test]
fn deps_group_needs_bumps_and_nothing_is_committed_without_it() {
    let repo = Repo::seeded();
    let before = repo.head();
    repo.write("Cargo.lock", "# lock\n# moved\n");
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("requires --bumps"),
        "{}",
        stderr(&out)
    );
    assert_eq!(repo.head(), before);

    let out = repo.cmt(&["--bumps", "serde 1.0.0 -> 1.0.1"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        repo.last_body(),
        "deps: update Cargo.lock\n\nBumps: serde 1.0.0 -> 1.0.1\n"
    );
}

#[test]
fn deps_bumps_are_inferred_from_cargo_lock() {
    let repo = Repo::seeded();
    repo.write(
        "Cargo.lock",
        "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
    );
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "lock serde"]);
    repo.write(
        "Cargo.lock",
        "[[package]]\nname = \"serde\"\nversion = \"1.0.1\"\n",
    );
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        repo.last_body(),
        "deps: update Cargo.lock\n\nBumps: serde 1.0.0 -> 1.0.1\n"
    );
}

#[test]
fn clean_tree_is_exit_3() {
    let repo = Repo::seeded();
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(stderr(&out), "nothing to commit\n");
}

#[test]
fn type_override_makes_one_commit_with_provenance() {
    let repo = Repo::seeded();
    repo.write("src/parser/a.rs", "pub fn a() {}\n");
    repo.write("src/parser/b.rs", "pub fn b() {}\n");
    let out = repo.cmt(&["--type", "feat", "--assisted-by", "m h/1 mode=autonomous"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        repo.last_body(),
        "feat(parser): add 2 files in src/parser\n\nAssisted-by: m h/1 mode=autonomous\n"
    );
    assert_eq!(
        repo.subjects(),
        "seed\nfeat(parser): add 2 files in src/parser\n"
    );
}

#[test]
fn single_mode_mixed_set_needs_type() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    repo.write("README.md", "# seed\n\nmore\n");
    let out = repo.cmt(&["--single", "--subject", "x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("needs --type; inferred groups: refactor, docs"),
        "{}",
        stderr(&out)
    );
    assert_eq!(repo.subjects(), "seed\n");
}

#[test]
fn intent_targets_the_source_commit() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    repo.write("README.md", "# seed\n\nmore\n");
    let out = repo.cmt(&["--type", "feat", "--breaking", "--body", "why"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        repo.subjects(),
        "seed\nfeat!: update src/lib.rs\ndocs: update README.md\n"
    );
    assert!(
        repo.git(&["log", "-1", "--format=%B", "HEAD~1"])
            .contains("why")
    );
}

#[test]
fn single_squashes_every_group() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    repo.write("README.md", "# seed\n\nmore\n");
    let out = repo.cmt(&["--single", "--type", "feat"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nfeat: update 2 files\n");
}

#[test]
fn pathspec_limits_the_selection_and_plan_precedes_source() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    repo.write("README.md", "# seed\n\nmore\n");
    let out = repo.cmt(&["README.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\ndocs: update README.md\n");
    assert_eq!(repo.status(), " M src/lib.rs\n");

    repo.write(
        "PLAN.md",
        "# Plan\n\nImplements docs/decisions/2026-09-06-x.md.\n",
    );
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        repo.subjects(),
        "seed\ndocs: update README.md\nplan: start 2026-09-06-x\nrefactor: update src/lib.rs\n"
    );
    assert_eq!(repo.status(), "");
}

#[test]
fn agent_prompt_prints_and_commits_nothing() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    repo.write("README.md", "# seed\n\nmore\n");
    let before = repo.head();
    let out = repo.cmt(&["--agent-prompt"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("1. refactor: update src/lib.rs   [primary: intent undecided]"),
        "{text}"
    );
    assert!(text.contains("## Staged diff"), "{text}");
    assert!(text.contains("+pub fn f() -> u8 { 1 }"), "{text}");
    assert!(text.contains("## Answer"), "{text}");
    assert_eq!(repo.head(), before);
    assert_eq!(repo.status(), "M  README.md\nM  src/lib.rs\n");
}

#[test]
fn agent_prompt_conflicts_with_intent_flags() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    let out = repo.cmt(&["--agent-prompt", "--type", "fix"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("cannot be used with"),
        "{}",
        stderr(&out)
    );
}
