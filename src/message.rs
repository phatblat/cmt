use std::fmt;
use std::str::FromStr;

/// The agent-commits type set plus the conventional-docs lifecycle events.
/// No `chore`, `style`, or `revert`: those are retired by the convention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommitType {
    Release,
    Security,
    Feat,
    Fix,
    Perf,
    Deps,
    Refactor,
    Build,
    Ci,
    Test,
    Docs,
    Ignore,
    Decision,
    Deploy,
    Plan,
    Todo,
}

impl CommitType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Security => "security",
            Self::Feat => "feat",
            Self::Fix => "fix",
            Self::Perf => "perf",
            Self::Deps => "deps",
            Self::Refactor => "refactor",
            Self::Build => "build",
            Self::Ci => "ci",
            Self::Test => "test",
            Self::Docs => "docs",
            Self::Ignore => "ignore",
            Self::Decision => "decision",
            Self::Deploy => "deploy",
            Self::Plan => "plan",
            Self::Todo => "todo",
        }
    }
}

impl FromStr for CommitType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        Ok(match s {
            "release" => Self::Release,
            "security" => Self::Security,
            "feat" => Self::Feat,
            "fix" => Self::Fix,
            "perf" => Self::Perf,
            "deps" => Self::Deps,
            "refactor" => Self::Refactor,
            "build" => Self::Build,
            "ci" => Self::Ci,
            "test" => Self::Test,
            "docs" => Self::Docs,
            "ignore" => Self::Ignore,
            "decision" => Self::Decision,
            "deploy" => Self::Deploy,
            "plan" => Self::Plan,
            "todo" => Self::Todo,
            "chore" | "style" | "revert" => {
                return Err(format!(
                    "retired type `{s}`: use a specific type (`chore`), `refactor` or `build` (`style`), or the original type plus `--reverts` (`revert`)"
                ));
            }
            _ => return Err(format!("unknown type `{s}`")),
        })
    }
}

impl fmt::Display for CommitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: CommitType,
    pub scope: Option<String>,
    pub breaking: bool,
    pub subject: String,
    pub body: Option<String>,
    pub bumps: Vec<String>,
    pub advisory: Vec<String>,
    pub reverts: Option<String>,
    pub also: Option<CommitType>,
    pub generator: Option<String>,
    pub assisted_by: Option<String>,
    pub reviewed_by: Vec<String>,
    pub tested_by: Vec<String>,
}

impl Message {
    pub fn new(kind: CommitType, scope: Option<String>, subject: String) -> Self {
        Self {
            kind,
            scope,
            breaking: false,
            subject,
            body: None,
            bumps: Vec::new(),
            advisory: Vec::new(),
            reverts: None,
            also: None,
            generator: None,
            assisted_by: None,
            reviewed_by: Vec::new(),
            tested_by: Vec::new(),
        }
    }

    pub fn header(&self) -> String {
        let mut header = self.kind.as_str().to_string();
        if let Some(scope) = &self.scope {
            header.push('(');
            header.push_str(scope);
            header.push(')');
        }
        if self.breaking {
            header.push('!');
        }
        header.push_str(": ");
        header.push_str(&self.subject);
        header
    }

    fn trailers(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.extend(self.bumps.iter().map(|v| format!("Bumps: {v}")));
        lines.extend(self.advisory.iter().map(|v| format!("Advisory: {v}")));
        lines.extend(self.reverts.iter().map(|v| format!("Reverts: {v}")));
        lines.extend(self.also.iter().map(|v| format!("Also: {v}")));
        lines.extend(self.generator.iter().map(|v| format!("Generator: {v}")));
        lines.extend(self.assisted_by.iter().map(|v| format!("Assisted-by: {v}")));
        lines.extend(self.reviewed_by.iter().map(|v| format!("Reviewed-by: {v}")));
        lines.extend(self.tested_by.iter().map(|v| format!("Tested-by: {v}")));
        lines
    }

    /// Per-type trailer requirements and format rules. The string is the
    /// one-line reason shown to the user; nothing is committed when it fails.
    pub fn validate(&self) -> Result<(), String> {
        match self.kind {
            CommitType::Deps if self.bumps.is_empty() => {
                return Err("`deps` requires --bumps \"<pkg> <from> -> <to>\"".into());
            }
            CommitType::Security if self.advisory.is_empty() => {
                return Err("`security` requires --advisory <id>".into());
            }
            CommitType::Perf if self.body.is_none() => {
                return Err("`perf` requires --body with a before/after measurement".into());
            }
            CommitType::Security => {}
            _ if !self.advisory.is_empty() => {
                return Err("--advisory is only valid with --type security".into());
            }
            _ => {}
        }
        if self
            .assisted_by
            .as_deref()
            .is_some_and(|a| !assisted_by_is_valid(a))
        {
            return Err(
                "--assisted-by must be \"<model> <harness>/<version> mode=<suggested|supervised|autonomous>\""
                    .into(),
            );
        }
        let len = self.header().chars().count();
        if len > 100 {
            return Err(format!("header is {len} chars; commitlint limit is 100"));
        }
        Ok(())
    }

    /// The convention's tell for a commit doing two things.
    pub fn reads_like_two_intents(&self) -> bool {
        self.subject.contains(" and ") || self.subject.contains(", ")
    }
}

/// `<model> <harness>/<version> mode=<suggested|supervised|autonomous>`
fn assisted_by_is_valid(value: &str) -> bool {
    let mut parts = value.split(' ');
    let (Some(model), Some(harness), Some(mode), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let word = |s: &str| !s.is_empty() && !s.chars().any(char::is_whitespace);
    word(model)
        && harness
            .split_once('/')
            .is_some_and(|(h, v)| word(h) && word(v))
        && matches!(
            mode,
            "mode=suggested" | "mode=supervised" | "mode=autonomous"
        )
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.header())?;
        if let Some(body) = &self.body {
            writeln!(f)?;
            writeln!(f, "{}", body.trim_end())?;
        }
        let trailers = self.trailers();
        if !trailers.is_empty() {
            writeln!(f)?;
            for line in trailers {
                writeln!(f, "{line}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_agent_authored_fix_example() {
        let mut m = Message::new(
            CommitType::Fix,
            Some("parser".into()),
            "reject trailing commas in strict mode".into(),
        );
        m.body = Some(
            "strictMode:true still accepted `[1,2,]`, diverging from the ECMA-404\ngrammar the flag claims to enforce."
                .into(),
        );
        m.assisted_by = Some("claude-opus-4-6 claude-code/2.1.4 mode=autonomous".into());
        m.tested_by = vec!["ci/unit".into()];
        assert_eq!(
            m.to_string(),
            "fix(parser): reject trailing commas in strict mode\n\
             \n\
             strictMode:true still accepted `[1,2,]`, diverging from the ECMA-404\n\
             grammar the flag claims to enforce.\n\
             \n\
             Assisted-by: claude-opus-4-6 claude-code/2.1.4 mode=autonomous\n\
             Tested-by: ci/unit\n"
        );
        assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn renders_breaking_security_example_with_trailer_order() {
        let mut m = Message::new(
            CommitType::Security,
            Some("axios".into()),
            "upgrade to 1.8.4, patching SSRF via absolute baseURL".into(),
        );
        m.breaking = true;
        m.body = Some(
            "axios <1.8.2 resolves an absolute `url` against `baseURL`, letting a\n\
             caller-controlled path escape the configured host. The upgrade also\n\
             changes default paramsSerializer behaviour for array params."
                .into(),
        );
        m.advisory = vec!["GHSA-jwcz-9h9m-zp5q (CVE-2025-27152)".into()];
        m.bumps = vec!["axios 0.27.2 -> 1.8.4".into()];
        assert_eq!(
            m.to_string(),
            "security(axios)!: upgrade to 1.8.4, patching SSRF via absolute baseURL\n\
             \n\
             axios <1.8.2 resolves an absolute `url` against `baseURL`, letting a\n\
             caller-controlled path escape the configured host. The upgrade also\n\
             changes default paramsSerializer behaviour for array params.\n\
             \n\
             Bumps: axios 0.27.2 -> 1.8.4\n\
             Advisory: GHSA-jwcz-9h9m-zp5q (CVE-2025-27152)\n"
        );
        assert_eq!(m.validate(), Ok(()));
        assert!(m.reads_like_two_intents());
    }

    #[test]
    fn header_only_message_is_one_line() {
        let m = Message::new(CommitType::Ignore, None, "add .gitignore".into());
        assert_eq!(m.to_string(), "ignore: add .gitignore\n");
        assert!(!m.reads_like_two_intents());
    }

    #[test]
    fn deps_requires_bumps() {
        let mut m = Message::new(CommitType::Deps, None, "update Cargo.lock".into());
        assert_eq!(
            m.validate(),
            Err("`deps` requires --bumps \"<pkg> <from> -> <to>\"".into())
        );
        m.bumps = vec!["serde 1.0.0 -> 1.0.1".into()];
        assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn security_requires_advisory() {
        let mut m = Message::new(CommitType::Security, None, "patch ssrf".into());
        assert_eq!(
            m.validate(),
            Err("`security` requires --advisory <id>".into())
        );
        m.advisory = vec!["CVE-2025-1".into()];
        assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn advisory_is_security_only() {
        let mut m = Message::new(CommitType::Fix, None, "patch ssrf".into());
        m.advisory = vec!["CVE-2025-1".into()];
        assert_eq!(
            m.validate(),
            Err("--advisory is only valid with --type security".into())
        );
    }

    #[test]
    fn perf_requires_body() {
        let mut m = Message::new(CommitType::Perf, None, "cache parse table".into());
        assert_eq!(
            m.validate(),
            Err("`perf` requires --body with a before/after measurement".into())
        );
        m.body = Some("p50 12ms -> 3ms".into());
        assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn assisted_by_shape_is_enforced() {
        let mut m = Message::new(CommitType::Refactor, None, "update src/a.rs".into());
        for bad in [
            "claude",
            "claude omp mode=autonomous",
            "claude omp/1 mode=yolo",
            "claude omp/1 mode=autonomous extra",
        ] {
            m.assisted_by = Some(bad.into());
            assert_eq!(
                m.validate(),
                Err("--assisted-by must be \"<model> <harness>/<version> mode=<suggested|supervised|autonomous>\"".into()),
                "{bad}"
            );
        }
        m.assisted_by = Some("claude omp/1 mode=supervised".into());
        assert_eq!(m.validate(), Ok(()));
    }

    #[test]
    fn header_over_100_chars_is_rejected() {
        let mut m = Message::new(CommitType::Refactor, None, "x".repeat(90));
        assert_eq!(m.validate(), Ok(()));
        m.subject = "x".repeat(91);
        assert_eq!(
            m.validate(),
            Err("header is 101 chars; commitlint limit is 100".into())
        );
    }

    #[test]
    fn retired_and_unknown_types_are_rejected() {
        assert_eq!("feat".parse::<CommitType>(), Ok(CommitType::Feat));
        assert_eq!(
            "chore".parse::<CommitType>(),
            Err("retired type `chore`: use a specific type (`chore`), `refactor` or `build` (`style`), or the original type plus `--reverts` (`revert`)".into())
        );
        assert_eq!(
            "wip".parse::<CommitType>(),
            Err("unknown type `wip`".into())
        );
    }
}
