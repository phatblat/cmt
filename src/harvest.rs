//! `git show --name-status` parsing, extracted so it's reachable by `cargo
//! test`; `utils/harvest.rs` is a thin CLI wrapper over this plus the git
//! and filesystem I/O.

use crate::change::{Change, Status};

/// Parses `git show --name-status -M` output (or `git diff --name-status`)
/// into `Change` values, sorted by path. Unrecognised status codes (`C`,
/// `T`, ...) are treated as `Modified`, matching how `staged_changes` in
/// `src/git.rs` reads anything it doesn't special-case.
pub fn parse_name_status(raw: &str) -> Vec<Change> {
    let mut changes = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let mut fields = line.split('\t');
        let code = fields.next().unwrap_or_default();
        let first = fields.next().unwrap_or_default().to_string();
        let second = fields.next().map(str::to_string);
        let change = match (code.chars().next(), second) {
            (Some('A'), _) => Change {
                path: first,
                status: Status::Added,
            },
            (Some('D'), _) => Change {
                path: first,
                status: Status::Deleted,
            },
            (Some('R'), Some(to)) => Change {
                path: to,
                status: Status::Renamed { from: first },
            },
            _ => Change {
                path: first,
                status: Status::Modified,
            },
        };
        changes.push(change);
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_add_delete_modify_and_rename() {
        let raw =
            "A\tsrc/new.rs\nD\tsrc/gone.rs\nM\tsrc/changed.rs\nR100\tsrc/old.rs\tsrc/moved.rs\n";
        let changes = parse_name_status(raw);

        assert_eq!(
            changes,
            vec![
                Change {
                    path: "src/changed.rs".into(),
                    status: Status::Modified,
                },
                Change {
                    path: "src/gone.rs".into(),
                    status: Status::Deleted,
                },
                Change {
                    path: "src/moved.rs".into(),
                    status: Status::Renamed {
                        from: "src/old.rs".into(),
                    },
                },
                Change {
                    path: "src/new.rs".into(),
                    status: Status::Added,
                },
            ]
        );
    }

    #[test]
    fn an_unrecognised_status_code_is_modified() {
        let changes = parse_name_status("C100\tsrc/a.rs\tsrc/b.rs\n");

        assert_eq!(
            changes,
            vec![Change {
                path: "src/a.rs".into(),
                status: Status::Modified,
            }]
        );
    }

    #[test]
    fn blank_lines_are_skipped() {
        assert_eq!(
            parse_name_status("\n\nA\tsrc/a.rs\n\n"),
            parse_name_status("A\tsrc/a.rs\n")
        );
    }

    #[test]
    fn results_are_sorted_by_path() {
        let changes = parse_name_status("M\tsrc/z.rs\nM\tsrc/a.rs\n");

        assert_eq!(changes[0].path, "src/a.rs");
        assert_eq!(changes[1].path, "src/z.rs");
    }
}
