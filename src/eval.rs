//! Scoring logic for the eval harness, extracted so `score`, `tally`, and
//! the baseline-drift diff are reachable by `cargo test`; `utils/eval.rs`
//! is a thin CLI wrapper over this module plus the live/bless side effects.

use std::collections::BTreeMap;

use crate::evalcase::Case;
use crate::intent::{self, Answers, Band};
use crate::message;
use crate::message::CommitType;

pub const CASES: &str = "evals/cases";
pub const RECORDINGS: &str = "evals/recordings";
pub const BASELINE: &str = "evals/baseline.toml";

/// What one case resolved to, or why it did not.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Decided {
        kind: CommitType,
        read: Vec<&'static str>,
    },
    Refused {
        predicate: &'static str,
    },
    Unrecorded,
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Outcome::Decided { kind, .. } => kind.as_str().to_string(),
            Outcome::Refused { predicate } => format!("refused:{predicate}"),
            Outcome::Unrecorded => "unrecorded".to_string(),
        }
    }
}

pub fn recording_path(name: &str) -> String {
    format!("{RECORDINGS}/{name}.json")
}

/// The recorded answers for a case, plus the token count it cost, if a
/// recording exists. A missing file is silently `None` (never ran
/// `eval-live`); a present-but-unparseable file logs distinctly, so the
/// report can tell the two apart.
pub fn recording(name: &str) -> Option<(Answers, u64)> {
    let raw = std::fs::read_to_string(recording_path(name)).ok()?;
    let response: crate::jev::Response = match serde_json::from_str(&raw) {
        Ok(response) => response,
        Err(e) => {
            eprintln!("{name}: corrupt recording: {e}");
            return None;
        }
    };
    let tokens = response.usage.input_tokens;
    let answers = response
        .answers
        .into_iter()
        .map(|(id, answer)| (id, answer.noul))
        .collect();
    Some((answers, tokens))
}

pub fn score(case: &Case, answers: Option<&Answers>, band: Band) -> Outcome {
    let Some(answers) = answers else {
        return Outcome::Unrecorded;
    };
    let has_measurement = case.body.as_deref().is_some_and(message::has_measurement);
    match intent::decide(answers, has_measurement, band) {
        Ok(intent) => Outcome::Decided {
            kind: intent.kind,
            read: intent.read,
        },
        Err(refusal) => Outcome::Refused {
            predicate: refusal.predicate,
        },
    }
}

/// Coverage and accuracy for one band, used by the sweep.
pub fn tally(
    cases: &[Case],
    answers: &BTreeMap<String, Answers>,
    band: Band,
) -> (usize, usize, usize) {
    let mut correct = 0;
    let mut answered = 0;
    for case in cases {
        if let Outcome::Decided { kind, .. } = score(case, answers.get(&case.name), band) {
            answered += 1;
            if kind == case.expected {
                correct += 1;
            }
        }
    }
    (correct, answered, cases.len())
}

/// Parses `name = "label"` lines, as written by `--bless`.
pub fn parse_baseline(raw: &str) -> BTreeMap<String, String> {
    raw.lines()
        .filter_map(|line| {
            let (name, label) = line.split_once(" = ")?;
            Some((
                name.trim().to_string(),
                label.trim().trim_matches('"').to_string(),
            ))
        })
        .collect()
}

pub fn read_baseline() -> BTreeMap<String, String> {
    parse_baseline(&std::fs::read_to_string(BASELINE).unwrap_or_default())
}

/// Every case whose `current` label differs from `expected`'s, in name
/// order, paired with its current label.
pub fn drift(
    current: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
) -> Vec<(String, String)> {
    current
        .iter()
        .filter(|(name, label)| expected.get(*name) != Some(*label))
        .map(|(name, label)| (name.clone(), label.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(name: &str, expected: CommitType) -> Case {
        Case {
            name: name.to_string(),
            expected,
            rationale: String::new(),
            body: None,
            files: vec!["src/a.rs".to_string()],
            diff: String::new(),
        }
    }

    fn answers(pairs: &[(&str, f64)]) -> Answers {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn outcome_labels_match_the_report_vocabulary() {
        assert_eq!(
            Outcome::Decided {
                kind: CommitType::Feat,
                read: vec![]
            }
            .label(),
            "feat"
        );
        assert_eq!(
            Outcome::Refused {
                predicate: "observable_delta"
            }
            .label(),
            "refused:observable_delta"
        );
        assert_eq!(Outcome::Unrecorded.label(), "unrecorded");
    }

    #[test]
    fn score_is_unrecorded_without_answers() {
        let c = case("x", CommitType::Feat);
        assert_eq!(score(&c, None, intent::DEFAULT_BAND), Outcome::Unrecorded);
    }

    #[test]
    fn score_decides_from_recorded_answers() {
        let c = case("x", CommitType::Feat);
        let a = answers(&[
            (intent::OBSERVABLE_DELTA, 0.9),
            (intent::ADDS_CAPABILITY, 0.9),
        ]);
        assert_eq!(
            score(&c, Some(&a), intent::DEFAULT_BAND),
            Outcome::Decided {
                kind: CommitType::Feat,
                read: vec![
                    intent::NAMED_VULNERABILITY,
                    intent::OBSERVABLE_DELTA,
                    intent::CONTRADICTED_STATED_CONTRACT,
                    intent::ADDS_CAPABILITY,
                    intent::CONSUMER_MUST_CHANGE,
                ],
            }
        );
    }

    #[test]
    fn score_refuses_on_an_ambiguous_predicate() {
        let c = case("x", CommitType::Feat);
        let a = answers(&[(intent::OBSERVABLE_DELTA, 0.5)]);
        assert_eq!(
            score(&c, Some(&a), intent::DEFAULT_BAND),
            Outcome::Refused {
                predicate: intent::OBSERVABLE_DELTA
            }
        );
    }

    #[test]
    fn tally_counts_only_decided_cases() {
        let cases = vec![case("a", CommitType::Feat), case("b", CommitType::Refactor)];
        let mut answers_map = BTreeMap::new();
        answers_map.insert(
            "a".to_string(),
            answers(&[
                (intent::OBSERVABLE_DELTA, 0.9),
                (intent::ADDS_CAPABILITY, 0.9),
            ]),
        );
        // "b" has no recording at all: Unrecorded, not counted as answered.
        let (correct, answered, total) = tally(&cases, &answers_map, intent::DEFAULT_BAND);

        assert_eq!((correct, answered, total), (1, 1, 2));
    }

    #[test]
    fn parse_baseline_reads_quoted_labels() {
        let raw = "new-flag = \"feat\"\nrename-only = \"refactor\"\n";
        let parsed = parse_baseline(raw);

        assert_eq!(parsed.get("new-flag").map(String::as_str), Some("feat"));
        assert_eq!(
            parsed.get("rename-only").map(String::as_str),
            Some("refactor")
        );
    }

    #[test]
    fn drift_reports_only_changed_labels() {
        let mut expected = BTreeMap::new();
        expected.insert("a".to_string(), "feat".to_string());
        expected.insert("b".to_string(), "refactor".to_string());
        let mut current = expected.clone();
        current.insert("a".to_string(), "fix".to_string());

        let changed = drift(&current, &expected);

        assert_eq!(changed, vec![("a".to_string(), "fix".to_string())]);
    }
}
