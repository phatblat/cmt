use crate::change::{Change, Status};

const MAX_LEN: usize = 72;

/// Mechanical subject: `<verb> <path>` for one change, `<verb> <N> files in
/// <dir>` for several, shortened when it would run past 72 characters.
pub fn subject(changes: &[Change]) -> String {
    let verb = verb(changes);
    if let [only] = changes {
        let full = match &only.status {
            Status::Renamed { from } => format!("rename {from} to {}", only.path),
            _ => format!("{verb} {}", only.path),
        };
        if fits(&full) {
            return full;
        }
        return format!("{verb} {}", basename(&only.path));
    }
    let n = changes.len();
    if let Some(dir) = common_dir(changes) {
        let with_dir = format!("{verb} {n} files in {dir}");
        if fits(&with_dir) {
            return with_dir;
        }
    }
    format!("{verb} {n} files")
}

fn fits(s: &str) -> bool {
    s.chars().count() <= MAX_LEN
}

fn verb(changes: &[Change]) -> &'static str {
    let all = |pred: fn(&Status) -> bool| changes.iter().all(|c| pred(&c.status));
    if all(|s| *s == Status::Added) {
        "add"
    } else if all(|s| *s == Status::Deleted) {
        "remove"
    } else if all(|s| matches!(s, Status::Renamed { .. })) {
        "rename"
    } else {
        "update"
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Longest directory prefix shared by every path; `None` at the repo root.
fn common_dir(changes: &[Change]) -> Option<String> {
    let mut common: Option<Vec<&str>> = None;
    for change in changes {
        let mut dirs: Vec<&str> = change.path.split('/').collect();
        dirs.pop();
        common = Some(match common {
            None => dirs,
            Some(seen) => seen
                .iter()
                .zip(&dirs)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect(),
        });
    }
    let dirs = common?;
    if dirs.is_empty() {
        None
    } else {
        Some(dirs.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, status: Status) -> Change {
        Change {
            path: path.into(),
            status,
        }
    }

    #[test]
    fn single_change_names_the_path_with_its_verb() {
        assert_eq!(
            subject(&[change("src/a.rs", Status::Added)]),
            "add src/a.rs"
        );
        assert_eq!(
            subject(&[change("src/a.rs", Status::Modified)]),
            "update src/a.rs"
        );
        assert_eq!(
            subject(&[change("src/a.rs", Status::Deleted)]),
            "remove src/a.rs"
        );
    }

    #[test]
    fn rename_names_both_paths() {
        assert_eq!(
            subject(&[change(
                "src/b.rs",
                Status::Renamed {
                    from: "src/a.rs".into()
                }
            )]),
            "rename src/a.rs to src/b.rs"
        );
    }

    #[test]
    fn mixed_statuses_are_an_update() {
        assert_eq!(
            subject(&[
                change("src/a.rs", Status::Added),
                change("src/b.rs", Status::Deleted),
            ]),
            "update 2 files in src"
        );
    }

    #[test]
    fn uniform_statuses_keep_their_verb_across_files() {
        assert_eq!(
            subject(&[
                change("src/a.rs", Status::Deleted),
                change("src/b.rs", Status::Deleted),
            ]),
            "remove 2 files in src"
        );
    }

    #[test]
    fn no_common_directory_drops_the_location() {
        assert_eq!(
            subject(&[
                change("src/a.rs", Status::Modified),
                change("README.md", Status::Modified),
            ]),
            "update 2 files"
        );
    }

    #[test]
    fn long_single_path_falls_back_to_basename() {
        let path = format!("{}/x.rs", "d".repeat(70));
        assert_eq!(subject(&[change(&path, Status::Modified)]), "update x.rs");
    }

    #[test]
    fn long_common_directory_falls_back_to_count() {
        let dir = "d".repeat(70);
        assert_eq!(
            subject(&[
                change(&format!("{dir}/a.rs"), Status::Modified),
                change(&format!("{dir}/b.rs"), Status::Modified),
            ]),
            "update 2 files"
        );
    }
}
