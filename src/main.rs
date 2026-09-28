use clap::Parser;

use cmt::change::Change;
use cmt::classify::{self, Group};
use cmt::git;
use cmt::intent::{self, Intent};
use cmt::jev;
use cmt::message::{self, CommitType, Message};
use cmt::prompt;
use cmt::scope;
use cmt::subject;

/// Commit dirty files as one agent-commits conventional commit per logical
/// group, inferred from the paths.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// git pathspecs to stage and commit; default: everything dirty
    paths: Vec<String>,

    /// commit type; feat, fix, perf, security are never inferred
    #[arg(
        long = "type",
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    kind: Option<CommitType>,
    /// scope; "" removes the inferred one
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    scope: Option<String>,
    /// subject line
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    subject: Option<String>,
    /// body paragraph (required for perf)
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    body: Option<String>,
    /// append ! to the header
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    breaking: bool,
    /// Also: trailer, the type that lost the tiebreak
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    also: Option<CommitType>,
    /// Reverts: trailer
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    reverts: Option<String>,
    /// Advisory: trailer (repeatable; required for security)
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    advisory: Vec<String>,
    /// squash the whole selection into one commit
    #[arg(
        long,
        help_heading = "Intent (targets the source commit, or the only commit)"
    )]
    single: bool,

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

    /// ask jev to decide the source commit's intent; needs TYPESAFE_API_KEY
    #[arg(
        long,
        // `also` conflicts even though the jev overlay never touches
        // `message.also`: `--also` names the type that lost a manual
        // tiebreak, which has no meaning once jev is the one deciding.
        conflicts_with_all = ["kind", "also", "breaking", "single", "agent_prompt"]
    )]
    jev: bool,

    /// print a prompt for an agent to decide intent with; stages, commits nothing
    #[arg(
        long,
        conflicts_with_all = [
            "kind", "scope", "subject", "body", "breaking", "also", "reverts", "advisory",
            "single", "dry_run", "jev",
        ]
    )]
    agent_prompt: bool,
}

impl Cli {
    fn has_intent(&self) -> bool {
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

/// Applies every intent flag onto the primary commit's message.
fn apply_intent(cli: &Cli, message: &mut Message) {
    if let Some(kind) = cli.kind {
        message.kind = kind;
    }
    if let Some(s) = &cli.scope {
        message.scope = (!s.is_empty()).then(|| s.clone());
    }
    if let Some(s) = &cli.subject {
        message.subject = s.clone();
    }
    message.breaking = cli.breaking;
    message.body = cli.body.clone();
    message.advisory = cli.advisory.clone();
    message.reverts = cli.reverts.clone();
    message.also = cli.also;
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

    fn jev(reason: impl Into<String>) -> Self {
        Self {
            code: 4,
            reason: reason.into(),
        }
    }
}

struct Planned {
    message: Message,
    changes: Vec<Change>,
    primary: bool,
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
    let jev_intent = if cli.jev {
        Some(resolve_jev(cli, &groups)?)
    } else {
        None
    };
    let planned = if cli.single {
        vec![single(cli, changes, &groups)?]
    } else {
        grouped(cli, groups, jev_intent)?
    };

    if cli.agent_prompt {
        let diff = git::diff(&cli.paths).map_err(Failure::git)?;
        let drafts: Vec<prompt::Draft> = planned
            .iter()
            .map(|p| prompt::Draft {
                message: &p.message,
                changes: &p.changes,
                primary: p.primary,
                missing: p.message.validate().err(),
            })
            .collect();
        print!("{}", prompt::render(&drafts, &diff, &cli.paths));
        return Ok(());
    }

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

/// Asks jev about the source group and resolves the answers into an intent.
/// Every failure here is exit 4: no source commit, a missing key, a diff over
/// budget, a transport failure, or an answer too close to call.
fn resolve_jev(cli: &Cli, groups: &[Group]) -> Result<Intent, Failure> {
    let group = groups.iter().find(|g| g.source).ok_or_else(|| {
        Failure::jev("--jev needs a source commit; this change set has no source files")
    })?;
    let transport = jev::Http::from_env().map_err(Failure::jev)?;

    let paths: Vec<String> = group
        .changes
        .iter()
        .flat_map(Change::pathspecs)
        .map(str::to_string)
        .collect();
    let diff = git::diff(&paths).map_err(Failure::git)?;
    let state = intent::state(&group.changes, diff).map_err(Failure::jev)?;

    let questions = intent::questions(intent::has_tests(&group.changes));
    let response = jev::ask(&transport, &state, questions).map_err(Failure::jev)?;
    let answers: intent::Answers = response
        .answers
        .into_iter()
        .map(|(id, answer)| (id, answer.noul))
        .collect();

    intent::decide(
        &answers,
        cli.body.as_deref().is_some_and(message::has_measurement),
        intent::DEFAULT_BAND,
    )
    .map_err(|refusal| Failure::jev(refusal.to_string()))
}

fn grouped(cli: &Cli, groups: Vec<Group>, jev: Option<Intent>) -> Result<Vec<Planned>, Failure> {
    if !cli.bumps.is_empty() && !groups.iter().any(|g| g.kind == CommitType::Deps) {
        return Err(Failure::invalid(
            "--bumps given but this change set has no deps group",
        ));
    }
    let primary = groups
        .iter()
        .position(|g| g.source)
        .or((groups.len() == 1).then_some(0));
    if cli.has_intent() && primary.is_none() {
        let kinds: Vec<&str> = groups.iter().map(|g| g.kind.as_str()).collect();
        return Err(Failure::invalid(format!(
            "intent flags need a source commit or a single commit; inferred groups: {}; pass --single to squash them",
            kinds.join(", ")
        )));
    }
    Ok(groups
        .into_iter()
        .enumerate()
        .map(|(i, g)| {
            let mut message = Message::new(g.kind, g.scope, g.subject);
            if g.kind == CommitType::Deps {
                message.bumps = if cli.bumps.is_empty() {
                    g.bumps.clone()
                } else {
                    cli.bumps.clone()
                };
            }
            let is_primary = Some(i) == primary;
            if is_primary {
                apply_intent(cli, &mut message);
                if let Some(intent) = &jev {
                    message.kind = intent.kind;
                    message.breaking = intent.breaking;
                }
            }
            Planned {
                message: cli.with_provenance(message),
                changes: g.changes,
                primary: is_primary,
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
    message.bumps = if cli.bumps.is_empty() {
        groups.iter().flat_map(|g| g.bumps.clone()).collect()
    } else {
        cli.bumps.clone()
    };
    apply_intent(cli, &mut message);
    Ok(Planned {
        message: cli.with_provenance(message),
        changes,
        primary: true,
    })
}
