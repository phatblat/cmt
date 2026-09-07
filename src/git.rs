use std::io::Write;
use std::process::{Command, Stdio};

use crate::change::{Change, Status};

fn run(args: &[&str], stdin: Option<&str>) -> Result<Vec<u8>, String> {
    let describe = || format!("git {}", args.join(" "));
    let mut child = Command::new("git")
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{} failed: {e}", describe()))?;
    if let Some(text) = stdin {
        let mut pipe = child.stdin.take().expect("stdin was piped");
        pipe.write_all(text.as_bytes())
            .map_err(|e| format!("{} failed: {e}", describe()))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{} failed: {e}", describe()))?;
    if !out.status.success() {
        return Err(format!(
            "{} failed: {}",
            describe(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn with_pathspecs<'a>(base: &[&'a str], pathspecs: &'a [String]) -> Vec<&'a str> {
    let mut args = base.to_vec();
    if !pathspecs.is_empty() {
        args.push("--");
        args.extend(pathspecs.iter().map(String::as_str));
    }
    args
}

/// `git add -A [-- <pathspecs>]`.
pub fn stage(pathspecs: &[String]) -> Result<(), String> {
    run(&with_pathspecs(&["add", "-A"], pathspecs), None).map(drop)
}

/// Staged changes, sorted by path, with rename detection.
pub fn staged_changes(pathspecs: &[String]) -> Result<Vec<Change>, String> {
    let out = run(
        &with_pathspecs(
            &["diff", "--cached", "--name-status", "-M", "-z"],
            pathspecs,
        ),
        None,
    )?;
    let text = String::from_utf8_lossy(&out);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut changes = Vec::new();
    while let Some(code) = fields.next() {
        let first = fields
            .next()
            .ok_or_else(|| "git diff --cached output ended mid-record".to_string())?;
        let change = match code.as_bytes()[0] {
            b'R' | b'C' => {
                let to = fields
                    .next()
                    .ok_or_else(|| "git diff --cached output ended mid-record".to_string())?;
                let status = if code.starts_with('R') {
                    Status::Renamed {
                        from: first.to_string(),
                    }
                } else {
                    Status::Added
                };
                Change {
                    path: to.to_string(),
                    status,
                }
            }
            b'A' => Change {
                path: first.to_string(),
                status: Status::Added,
            },
            b'D' => Change {
                path: first.to_string(),
                status: Status::Deleted,
            },
            _ => Change {
                path: first.to_string(),
                status: Status::Modified,
            },
        };
        changes.push(change);
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// `git diff --cached [-- <pathspecs>]`: the staged patch, as text.
pub fn diff(pathspecs: &[String]) -> Result<String, String> {
    let out = run(
        &with_pathspecs(
            &["diff", "--cached", "--no-color", "--no-ext-diff", "-M"],
            pathspecs,
        ),
        None,
    )?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// `git show <spec>`; `None` when the object does not exist (unborn HEAD,
/// missing path). `spec` is `HEAD:<path>` or `:<path>` (the index).
pub fn show(spec: &str) -> Option<String> {
    run(&["show", spec], None)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// `git commit -q -F - -- <paths>`; every path is repo-relative and matched
/// literally from the top of the working tree, so the caller's cwd does not
/// matter. With a pathspec, git records only those paths and leaves everything
/// else staged for the next call.
pub fn commit(message: &str, paths: &[&str]) -> Result<(), String> {
    let specs: Vec<String> = paths.iter().map(|p| format!(":(top,literal){p}")).collect();
    let mut args = vec!["commit", "-q", "-F", "-", "--"];
    args.extend(specs.iter().map(String::as_str));
    run(&args, Some(message)).map(drop)
}
