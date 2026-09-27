use std::collections::BTreeMap;

use crate::change::Change;
use crate::classify::{self, Category};
use crate::jev::{Criteria, Noul, State};
use crate::message::CommitType;

pub const NAMED_VULNERABILITY: &str = "named_vulnerability";
pub const OBSERVABLE_DELTA: &str = "observable_delta";
pub const ADDS_CAPABILITY: &str = "adds_capability";
pub const CONTRADICTED_STATED_CONTRACT: &str = "contradicted_stated_contract";
pub const TEST_EXPECTATION_CHANGED: &str = "test_expectation_changed";
pub const MEASURED_RESOURCE_CHANGE: &str = "measured_resource_change";
pub const CONSUMER_MUST_CHANGE: &str = "consumer_must_change";

fn noul(instructions: &'static str, yes: &'static str, no: &'static str) -> Noul {
    Noul {
        kind: "noul",
        instructions,
        criteria: Criteria { yes, no },
    }
}

/// The questions for one request. `has_tests` is whether the source group
/// contains a test file; when it does not, the Rule F predicate is omitted
/// rather than asked about a diff that cannot answer it. Decision clause 12.
pub fn questions(has_tests: bool) -> BTreeMap<&'static str, Noul> {
    let mut questions = BTreeMap::new();

    questions.insert(
        NAMED_VULNERABILITY,
        noul(
            "The `diff` fixes a specific, named security vulnerability in this project.",
            "The change removes a concrete security weakness that can be named, such as an \
             injection, an authentication bypass, a path traversal, or a leaked secret.",
            "The change fixes no security weakness, or only hardens something that was not \
             broken.",
        ),
    );
    questions.insert(
        OBSERVABLE_DELTA,
        noul(
            "After this change, a user of this project sees different output, different \
             accepted input, or different errors. Ignore how fast it runs and how much memory \
             it uses.",
            "The change alters what the project prints, accepts, returns, or rejects.",
            "The change rearranges code, renames things, or adjusts comments, and every \
             output, input, and error stays the same.",
        ),
    );
    questions.insert(
        ADDS_CAPABILITY,
        noul(
            "The `diff` lets a user of this project do something they could not do at all \
             before this change.",
            "The change introduces a new command, flag, function, endpoint, format, or output \
             that did not exist before.",
            "Everything the change touches could already be reached before; the change alters \
             existing behavior rather than adding to it.",
        ),
    );
    questions.insert(
        CONTRADICTED_STATED_CONTRACT,
        noul(
            "Before this change the project already promised the behavior the `diff` now \
             produces — in documentation, a type signature, a test, or a help string — and the \
             code did not match that promise.",
            "The `diff` shows an existing promise being brought back into agreement with the \
             code, with the promise visible in the removed or unchanged lines.",
            "Nothing in the project promised this behavior before the change.",
        ),
    );
    if has_tests {
        questions.insert(
            TEST_EXPECTATION_CHANGED,
            noul(
                "The `diff` changes what an existing test asserts, rather than adding a test \
                 for behavior that was already asserted elsewhere.",
                "An assertion's expected value, its condition, or the behavior it pins is \
                 different after the change.",
                "The test changes only add coverage, rename things, or adjust setup, and every \
                 existing assertion still expects the same thing.",
            ),
        );
    }
    questions.insert(
        MEASURED_RESOURCE_CHANGE,
        noul(
            "The `diff` changes how much time, memory, or input/output this project consumes, \
             while producing the same output as before.",
            "The change makes the same work cheaper or more expensive in time, memory, or \
             input/output.",
            "The change does not affect how much time, memory, or input/output the project \
             consumes.",
        ),
    );
    questions.insert(
        CONSUMER_MUST_CHANGE,
        noul(
            "After this change, a user of this project must change something on their side to \
             keep working.",
            "An existing command, flag, function signature, format, or output was removed or \
             changed so that existing usage stops working.",
            "Everything that worked before this change still works unchanged.",
        ),
    );
    questions
}

/// Jev's budget is 32k tokens for `state` plus the longest question; this
/// leaves room for the questions and for the estimate being rough. Decision
/// clause 9.
pub const MAX_STATE_TOKENS: usize = 24_000;
pub const BYTES_PER_TOKEN: usize = 4;

/// Whether the group carries a test file, by the same rule `classify` uses.
/// Decision clause 12.
pub fn has_tests(changes: &[Change]) -> bool {
    changes
        .iter()
        .any(|c| classify::category(c) == Category::Test)
}

/// The `state` for one request: the source group's paths and its staged
/// diff. Clause 9: the estimate is the serialized byte length over four, and
/// an oversize state is a refusal, never a truncation — silently trimming a
/// diff would change the answer invisibly.
pub fn state(changes: &[Change], diff: String) -> Result<State, String> {
    let files: Vec<String> = changes
        .iter()
        .flat_map(Change::pathspecs)
        .map(str::to_string)
        .collect();
    let state = State { files, diff };
    let tokens = serde_json::to_string(&state)
        .map_err(|e| format!("jev state: {e}"))?
        .len()
        / BYTES_PER_TOKEN;
    if tokens > MAX_STATE_TOKENS {
        return Err(format!(
            "staged diff is too large for jev: about {tokens} tokens, limit {MAX_STATE_TOKENS}; \
             commit a smaller selection or pass --type"
        ));
    }
    Ok(state)
}

/// Predicate id to probability, as returned by jev.
pub type Answers = BTreeMap<String, f64>;

/// The values a Noul must clear to count as true or false. A value inside the
/// band is ambiguous. Decision clause 14.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub low: f64,
    pub high: f64,
}

/// Tuned against `jev-1.13.0`; see the sweep `utils/eval.rs` prints before
/// changing either bound. Decision clause 15.
pub const DEFAULT_BAND: Band = Band {
    low: 0.25,
    high: 0.75,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub kind: CommitType,
    pub breaking: bool,
    /// Every predicate the cascade actually consulted, in order. The eval
    /// attributes a wrong type to a predicate with this; `cmt` ignores it.
    pub read: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub predicate: &'static str,
    pub value: f64,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "jev was unsure: `{}` answered {:.2}, inside the ambiguous band; \
             pass --type to decide it yourself",
            self.predicate, self.value
        )
    }
}

/// Reads one predicate, recording it in `trace`. A predicate absent from the
/// response is false, never ambiguous. Decision clause 12.
fn read(
    answers: &Answers,
    id: &'static str,
    band: Band,
    trace: &mut Vec<&'static str>,
) -> Result<bool, Refusal> {
    trace.push(id);
    let Some(&value) = answers.get(id) else {
        return Ok(false);
    };
    if value > band.high {
        Ok(true)
    } else if value < band.low {
        Ok(false)
    } else {
        Err(Refusal {
            predicate: id,
            value,
        })
    }
}

/// Walks the agent-commits precedence cascade. `body_has_measurement` is
/// whether `--body` carries a number, which Rule P requires before `perf` is
/// reachable at all. Decision clause 13.
pub fn decide(
    answers: &Answers,
    body_has_measurement: bool,
    band: Band,
) -> Result<Intent, Refusal> {
    let mut trace = Vec::new();
    let kind = if read(answers, NAMED_VULNERABILITY, band, &mut trace)? {
        CommitType::Security
    } else if read(answers, OBSERVABLE_DELTA, band, &mut trace)? {
        if read(answers, CONTRADICTED_STATED_CONTRACT, band, &mut trace)? {
            CommitType::Fix
        } else if read(answers, ADDS_CAPABILITY, band, &mut trace)? {
            CommitType::Feat
        } else {
            CommitType::Fix
        }
    } else if read(answers, TEST_EXPECTATION_CHANGED, band, &mut trace)? {
        CommitType::Fix
    } else if body_has_measurement && read(answers, MEASURED_RESOURCE_CHANGE, band, &mut trace)? {
        CommitType::Perf
    } else {
        CommitType::Refactor
    };
    let breaking = read(answers, CONSUMER_MUST_CHANGE, band, &mut trace)?;
    Ok(Intent {
        kind,
        breaking,
        read: trace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::Status;

    #[test]
    fn sends_every_predicate_when_tests_moved() {
        let questions = questions(true);

        assert_eq!(questions.len(), 7);
        assert!(questions.contains_key(TEST_EXPECTATION_CHANGED));
    }

    #[test]
    fn omits_the_test_predicate_when_no_test_moved() {
        let questions = questions(false);

        assert_eq!(questions.len(), 6);
        assert!(!questions.contains_key(TEST_EXPECTATION_CHANGED));
    }

    #[test]
    fn every_question_is_a_noul_with_both_criteria() {
        for (id, noul) in questions(true) {
            assert_eq!(noul.kind, "noul", "{id}");
            assert!(!noul.instructions.is_empty(), "{id}");
            assert!(!noul.criteria.yes.is_empty(), "{id}");
            assert!(!noul.criteria.no.is_empty(), "{id}");
        }
    }

    fn changed(path: &str) -> Change {
        Change {
            path: path.into(),
            status: Status::Modified,
        }
    }

    #[test]
    fn state_carries_the_group_paths_and_diff() {
        let changes = [changed("src/a.rs"), changed("tests/a.rs")];
        let state = state(&changes, "@@ -1 +1 @@\n".into()).expect("state");

        assert_eq!(state.files, vec!["src/a.rs", "tests/a.rs"]);
        assert_eq!(state.diff, "@@ -1 +1 @@\n");
    }

    #[test]
    fn state_includes_the_old_path_of_a_rename() {
        let changes = [Change {
            path: "src/new.rs".into(),
            status: Status::Renamed {
                from: "src/old.rs".into(),
            },
        }];
        let state = state(&changes, String::new()).expect("state");

        assert_eq!(state.files, vec!["src/old.rs", "src/new.rs"]);
    }

    #[test]
    fn an_oversize_diff_is_refused_not_truncated() {
        let changes = [changed("src/a.rs")];
        let huge = "x".repeat(MAX_STATE_TOKENS * BYTES_PER_TOKEN + 1);
        let err = state(&changes, huge).expect_err("should refuse");

        assert!(err.contains("too large"), "unexpected error: {err}");
        assert!(err.contains("--type"), "should name the way out: {err}");
    }

    #[test]
    fn has_tests_reads_the_group_not_the_model() {
        assert!(has_tests(&[changed("src/a.rs"), changed("tests/cli.rs")]));
        assert!(!has_tests(&[changed("src/a.rs")]));
    }

    fn answers(pairs: &[(&str, f64)]) -> Answers {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    /// Every predicate false; the cascade falls through to refactor.
    fn quiet() -> Vec<(&'static str, f64)> {
        vec![
            (NAMED_VULNERABILITY, 0.0),
            (OBSERVABLE_DELTA, 0.0),
            (ADDS_CAPABILITY, 0.0),
            (CONTRADICTED_STATED_CONTRACT, 0.0),
            (TEST_EXPECTATION_CHANGED, 0.0),
            (MEASURED_RESOURCE_CHANGE, 0.0),
            (CONSUMER_MUST_CHANGE, 0.0),
        ]
    }

    fn with(overrides: &[(&'static str, f64)]) -> Answers {
        let mut out: Answers = quiet()
            .into_iter()
            .map(|(id, value)| (id.to_string(), value))
            .collect();
        for (id, value) in overrides {
            out.insert((*id).to_string(), *value);
        }
        out
    }

    #[test]
    fn falls_through_to_refactor() {
        let out = decide(&with(&[]), false, DEFAULT_BAND).expect("decided");

        assert_eq!(out.kind, CommitType::Refactor);
        assert!(!out.breaking);
    }

    #[test]
    fn a_named_vulnerability_wins_outright() {
        let out = decide(
            &with(&[(NAMED_VULNERABILITY, 0.9), (OBSERVABLE_DELTA, 0.9)]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Security);
    }

    #[test]
    fn a_broken_promise_is_a_fix_even_when_it_adds_capability() {
        let out = decide(
            &with(&[
                (OBSERVABLE_DELTA, 0.9),
                (CONTRADICTED_STATED_CONTRACT, 0.9),
                (ADDS_CAPABILITY, 0.9),
            ]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Fix);
    }

    #[test]
    fn a_new_capability_nothing_promised_is_a_feat() {
        let out = decide(
            &with(&[(OBSERVABLE_DELTA, 0.9), (ADDS_CAPABILITY, 0.9)]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Feat);
    }

    #[test]
    fn an_observable_change_that_adds_nothing_is_a_fix() {
        let out = decide(&with(&[(OBSERVABLE_DELTA, 0.9)]), false, DEFAULT_BAND).expect("decided");

        assert_eq!(out.kind, CommitType::Fix);
    }

    #[test]
    fn rule_f_makes_a_changed_expectation_a_fix() {
        let out = decide(
            &with(&[(TEST_EXPECTATION_CHANGED, 0.9)]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Fix);
    }

    #[test]
    fn rule_p_needs_a_measurement_in_the_body() {
        let measured = with(&[(MEASURED_RESOURCE_CHANGE, 0.9)]);

        assert_eq!(
            decide(&measured, false, DEFAULT_BAND)
                .expect("decided")
                .kind,
            CommitType::Refactor,
            "no body means no perf"
        );
        assert_eq!(
            decide(&measured, true, DEFAULT_BAND).expect("decided").kind,
            CommitType::Perf
        );
    }

    #[test]
    fn breaking_is_independent_of_the_type() {
        let out = decide(
            &with(&[(OBSERVABLE_DELTA, 0.9), (CONSUMER_MUST_CHANGE, 0.9)]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Fix);
        assert!(out.breaking);
    }

    #[test]
    fn a_missing_answer_reads_as_false() {
        let sparse = answers(&[(CONSUMER_MUST_CHANGE, 0.0)]);
        let out = decide(&sparse, false, DEFAULT_BAND).expect("decided");

        assert_eq!(out.kind, CommitType::Refactor);
    }

    #[test]
    fn an_ambiguous_branch_predicate_refuses() {
        let err = decide(&with(&[(OBSERVABLE_DELTA, 0.5)]), false, DEFAULT_BAND)
            .expect_err("should refuse");

        assert_eq!(err.predicate, OBSERVABLE_DELTA);
        assert_eq!(err.value, 0.5);
    }

    #[test]
    fn an_ambiguous_predicate_the_cascade_skipped_does_not_refuse() {
        // named_vulnerability decides at the first branch, so the later
        // ambiguous predicates are never read.
        let out = decide(
            &with(&[(NAMED_VULNERABILITY, 0.9), (OBSERVABLE_DELTA, 0.5)]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Security);
    }

    #[test]
    fn perf_is_not_consulted_without_a_body_so_it_cannot_refuse() {
        let out = decide(
            &with(&[(MEASURED_RESOURCE_CHANGE, 0.5)]),
            false,
            DEFAULT_BAND,
        )
        .expect("decided");

        assert_eq!(out.kind, CommitType::Refactor);
    }

    #[test]
    fn the_trace_records_only_the_predicates_read() {
        let out =
            decide(&with(&[(NAMED_VULNERABILITY, 0.9)]), false, DEFAULT_BAND).expect("decided");

        assert_eq!(out.read, vec![NAMED_VULNERABILITY, CONSUMER_MUST_CHANGE]);
    }

    #[test]
    fn a_wider_band_refuses_where_the_default_decides() {
        let answers = with(&[(OBSERVABLE_DELTA, 0.8)]);

        assert_eq!(
            decide(&answers, false, DEFAULT_BAND).expect("decided").kind,
            CommitType::Fix
        );
        assert!(
            decide(
                &answers,
                false,
                Band {
                    low: 0.1,
                    high: 0.9
                }
            )
            .is_err(),
            "0.8 is inside the wider band"
        );
    }
}
