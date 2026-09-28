//! Proposes unlabelled eval fixtures from a repository's history.
//!
//! Usage: `just harvest <repo-path> <count>`
//!
//! For each of the most recent `count` commits, reconstructs the source group
//! the way `cmt` would and writes `evals/cases/<short-sha>.toml` with an empty
//! `expected` field. A human fills that in; the harvester never guesses a
//! label, because the historical subject is exactly the thing being measured.

use std::path::Path;
use std::process::Command;

use cmt::change::Change;
use cmt::classify;

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `git show --name-status` output into `Change` values.
fn changes(repo: &Path, sha: &str) -> Result<Vec<Change>, String> {
    let raw = git(repo, &["show", "--name-status", "--format=", "-M", sha])?;
    Ok(cmt::harvest::parse_name_status(&raw))
}

fn json(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (repo, count) = match args.as_slice() {
        [repo, count] => {
            let Ok(count) = count.parse::<usize>() else {
                eprintln!(
                    "usage: just harvest <repo-path> <count>; count must be a number, got {count:?}"
                );
                std::process::exit(2);
            };
            (Path::new(repo).to_path_buf(), count)
        }
        _ => {
            eprintln!("usage: just harvest <repo-path> <count>");
            std::process::exit(2);
        }
    };

    let log = match git(&repo, &["log", "--format=%H", &format!("-{count}")]) {
        Ok(log) => log,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = std::fs::create_dir_all("evals/cases") {
        eprintln!("evals/cases: {e}");
        std::process::exit(1);
    }
    let mut written = 0usize;
    for sha in log.lines() {
        let Ok(changes) = changes(&repo, sha) else {
            continue;
        };
        let groups = classify::group(&changes, &|_: &str| -> Option<String> { None });
        let Some(group) = groups.iter().find(|g| g.source) else {
            continue;
        };
        let paths: Vec<&str> = group.changes.iter().flat_map(Change::pathspecs).collect();
        let mut show_args = vec!["show", "--format=", "-M", sha, "--"];
        show_args.extend(paths.iter().copied());
        let Ok(diff) = git(&repo, &show_args) else {
            continue;
        };
        let subject = git(&repo, &["log", "--format=%s", "-1", sha]).unwrap_or_default();
        let short = &sha[..8];
        let path = format!("evals/cases/{short}.toml");
        if Path::new(&path).exists() {
            continue;
        }
        let files: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        let body = format!(
            "# candidate from {short}\n\
             # original subject: {}\n\
             # LABEL ME: set `expected` to the type this diff deserves, then \
             delete these comments.\n\
             expected = \"\"\n\
             rationale = \"\"\n\
             body = \"\"\n\
             files = {}\n\
             diff = {}\n",
            subject.trim(),
            serde_json::to_string(&files).expect("files serialize"),
            json(&diff),
        );
        if let Err(e) = std::fs::write(&path, body) {
            eprintln!("{path}: {e}");
            continue;
        }
        written += 1;
    }
    println!("wrote {written} unlabelled candidates to evals/cases/");
    println!("label each one's `expected` field before running `just eval`");
}
