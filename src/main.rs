mod change;
mod classify;
mod git;
mod message;
mod scope;
mod subject;

use clap::Parser;

use change::Change;
use classify::Group;
use message::{CommitType, Message};

/// Commit dirty files as one agent-commits conventional commit per logical
/// group, inferred from the paths.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// git pathspecs to stage and commit; default: everything dirty
    paths: Vec<String>,

    /// commit type (feat, fix, perf, security, ... are never inferred)
    #[arg(long = "type", help_heading = "Intent (switches to a single commit)")]
    kind: Option<CommitType>,
    /// scope; "" removes the inferred one
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    scope: Option<String>,
    /// subject line
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    subject: Option<String>,
    /// body paragraph (required for perf)
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    body: Option<String>,
    /// append ! to the header
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    breaking: bool,
    /// Also: trailer, the type that lost the tiebreak
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    also: Option<CommitType>,
    /// Reverts: trailer
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    reverts: Option<String>,
    /// Advisory: trailer (repeatable; required for security)
    #[arg(long, help_heading = "Intent (switches to a single commit)")]
    advisory: Vec<String>,

    /// Bumps: trailer (repeatable; required for deps, attaches to the deps group)
    #[arg(long, help_heading = "Group-targeted")]
    bumps: Vec<String>,

    /// Generator: trailer
    #[arg(long, help_heading = "Provenance (applied to every commit)")]
    generator: Option<String>,
    /// Assisted-by: trailer, "<model> <harness>/<version> mode=<mode>"
    #[arg(long, help_heading = "Provenance (applied to every commit)")]
    assisted_by: Option<String>,
    /// Reviewed-by: trailer (repeatable)
    #[arg(long, help_heading = "Provenance (applied to every commit)")]
    reviewed_by: Vec<String>,
    /// Tested-by: trailer (repeatable)
    #[arg(long, help_heading = "Provenance (applied to every commit)")]
    tested_by: Vec<String>,

    /// stage and print every message; commit nothing
    #[arg(long)]
    dry_run: bool,
}

impl Cli {
    fn single_intent(&self) -> bool {
        self.kind.is_some()
            || self.scope.is_some()
            || self.subject.is_some()
            || self.body.is_some()
            || self.breaking
            || self.also.is_some()
            || self.reverts.is_some()
            || !self.advisory.is_empty()
    }

    fn with_provenance(&self, mut message: Message) -> Message {
        message.generator = self.generator.clone();
        message.assisted_by = self.assisted_by.clone();
        message.reviewed_by = self.reviewed_by.clone();
        message.tested_by = self.tested_by.clone();
        message
    }
}

struct Failure {
    code: i32,
    reason: String,
}

impl Failure {
    fn git(reason: String) -> Self {
        Self { code: 1, reason }
    }

    fn invalid(reason: impl Into<String>) -> Self {
        Self {
            code: 2,
            reason: reason.into(),
        }
    }
}

struct Planned {
    message: Message,
    changes: Vec<Change>,
}

fn main() {
    let code = match execute(&Cli::parse()) {
        Ok(()) => 0,
        Err(Failure { code, reason }) => {
            eprintln!("{reason}");
            code
        }
    };
    std::process::exit(code);
}

fn execute(cli: &Cli) -> Result<(), Failure> {
    git::stage(&cli.paths).map_err(Failure::git)?;
    let changes = git::staged_changes(&cli.paths).map_err(Failure::git)?;
    if changes.is_empty() {
        return Err(Failure {
            code: 3,
            reason: "nothing to commit".into(),
        });
    }
    let groups = classify::group(&changes, &git::show);
    let planned = if cli.single_intent() {
        vec![single(cli, changes, &groups)?]
    } else {
        grouped(cli, groups)?
    };

    for p in &planned {
        p.message.validate().map_err(Failure::invalid)?;
    }
    for p in &planned {
        if p.message.reads_like_two_intents() {
            eprintln!("warning: subject reads like two intents; consider splitting");
        }
    }
    let rendered: Vec<String> = planned.iter().map(|p| p.message.to_string()).collect();
    print!("{}", rendered.join("\n"));
    if cli.dry_run {
        return Ok(());
    }
    for (done, (p, text)) in planned.iter().zip(&rendered).enumerate() {
        let paths: Vec<&str> = p.changes.iter().flat_map(Change::pathspecs).collect();
        git::commit(text, &paths).map_err(|e| {
            Failure::git(format!(
                "committed {done} of {} groups; git commit failed: {e}",
                planned.len()
            ))
        })?;
    }
    Ok(())
}

fn grouped(cli: &Cli, groups: Vec<Group>) -> Result<Vec<Planned>, Failure> {
    if !cli.bumps.is_empty() && !groups.iter().any(|g| g.kind == CommitType::Deps) {
        return Err(Failure::invalid(
            "--bumps given but this change set has no deps group",
        ));
    }
    Ok(groups
        .into_iter()
        .map(|g| {
            let mut message = Message::new(g.kind, g.scope, g.subject);
            if g.kind == CommitType::Deps {
                message.bumps = cli.bumps.clone();
            }
            Planned {
                message: cli.with_provenance(message),
                changes: g.changes,
            }
        })
        .collect())
}

fn single(cli: &Cli, changes: Vec<Change>, groups: &[Group]) -> Result<Planned, Failure> {
    let sole = match groups {
        [only] => Some(only),
        _ => None,
    };
    let kind = match (cli.kind, sole) {
        (Some(kind), _) => kind,
        (None, Some(g)) => g.kind,
        (None, None) => {
            let kinds: Vec<&str> = groups.iter().map(|g| g.kind.as_str()).collect();
            return Err(Failure::invalid(format!(
                "mixed change set needs --type; inferred groups: {}",
                kinds.join(", ")
            )));
        }
    };
    let scope = match (&cli.scope, sole) {
        (Some(s), _) => (!s.is_empty()).then(|| s.clone()),
        (None, Some(g)) => g.scope.clone(),
        (None, None) => {
            let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
            scope::scope(&paths)
        }
    };
    let subject = match (&cli.subject, sole) {
        (Some(s), _) => s.clone(),
        (None, Some(g)) => g.subject.clone(),
        (None, None) => subject::subject(&changes),
    };
    let mut message = Message::new(kind, scope, subject);
    message.breaking = cli.breaking;
    message.body = cli.body.clone();
    message.bumps = cli.bumps.clone();
    message.advisory = cli.advisory.clone();
    message.reverts = cli.reverts.clone();
    message.also = cli.also;
    Ok(Planned {
        message: cli.with_provenance(message),
        changes,
    })
}
