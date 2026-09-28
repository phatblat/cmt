use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::thread;

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

    /// Runs `cmt` with `TYPESAFE_API_KEY`/`TYPESAFE_ENDPOINT` cleared, then
    /// `env` applied on top, so `--jev` tests never depend on the ambient
    /// environment.
    fn cmt_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = self.command(env!("CARGO_BIN_EXE_cmt"));
        cmd.env_remove("TYPESAFE_API_KEY");
        cmd.env_remove("TYPESAFE_ENDPOINT");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.args(args).output().expect("run cmt")
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
fn mise_config_and_source_become_two_ordered_commits() {
    let repo = Repo::seeded();
    repo.write("mise.toml", "[tools]\nrust = \"1.80\"\n");
    repo.write("src/lib.rs", "pub fn f() -> u8 { 1 }\n");
    let out = repo.cmt(&[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "build: add mise.toml\n\nrefactor: update src/lib.rs\n"
    );
    assert_eq!(
        repo.subjects(),
        "seed\nbuild: add mise.toml\nrefactor: update src/lib.rs\n"
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

/// Accepts one connection, replies with `response`, and returns the request
/// body it received (read via `Content-Length`).
fn serve_once(response: String) -> (u16, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("set read timeout");
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read line");
            if line == "\r\n" || line.is_empty() {
                break;
            }
            if let Some(len) = line
                .to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::trim)
            {
                content_length = len.parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).expect("read body");
        let mut stream = stream;
        stream.write_all(response.as_bytes()).expect("write");
        String::from_utf8_lossy(&body).into_owned()
    });
    (port, handle)
}

/// A jev response naming `observable_delta` and `adds_capability`, so the
/// cascade resolves to `feat`, with every other predicate quiet.
fn feat_answers(observable_delta: f64) -> String {
    format!(
        r#"{{"model":"jev-1.13.0","answers":{{"named_vulnerability":{{"type":"noul","noul":0.01}},"observable_delta":{{"type":"noul","noul":{observable_delta}}},"adds_capability":{{"type":"noul","noul":0.94}},"contradicted_stated_contract":{{"type":"noul","noul":0.03}},"measured_resource_change":{{"type":"noul","noul":0.02}},"consumer_must_change":{{"type":"noul","noul":0.02}}}},"usage":{{"input_tokens":1200,"output_tokens":20}}}}"#
    )
}

/// A jev response naming each of the six requested (non-test) predicates at
/// the given confidence, so the full cascade — not just the feat path — is
/// reachable end to end through the real CLI.
fn cascade_answers(
    named_vulnerability: f64,
    observable_delta: f64,
    contradicted_stated_contract: f64,
    adds_capability: f64,
    measured_resource_change: f64,
    consumer_must_change: f64,
) -> String {
    format!(
        r#"{{"model":"jev-1.13.0","answers":{{"named_vulnerability":{{"type":"noul","noul":{named_vulnerability}}},"observable_delta":{{"type":"noul","noul":{observable_delta}}},"adds_capability":{{"type":"noul","noul":{adds_capability}}},"contradicted_stated_contract":{{"type":"noul","noul":{contradicted_stated_contract}}},"measured_resource_change":{{"type":"noul","noul":{measured_resource_change}}},"consumer_must_change":{{"type":"noul","noul":{consumer_must_change}}}}},"usage":{{"input_tokens":1200,"output_tokens":20}}}}"#
    )
}

fn http_ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

#[test]
fn jev_without_a_key_exits_four() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() {}\npub fn g() {}\n");

    let out = repo.cmt_env(&["--jev"], &[]);

    assert_eq!(out.status.code(), Some(4));
    assert!(
        stderr(&out).contains("TYPESAFE_API_KEY"),
        "stderr should name the missing variable: {}",
        stderr(&out)
    );
}

#[test]
fn jev_conflicts_with_type() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() {}\npub fn g() {}\n");

    let out = repo.cmt(&["--jev", "--type", "feat"]);

    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr(&out).contains("cannot be used with"),
        "clap should reject the combination: {}",
        stderr(&out)
    );
}

#[test]
fn jev_without_a_source_group_exits_four() {
    let repo = Repo::seeded();
    repo.write("README.md", "# seed\n\nmore\n");

    let out = repo.cmt(&["--jev"]);

    assert_eq!(out.status.code(), Some(4));
    assert!(
        stderr(&out).contains("no source"),
        "stderr should say why there is nothing to ask about: {}",
        stderr(&out)
    );
}

#[test]
fn jev_decides_the_source_commit() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let (port, handle) = serve_once(http_ok(&feat_answers(0.96)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    let request_body = handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nfeat: update src/lib.rs\n");
    assert!(
        request_body.contains(r#""model":"jev-1.13.0""#),
        "{request_body}"
    );
    assert!(request_body.contains("observable_delta"), "{request_body}");
    assert!(
        request_body.contains(r#""files":["src/lib.rs"]"#),
        "{request_body}"
    );
    assert!(
        !request_body.contains("test_expectation_changed"),
        "no test file staged, so Rule F's predicate must be omitted: {request_body}"
    );
}

#[test]
fn jev_refuses_an_ambiguous_answer() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let before = repo.head();
    let (port, handle) = serve_once(http_ok(&feat_answers(0.5)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(4));
    assert!(stderr(&out).contains("jev was unsure"), "{}", stderr(&out));
    assert_eq!(repo.head(), before, "a refusal must leave HEAD unmoved");
}

#[test]
fn jev_decides_fix_for_a_broken_promise() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let (port, handle) = serve_once(http_ok(&cascade_answers(0.01, 0.9, 0.9, 0.01, 0.02, 0.02)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nfix: update src/lib.rs\n");
}

#[test]
fn jev_decides_refactor_when_every_predicate_is_quiet() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let (port, handle) = serve_once(http_ok(&cascade_answers(
        0.01, 0.02, 0.02, 0.02, 0.02, 0.02,
    )));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nrefactor: update src/lib.rs\n");
}

#[test]
fn jev_decides_perf_with_a_measurement_in_the_body() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let (port, handle) = serve_once(http_ok(&cascade_answers(0.01, 0.02, 0.02, 0.02, 0.9, 0.02)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev", "--body", "before 10ms after 4ms"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nperf: update src/lib.rs\n");
}

#[test]
fn jev_decides_security_and_still_requires_advisory() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let (port, handle) = serve_once(http_ok(&cascade_answers(0.9, 0.9, 0.02, 0.02, 0.02, 0.02)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("security` requires --advisory"),
        "{}",
        stderr(&out)
    );

    let (port, handle) = serve_once(http_ok(&cascade_answers(0.9, 0.9, 0.02, 0.02, 0.02, 0.02)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");
    let out = repo.cmt_env(
        &["--jev", "--advisory", "GHSA-xxxx-xxxx-xxxx"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nsecurity: update src/lib.rs\n");
}

#[test]
fn jev_appends_the_breaking_marker_from_consumer_must_change() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() -> u8 { 2 }\n");
    let (port, handle) = serve_once(http_ok(&cascade_answers(0.01, 0.9, 0.02, 0.9, 0.02, 0.9)));
    let url = format!("http://127.0.0.1:{port}/v1/systemone");

    let out = repo.cmt_env(
        &["--jev"],
        &[("TYPESAFE_API_KEY", "test"), ("TYPESAFE_ENDPOINT", &url)],
    );
    handle.join().expect("server thread");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(repo.subjects(), "seed\nfeat!: update src/lib.rs\n");
}
