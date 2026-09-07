use crate::change::{Change, Status};
use crate::message::Message;

pub struct Draft<'a> {
    pub message: &'a Message,
    pub changes: &'a [Change],
    pub primary: bool,
    pub missing: Option<String>,
}

const RULES: &str = "Types, read top to bottom; the first match is the type:
  release   version bump, changelog cut, tag prep
  security  remediates a NAMED vulnerability (needs Advisory:)
  feat      adds a capability a consumer can reach
  fix       corrects behavior that contradicted a stated contract
  perf      same output, MEASURED resource change (needs a number in the body)
  deps      moves the resolved dependency graph (needs Bumps:)
  refactor  changes product source, no observable delta
  build     how the artifact is produced or validated
  ci        CI workflow config
  test      test code only
  docs      prose for human readers
  ignore    diff touches only ignore-family files
No chore, style, or revert. If nothing fits, the change is doing two things.

Rule N (feat vs fix): did the parent's docs, type signature, or tests already
claim the behavior existed? Broken promise -> fix. Nothing claimed it -> feat.
Rule F (fix vs refactor): does the change add or alter a test expectation that
fails against the parent commit? Yes -> fix. No -> refactor.
Rule P (perf): requires a before/after measurement in the body; otherwise refactor.
Precedence when two types fit: consumer-observable beats internal; named
vulnerability beats dependency bump beats behavior fix. The winner is the
type; the loser goes in Also:.
Breaking (!): a consumer must change something to keep working.";

/// `cmt -- <pathspecs>`, or bare `cmt` when there are none.
fn cmt_with_pathspecs(joined: &str) -> String {
    if joined.is_empty() {
        "cmt".to_string()
    } else {
        format!("cmt -- {joined}")
    }
}

fn push_commits(out: &mut String, drafts: &[Draft]) {
    out.push_str("## Proposed commits\n\n");
    for (i, d) in drafts.iter().enumerate() {
        let n = i + 1;
        let rendered = d.message.to_string();
        let rendered = rendered.trim_end_matches('\n');
        let mut lines = rendered.split('\n');
        let first = lines.next().unwrap_or("");
        if d.primary {
            out.push_str(&format!("{n}. {first}   [primary: intent undecided]\n"));
        } else {
            out.push_str(&format!("{n}. {first}\n"));
        }
        for line in lines {
            if line.is_empty() {
                out.push('\n');
            } else {
                out.push_str("   ");
                out.push_str(line);
                out.push('\n');
            }
        }
        for c in d.changes {
            match &c.status {
                Status::Added => out.push_str(&format!("   A {}\n", c.path)),
                Status::Modified => out.push_str(&format!("   M {}\n", c.path)),
                Status::Deleted => out.push_str(&format!("   D {}\n", c.path)),
                Status::Renamed { from } => out.push_str(&format!("   R {from} -> {}\n", c.path)),
            }
        }
    }
}

fn push_missing_facts(out: &mut String, drafts: &[Draft], joined: &str) {
    out.push_str("## Missing facts\n\n");
    let missing: Vec<(usize, &str)> = drafts
        .iter()
        .enumerate()
        .filter_map(|(i, d)| d.missing.as_deref().map(|m| (i + 1, m)))
        .collect();
    if missing.is_empty() {
        out.push_str("- none\n");
    } else {
        for (n, m) in missing {
            out.push_str(&format!("- commit {n}: {m}\n"));
        }
    }
    if !drafts.iter().any(|d| d.primary) {
        out.push_str(&format!(
            "\nNo commit is primary: every type above is path-derived; answer `{}` unless one is wrong.\n",
            cmt_with_pathspecs(joined)
        ));
    }
}

fn push_answer(out: &mut String, joined: &str) {
    out.push_str("## Answer\n\n");
    out.push_str("Reply with exactly one line and nothing else:\n\n");
    let flags = "[--type <feat|fix|perf|security|refactor>] [--breaking] [--scope <s>] \
                 [--subject \"<s>\"] [--body \"<why>\"] [--also <type>] [--advisory <id>]... \
                 [--bumps \"<pkg> <from> -> <to>\"]... [--single]";
    if joined.is_empty() {
        out.push_str(&format!("cmt {flags}\n\n"));
    } else {
        out.push_str(&format!("cmt {flags} -- {joined}\n\n"));
    }
    out.push_str("- Intent flags apply to the primary commit only; the others keep their types.\n");
    out.push_str(&format!(
        "- Omit every flag whose default above is already right; `{}` accepts the plan as is.\n",
        cmt_with_pathspecs(joined)
    ));
    out.push_str("- --breaking only when a consumer must change something to keep working.\n");
    out.push_str(
        "- --body is required for perf (with a measurement such as `12ms -> 3ms`) and should say why for feat and fix.\n",
    );
    out.push_str(
        "- --single squashes every commit above into one; use it only when the other commits exist solely because of the primary change and describe nothing on their own.\n",
    );
    out.push_str(
        "- --advisory only with --type security; --bumps only when a deps commit is listed.\n",
    );
    out.push_str(
        "- Subject: imperative, under 72 characters, no trailing period; keep the mechanical subject unless it hides the intent.\n",
    );
    out.push_str(
        "- Append your own --assisted-by \"<model> <harness>/<version> mode=<suggested|supervised|autonomous>\".\n",
    );
    if joined.is_empty() {
        out.push_str("- Pass no pathspecs: the selection is everything dirty.\n");
    } else {
        out.push_str(&format!("- The pathspecs must be exactly: {joined}\n"));
    }
}

/// The prompt an agent hands to an LLM to decide intent; cmt never calls one
/// itself.
pub fn render(drafts: &[Draft], diff: &str, pathspecs: &[String]) -> String {
    let joined = pathspecs.join(" ");
    let mut out = String::new();
    out.push_str(
        "You are deciding the intent of a staged change set under the agent-commits\n\
         convention. cmt has already split it by path into the commits below and will\n\
         create them as written unless you supply intent. Do not run git; answer with\n\
         one cmt command line.\n\n",
    );
    push_commits(&mut out, drafts);
    out.push('\n');
    push_missing_facts(&mut out, drafts, &joined);
    out.push('\n');
    out.push_str("## Convention\n\n");
    out.push_str(RULES);
    out.push_str("\n\n");
    out.push_str("## Staged diff\n\n");
    out.push_str("```diff\n");
    out.push_str(diff.trim_end_matches('\n'));
    out.push_str("\n```\n\n");
    push_answer(&mut out, &joined);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::CommitType;

    #[test]
    fn renders_every_section() {
        let refactor_changes = [
            Change {
                path: "src/parser/a.rs".into(),
                status: Status::Modified,
            },
            Change {
                path: "src/parser/b.rs".into(),
                status: Status::Renamed {
                    from: "src/parser/old.rs".into(),
                },
            },
        ];
        let refactor = Message::new(
            CommitType::Refactor,
            Some("parser".into()),
            "update 2 files in src/parser".into(),
        );
        let mut deps = Message::new(CommitType::Deps, None, "update Cargo.lock".into());
        deps.bumps = Vec::new();
        let deps_changes = [Change {
            path: "Cargo.lock".into(),
            status: Status::Modified,
        }];
        let drafts = [
            Draft {
                message: &refactor,
                changes: &refactor_changes,
                primary: true,
                missing: None,
            },
            Draft {
                message: &deps,
                changes: &deps_changes,
                primary: false,
                missing: Some("`deps` requires --bumps \"<pkg> <from> -> <to>\"".into()),
            },
        ];
        let pathspecs = ["src".to_string()];
        let out = render(&drafts, "+x", &pathspecs);

        assert!(out.contains(
            "## Proposed commits\n\n\
             1. refactor(parser): update 2 files in src/parser   [primary: intent undecided]\n\
             \x20\x20\x20M src/parser/a.rs\n\
             \x20\x20\x20R src/parser/old.rs -> src/parser/b.rs\n\
             2. deps: update Cargo.lock\n\
             \x20\x20\x20M Cargo.lock\n"
        ));
        assert!(out.contains(
            "## Missing facts\n\n\
             - commit 2: `deps` requires --bumps \"<pkg> <from> -> <to>\"\n"
        ));
        assert!(out.ends_with("- The pathspecs must be exactly: src\n"));
    }
}
