# jev Commit-Intent Classifier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `--jev` flag that asks TypeSafe's jev seven yes/no questions about the staged source diff and resolves them into a conventional commit type through a cascade owned by Rust code.

**Decision:** [2026-09-26-infer-commit-intent-with-jev-nouls — Infer commit intent with jev Nouls](docs/decisions/2026-09-26-infer-commit-intent-with-jev-nouls.md)

**Architecture:** `cmt` classifies paths into groups today; exactly one group is marked `source: true` and always gets type `refactor`. `--jev` sends that group's diff as `state` plus seven Noul questions in one request, then walks a fixed precedence cascade in Rust to turn the seven probabilities into one of `security`, `fix`, `feat`, `perf`, or `refactor`. No network call decides ordering, arithmetic, or trailer requirements. An offline eval harness scores the cascade against a hand-labelled fixture corpus using recorded jev responses.

**Tech Stack:** Rust 2024 (1.98.1, pinned in `mise.toml`), `clap` 4, `ureq` 3 (blocking HTTP), `serde` + `serde_json`, `just` as the only command surface.

---

## Background for someone new to this codebase

- `cmt` is a single binary that stages dirty files, splits them into logical groups by path, and makes one commit per group. Read `README.md` first.
- `src/classify.rs:383` `group()` returns `Vec<Group>`. `src/classify.rs:566` sets `source: true` on exactly one of them — the `refactor` group holding product source and its tests. Every other group's type is decided by filename with certainty.
- `src/main.rs:241` `grouped()` picks that group as `primary` and `src/main.rs:133` `apply_intent()` writes `--type` onto it. That is the exact seam this plan plugs into.
- `src/git.rs` wraps `git` subprocess calls. `src/message.rs:87` `Message` is the commit message struct; `Message::validate()` at `src/message.rs:152` already enforces that `security` needs `--advisory` and `deps` needs `--bumps`. Do not re-implement those checks.
- Exit codes are a documented contract (`README.md:34-39`): `0` success, `1` git failure, `2` validation failed, `3` nothing to commit. This plan adds `4`.
- The repo has no `CHANGELOG.md` yet. Task 13 creates it.
- Run everything through `just`. Never call `cargo` directly except where a step says to.

## File Structure

| Path | Status | Responsibility |
|---|---|---|
| `src/lib.rs` | create | Declares every module so non-binary targets can reach them |
| `src/main.rs` | modify | Keeps `Cli`, `execute`, `grouped`, `single`; imports the rest from the lib |
| `src/jev.rs` | create | Wire types, the `Transport` trait, the `ureq` implementation. Knows nothing about commits |
| `src/intent.rs` | create | The seven question definitions, the `Band`, and the pure cascade. Knows nothing about HTTP |
| `utils/harvest.rs` | create | Proposes unlabelled fixture candidates from a repo's history |
| `utils/eval.rs` | create | Scores the corpus in replay or live mode against the baseline |
| `evals/cases/*.toml` | create | Hand-labelled fixtures — the ground truth |
| `evals/recordings/*.json` | create | Raw jev responses, so replay needs no network |
| `evals/baseline.toml` | create | Golden file: the type each recorded case currently resolves to |
| `Cargo.toml` | modify | Three dependencies, two `[[example]]` targets |
| `justfile` | modify | `eval`, `eval-live`, `harvest` recipes; `check` gains `eval` |
| `README.md` | modify | `--jev` flag, exit code `4`, determinism caveat |
| `CHANGELOG.md` | create | Keep a Changelog, `Unreleased` entry |

---

## Task 1: Add a library target

`utils/eval.rs` must call `intent::decide` directly. `tests/cli.rs` drives the built binary as a subprocess, so today nothing but `main.rs` can reach the modules. Cargo auto-detects `src/lib.rs` alongside `src/main.rs`; no `[lib]` section is needed.

**Files:**
- Create: `src/lib.rs`
- Modify: `src/main.rs:1-14`

- [ ] **Step 1: Create the library root**

Create `src/lib.rs`:

```rust
pub mod bumps;
pub mod change;
pub mod classify;
pub mod git;
pub mod message;
pub mod prompt;
pub mod scope;
pub mod subject;
```

- [ ] **Step 2: Point the binary at the library**

Replace `src/main.rs` lines 1-14 (the `mod` declarations and `use` statements) with:

```rust
use clap::Parser;

use cmt::change::Change;
use cmt::classify::{self, Group};
use cmt::git;
use cmt::message::{CommitType, Message};
use cmt::prompt;
```

Then fix the remaining references in `src/main.rs`: `Change::pathspecs` is used at line 230 via `change::Change::pathspecs`, `scope::scope` at line 304 and `subject::subject` at line 310 need `cmt::scope` and `cmt::subject` added to the `use` block. Compile errors will name every one.

- [ ] **Step 3: Verify the build and the existing suite**

Run: `just check`
Expected: PASS — formatting, clippy with `-D warnings`, and every test in `tests/cli.rs`.

- [ ] **Step 4: Commit**

```bash
cargo run --
```

Expected: one `refactor: ...` commit. This is an internal reorganisation with no observable delta.

---

## Task 2: Add dependencies and jev wire types

**Files:**
- Modify: `Cargo.toml`
- Create: `src/jev.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Add the dependencies**

In `Cargo.toml`, replace the `[dependencies]` section with:

```toml
[dependencies]
clap = { version = "4.6.6", features = ["derive"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
ureq = "3.4.2"
```

- [ ] **Step 2: Write the failing test**

Create `src/jev.rs` with only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_a_request_in_the_documented_shape() {
        let state = State {
            files: vec!["src/a.rs".into()],
            diff: "@@ -1 +1 @@\n-a\n+b\n".into(),
        };
        let mut questions = std::collections::BTreeMap::new();
        questions.insert(
            "is_urgent",
            Noul {
                kind: "noul",
                instructions: "Does this convey urgency?",
                criteria: Criteria {
                    yes: "It does",
                    no: "It does not",
                },
            },
        );
        let body = serde_json::to_string(&Request {
            state: &state,
            model: MODEL,
            questions,
        })
        .expect("serialize");

        assert!(body.contains(r#""model":"jev-1.13.0""#));
        assert!(body.contains(r#""type":"noul""#));
        assert!(body.contains(r#""criteria":{"true":"It does","false":"It does not"}"#));
        assert!(body.contains(r#""files":["src/a.rs"]"#));
    }

    #[test]
    fn deserializes_a_documented_response() {
        let raw = r#"{
          "model": "jev-1.13.0",
          "answers": { "is_urgent": { "type": "noul", "noul": 0.95 } },
          "usage": { "input_tokens": 296, "output_tokens": 20 }
        }"#;
        let parsed: Response = serde_json::from_str(raw).expect("parse");

        assert_eq!(parsed.answers["is_urgent"].noul, 0.95);
        assert_eq!(parsed.usage.input_tokens, 296);
    }
}
```

- [ ] **Step 2b: Register the module**

Add `pub mod jev;` to `src/lib.rs`, keeping the list alphabetical (after `git`).

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --lib jev`
Expected: FAIL — `cannot find struct State in this scope` and similar for every type.

- [ ] **Step 4: Write the types**

Prepend to `src/jev.rs`, above the test module:

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
pub const MODEL: &str = "jev-1.13.0";
pub const API_KEY_ENV: &str = "TYPESAFE_API_KEY";

/// The `state` field: the source group's paths and its staged diff, and
/// nothing else. Decision clause 8.
#[derive(Debug, Serialize)]
pub struct State {
    pub files: Vec<String>,
    pub diff: String,
}

#[derive(Debug, Serialize)]
pub struct Criteria {
    #[serde(rename = "true")]
    pub yes: &'static str,
    #[serde(rename = "false")]
    pub no: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Noul {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub instructions: &'static str,
    pub criteria: Criteria,
}

#[derive(Debug, Serialize)]
pub struct Request<'a> {
    pub state: &'a State,
    pub model: &'a str,
    pub questions: BTreeMap<&'static str, Noul>,
}

#[derive(Debug, Deserialize)]
pub struct Answer {
    pub noul: f64,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub struct Response {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test --lib jev`
Expected: PASS, 2 tests.

- [ ] **Step 6: Commit**

```bash
cargo run -- --type deps --bumps "serde 0 -> 1.0.229" --bumps "serde_json 0 -> 1.0.151" --bumps "ureq 0 -> 3.4.2"
```

---

## Task 3: The transport

The trait is the test seam. Production uses `ureq`; the eval harness and unit tests use a canned implementation, so `just check` never touches the network.

**Files:**
- Modify: `src/jev.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/jev.rs`:

```rust
    struct Canned(&'static str);

    impl Transport for Canned {
        fn post(&self, _body: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    #[test]
    fn ask_parses_answers_from_the_transport() {
        let canned = Canned(
            r#"{"model":"jev-1.13.0",
                "answers":{"a":{"type":"noul","noul":0.9}},
                "usage":{"input_tokens":1,"output_tokens":2}}"#,
        );
        let state = State {
            files: vec!["src/a.rs".into()],
            diff: "@@".into(),
        };
        let response = ask(&canned, &state, BTreeMap::new()).expect("ask");

        assert_eq!(response.answers["a"].noul, 0.9);
    }

    #[test]
    fn ask_reports_unparseable_bodies() {
        struct Garbage;
        impl Transport for Garbage {
            fn post(&self, _body: &str) -> Result<String, String> {
                Ok("not json".into())
            }
        }
        let state = State {
            files: Vec::new(),
            diff: String::new(),
        };
        let err = ask(&Garbage, &state, BTreeMap::new()).expect_err("should fail");

        assert!(err.contains("jev response"), "unexpected error: {err}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib jev`
Expected: FAIL — `cannot find trait Transport`, `cannot find function ask`.

- [ ] **Step 3: Write the transport**

Add to `src/jev.rs`, after the type definitions:

```rust
pub trait Transport {
    /// POST `body` to the evaluation endpoint and return the response body.
    fn post(&self, body: &str) -> Result<String, String>;
}

/// Serializes the request, sends it, and parses the response.
pub fn ask(
    transport: &dyn Transport,
    state: &State,
    questions: BTreeMap<&'static str, Noul>,
) -> Result<Response, String> {
    let request = Request {
        state,
        model: MODEL,
        questions,
    };
    let body = serde_json::to_string(&request).map_err(|e| format!("jev request: {e}"))?;
    let raw = transport.post(&body)?;
    serde_json::from_str(&raw).map_err(|e| format!("jev response: {e}"))
}

/// The real transport. Reads the API key from the environment on construction
/// so a missing key fails before any work is done.
pub struct Http {
    key: String,
}

impl Http {
    pub fn from_env() -> Result<Self, String> {
        let key = std::env::var(API_KEY_ENV)
            .map_err(|_| format!("--jev requires {API_KEY_ENV} in the environment"))?;
        Ok(Self { key })
    }
}

impl Transport for Http {
    fn post(&self, body: &str) -> Result<String, String> {
        ureq::post(ENDPOINT)
            .header("Authorization", &format!("Bearer {}", self.key))
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| format!("jev request failed: {e}"))?
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("jev response failed: {e}"))
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib jev`
Expected: PASS, 4 tests.

If `ureq`'s builder methods do not match, check the installed version's docs with `cargo doc --open -p ureq`; the shape above is `ureq` 3.x. Do not downgrade to `ureq` 2.

- [ ] **Step 5: Commit**

```bash
cargo run --
```

---

## Task 4: The seven questions

Decision clause 10 fixes the seven ids as contract. Clause 11 requires `observable_delta` to exclude timing so it cannot overlap `measured_resource_change`. Clause 12 omits `test_expectation_changed` when the source group has no test file.

**Files:**
- Create: `src/intent.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src/intent.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

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
}
```

- [ ] **Step 1b: Register the module**

Add `pub mod intent;` to `src/lib.rs`, after `pub mod git;`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib intent`
Expected: FAIL — `cannot find function questions`.

- [ ] **Step 3: Write the questions**

Prepend to `src/intent.rs`:

```rust
use std::collections::BTreeMap;

use crate::jev::{Criteria, Noul};

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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib intent`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
cargo run --
```

---

## Task 5: The cascade

This is the load-bearing logic. Decision clause 13 fixes the order, clause 14 the dead band and the rule that only a predicate the cascade *branched on* can refuse, clause 12 that a missing answer reads as false.

**Files:**
- Modify: `src/intent.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/intent.rs`:

```rust
    use crate::message::CommitType;

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
        let out = decide(&with(&[(OBSERVABLE_DELTA, 0.9)]), false, DEFAULT_BAND)
            .expect("decided");

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
            decide(&measured, false, DEFAULT_BAND).expect("decided").kind,
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
        let out = decide(&with(&[(NAMED_VULNERABILITY, 0.9)]), false, DEFAULT_BAND)
            .expect("decided");

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
            decide(&answers, false, Band { low: 0.1, high: 0.9 }).is_err(),
            "0.8 is inside the wider band"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib intent`
Expected: FAIL — `cannot find type Answers`, `cannot find function decide`.

- [ ] **Step 3: Write the cascade**

Add to `src/intent.rs`, after `questions()`:

```rust
use crate::message::CommitType;

/// Predicate id to probability, as returned by jev.
pub type Answers = BTreeMap<String, f64>;

/// The values a Noul must clear to count as true or false. A value inside the
/// band is ambiguous. Decision clause 14.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub low: f64,
    pub high: f64,
}

/// Tuned against `jev-1.13.0` by `utils/eval.rs`; see the sweep in its report
/// before changing either bound. Decision clause 15.
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

/// Reads one predicate, recording it in `read`. A predicate absent from the
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

/// Rule P's precondition: a measurement is a number in the body.
pub fn has_measurement(body: Option<&str>) -> bool {
    body.is_some_and(|b| b.chars().any(|c| c.is_ascii_digit()))
}
```

Note the `body_has_measurement &&` short-circuit: without a body the `perf` predicate is never read, so it can never cause a refusal. That is what `perf_is_not_consulted_without_a_body_so_it_cannot_refuse` pins.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib intent`
Expected: PASS, 16 tests.

- [ ] **Step 5: Commit**

```bash
cargo run --
```

---

## Task 6: Build the state, with the size guard

Decision clause 8 restricts `state` to the source group; clause 9 makes an oversize state a refusal, never a truncation, estimated as bytes ÷ 4.

**Files:**
- Modify: `src/intent.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/intent.rs`:

```rust
    use crate::change::{Change, Status};

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
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib intent`
Expected: FAIL — `cannot find function state`, `cannot find function has_tests`.

- [ ] **Step 3: Write the state builder**

Add to `src/intent.rs`:

```rust
use crate::change::Change;
use crate::classify::{self, Category};
use crate::jev::State;

/// Jev's budget is 32k tokens for `state` plus the longest question; this
/// leaves room for the questions and for the estimate being rough.
/// Decision clause 9.
pub const MAX_STATE_TOKENS: usize = 24_000;
pub const BYTES_PER_TOKEN: usize = 4;

/// Whether the group carries a test file, which decides if Rule F's predicate
/// is worth asking. Decision clause 12.
pub fn has_tests(changes: &[Change]) -> bool {
    changes
        .iter()
        .any(|c| classify::category(c) == Category::Test)
}

/// The `state` for one request: the source group's paths and its staged diff.
pub fn state(changes: &[Change], diff: String) -> Result<State, String> {
    let files: Vec<String> = changes
        .iter()
        .flat_map(|c| c.pathspecs())
        .map(str::to_string)
        .collect();
    let bytes: usize = diff.len() + files.iter().map(|f| f.len() + 4).sum::<usize>();
    let tokens = bytes / BYTES_PER_TOKEN;
    if tokens > MAX_STATE_TOKENS {
        return Err(format!(
            "staged diff is too large for jev: about {tokens} tokens, limit {MAX_STATE_TOKENS}; \
             commit a smaller selection or pass --type"
        ));
    }
    Ok(State { files, diff })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib intent`
Expected: PASS, 20 tests.

- [ ] **Step 5: Commit**

```bash
cargo run --
```

---

## Task 7: Wire `--jev` into the CLI

**Files:**
- Modify: `src/main.rs:96-109` (flags), `src/main.rs:150-166` (`Failure`), `src/main.rs:185-199` (`execute`), `src/main.rs:241-281` (`grouped`)
- Test: `tests/cli.rs`

- [ ] **Step 1: Write the failing test**

Add to `tests/cli.rs`, following the existing `Repo` helper style in that file:

```rust
#[test]
fn jev_without_a_key_exits_four() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() {}\npub fn g() {}\n");

    let out = repo.cmt(&["--jev"]);

    assert_eq!(out.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("TYPESAFE_API_KEY"),
        "stderr should name the missing variable"
    );
}

#[test]
fn jev_conflicts_with_type() {
    let repo = Repo::seeded();
    repo.write("src/lib.rs", "pub fn f() {}\npub fn g() {}\n");

    let out = repo.cmt(&["--jev", "--type", "feat"]);

    assert_ne!(out.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot be used with"),
        "clap should reject the combination"
    );
}

#[test]
fn jev_without_a_source_group_exits_four() {
    let repo = Repo::seeded();
    repo.write("README.md", "# seed\n\nmore\n");

    let out = repo.cmt(&["--jev"]);

    assert_eq!(out.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no source"),
        "stderr should say why there is nothing to ask about"
    );
}
```

`Repo::cmt` is the existing helper that runs the built binary; check its exact name at `tests/cli.rs` before writing and use whatever the file already calls it. The `--jev` tests must not set `TYPESAFE_API_KEY`; the third test needs it unset too, since the source-group check runs first.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test cli jev`
Expected: FAIL — `error: unexpected argument '--jev' found`.

- [ ] **Step 3: Add the flag**

In `src/main.rs`, after the `dry_run` field (line 98) and before `agent_prompt`:

```rust
    /// ask jev to decide the source commit's intent; needs TYPESAFE_API_KEY
    #[arg(
        long,
        conflicts_with_all = ["kind", "also", "breaking", "single", "agent_prompt"]
    )]
    jev: bool,
```

Add `"jev"` to the existing `conflicts_with_all` list on `agent_prompt` so the conflict is declared from both sides — clap requires only one direction, but the help text reads correctly with both.

- [ ] **Step 4: Add the failure constructor**

In `src/main.rs`, add to `impl Failure`:

```rust
    fn jev(reason: impl Into<String>) -> Self {
        Self {
            code: 4,
            reason: reason.into(),
        }
    }
```

- [ ] **Step 5: Resolve the intent in `execute`**

In `src/main.rs`, replace lines 194-199 with:

```rust
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
```

Add the resolver, after `execute`:

```rust
/// Asks jev about the source group and resolves the answers into an intent.
/// Every failure here is exit 4: a missing key, an unreachable endpoint, a
/// diff over budget, or an answer too close to call.
fn resolve_jev(cli: &Cli, groups: &[Group]) -> Result<Intent, Failure> {
    let group = groups
        .iter()
        .find(|g| g.source)
        .ok_or_else(|| Failure::jev("--jev needs a source commit; this change set has no source files"))?;

    let paths: Vec<String> = group
        .changes
        .iter()
        .flat_map(Change::pathspecs)
        .map(str::to_string)
        .collect();
    let diff = git::diff(&paths).map_err(Failure::git)?;
    let state = intent::state(&group.changes, diff).map_err(Failure::jev)?;

    let transport = jev::Http::from_env().map_err(Failure::jev)?;
    let questions = intent::questions(intent::has_tests(&group.changes));
    let response = jev::ask(&transport, &state, questions).map_err(Failure::jev)?;

    let answers: intent::Answers = response
        .answers
        .into_iter()
        .map(|(id, answer)| (id, answer.noul))
        .collect();
    intent::decide(
        &answers,
        intent::has_measurement(cli.body.as_deref()),
        intent::DEFAULT_BAND,
    )
    .map_err(|refusal| Failure::jev(refusal.to_string()))
}
```

Add to the `use` block at the top of `src/main.rs`:

```rust
use cmt::intent::{self, Intent};
use cmt::jev;
```

- [ ] **Step 6: Apply the intent in `grouped`**

Change the signature at `src/main.rs:241` to:

```rust
fn grouped(cli: &Cli, groups: Vec<Group>, jev: Option<Intent>) -> Result<Vec<Planned>, Failure> {
```

Inside the `map` closure, replace the `if is_primary { apply_intent(cli, &mut message); }` block with:

```rust
            let is_primary = Some(i) == primary;
            if is_primary {
                apply_intent(cli, &mut message);
                if let Some(intent) = &jev {
                    message.kind = intent.kind;
                    message.breaking = intent.breaking;
                }
            }
```

Order matters: `apply_intent` unconditionally writes `message.breaking = cli.breaking`, which is always `false` under `--jev` because the two flags conflict. jev's answer must be written after it, or it would be erased.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `just check`
Expected: PASS — the three new `jev` tests and every pre-existing test.

- [ ] **Step 8: Commit**

```bash
cargo run -- --type feat --subject "add --jev to infer commit intent from a staged diff"
```

---

## Task 8: The fixture format

The corpus is data. Fixtures are hand-labelled; `utils/harvest.rs` only proposes candidates.

**Files:**
- Create: `evals/cases/rename-only.toml`
- Create: `evals/cases/new-flag.toml`
- Modify: `src/lib.rs`
- Create: `src/evalcase.rs`

- [ ] **Step 1: Write the failing test**

Create `src/evalcase.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_labelled_case() {
        let raw = r#"
expected = "feat"
rationale = "Adds a flag that did not exist."
body = "before 10ms after 4ms"
files = ["src/main.rs"]
diff = "@@ -1 +1 @@\n+flag\n"
"#;
        let case = Case::parse("new-flag", raw).expect("parse");

        assert_eq!(case.name, "new-flag");
        assert_eq!(case.expected, CommitType::Feat);
        assert_eq!(case.files, vec!["src/main.rs"]);
        assert!(case.body.is_some());
    }

    #[test]
    fn rejects_an_unlabelled_case() {
        let raw = "expected = \"\"\nrationale = \"\"\nfiles = []\ndiff = \"\"\n";
        let err = Case::parse("blank", raw).expect_err("should reject");

        assert!(err.contains("unlabelled"), "unexpected error: {err}");
    }
}
```

- [ ] **Step 1b: Register the module**

Add `pub mod evalcase;` to `src/lib.rs`, after `pub mod classify;`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib evalcase`
Expected: FAIL — `cannot find struct Case`.

- [ ] **Step 3: Write the parser**

There is no TOML dependency and this plan does not add one — the fixture format is a fixed, flat subset that a short hand-written parser covers. Prepend to `src/evalcase.rs`:

```rust
use std::str::FromStr;

use crate::message::CommitType;

/// One hand-labelled fixture: a diff, the type a human says it deserves, and
/// the reasoning behind that label. Decision clause 21.
#[derive(Clone, Debug)]
pub struct Case {
    pub name: String,
    pub expected: CommitType,
    pub rationale: String,
    pub body: Option<String>,
    pub files: Vec<String>,
    pub diff: String,
}

/// Reads `key = "value"` and `key = ["a", "b"]` lines. Values are JSON
/// strings, so escapes and newlines round-trip through `serde_json`.
fn field(raw: &str, key: &str) -> Option<String> {
    raw.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key} = ")))
        .map(str::trim)
        .and_then(|value| serde_json::from_str::<String>(value).ok())
}

fn list(raw: &str, key: &str) -> Vec<String> {
    raw.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key} = ")))
        .map(str::trim)
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

impl Case {
    pub fn parse(name: &str, raw: &str) -> Result<Self, String> {
        let expected = field(raw, "expected").unwrap_or_default();
        if expected.is_empty() {
            return Err(format!(
                "case `{name}` is unlabelled: set `expected` to a commit type before scoring it"
            ));
        }
        Ok(Self {
            name: name.to_string(),
            expected: CommitType::from_str(&expected)?,
            rationale: field(raw, "rationale").unwrap_or_default(),
            body: field(raw, "body").filter(|b| !b.is_empty()),
            files: list(raw, "files"),
            diff: field(raw, "diff").unwrap_or_default(),
        })
    }

    /// Every `.toml` under `dir`, sorted by name.
    pub fn load_dir(dir: &std::path::Path) -> Result<Vec<Self>, String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        entries.sort();
        entries
            .iter()
            .map(|path| {
                let name = path.file_stem().unwrap_or_default().to_string_lossy();
                let raw = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Self::parse(&name, &raw)
            })
            .collect()
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib evalcase`
Expected: PASS, 2 tests.

- [ ] **Step 5: Write two seed fixtures by hand**

Create `evals/cases/rename-only.toml`:

```toml
expected = "refactor"
rationale = "Renames a private helper. No caller outside the module can tell."
body = ""
files = ["src/scope.rs"]
diff = "@@ -12,7 +12,7 @@\n-fn shared_prefix(paths: &[&str]) -> Option<String> {\n+fn common_prefix(paths: &[&str]) -> Option<String> {\n"
```

Create `evals/cases/new-flag.toml`:

```toml
expected = "feat"
rationale = "Adds --tested-by, a trailer a user could not emit before."
body = ""
files = ["src/main.rs", "src/message.rs"]
diff = "@@ -90,6 +90,9 @@ struct Cli {\n+    /// Tested-by: trailer (repeatable)\n+    #[arg(long)]\n+    tested_by: Vec<String>,\n"
```

These two are the minimum that exercises both ends of the cascade. Task 9 grows the corpus.

- [ ] **Step 6: Commit**

```bash
cargo run --
```

---

## Task 9: The harvester

Decision clause 22. It proposes candidates; a human labels them. It never writes a label.

**Files:**
- Create: `utils/harvest.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Declare the target**

Append to `Cargo.toml`:

```toml
[[example]]
name = "harvest"
path = "utils/harvest.rs"

[[example]]
name = "eval"
path = "utils/eval.rs"
```

`[[example]]` targets are built by `cargo build --examples` and run by `cargo run --example`, but `cargo install --path .` never ships them. Decision clause 23.

- [ ] **Step 2: Create a placeholder for the second target**

Cargo fails if a declared target's file is missing. Create `utils/eval.rs` containing only:

```rust
fn main() {}
```

Task 11 fills it in.

- [ ] **Step 3: Write the harvester**

Create `utils/harvest.rs`:

```rust
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

use cmt::change::{Change, Status};
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
    Ok(changes)
}

fn json(value: &str) -> String {
    serde_json::to_string(value).expect("string serializes")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (repo, count) = match args.as_slice() {
        [repo, count] => (Path::new(repo).to_path_buf(), count.parse().unwrap_or(20)),
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

    std::fs::create_dir_all("evals/cases").expect("create evals/cases");
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
        let mut args = vec!["show", "--format=", "-M", sha, "--"];
        args.extend(paths.iter().copied());
        let Ok(diff) = git(&repo, &args) else {
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
        std::fs::write(&path, body).expect("write case");
        written += 1;
    }
    println!("wrote {written} unlabelled candidates to evals/cases/");
    println!("label each one's `expected` field before running `just eval`");
}
```

- [ ] **Step 4: Run it against this repo**

Run: `cargo run --example harvest -- . 20`
Expected: a count of candidates written, and new `evals/cases/<sha>.toml` files each containing `expected = ""`.

- [ ] **Step 5: Label every candidate by hand**

Open each generated file. Read the diff. Set `expected` to one of `feat`, `fix`, `perf`, `security`, `refactor` and write one sentence in `rationale` saying why. Set `body` only when the change carries a measurement. Delete the comment lines. Delete any candidate whose diff is too trivial to be informative.

Do not copy the original subject into `expected` without reading the diff — the historical label is exactly what this corpus exists to check.

- [ ] **Step 6: Verify every case parses**

Run: `cargo test --lib evalcase`
Expected: PASS. Then confirm no unlabelled file remains:

Run: `grep -l 'expected = ""' evals/cases/ -r`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
cargo run --
```

---

## Task 10: The eval harness, replay mode

Decision clauses 24 and 25. Replay scores the corpus from recorded responses with no network, so a cascade change is caught offline.

**Files:**
- Modify: `utils/eval.rs`

- [ ] **Step 1: Write the harness**

Replace `utils/eval.rs` entirely:

```rust
//! Scores the cascade against the hand-labelled corpus.
//!
//! `just eval`       — replay from evals/recordings, compare to the baseline
//! `just eval-live`  — call jev, rewrite the recordings, then score
//! `just eval-bless` — replay and overwrite the baseline with the result

use std::collections::BTreeMap;
use std::path::Path;

use cmt::evalcase::Case;
use cmt::intent::{self, Answers, Band};
use cmt::jev;
use cmt::message::CommitType;

const CASES: &str = "evals/cases";
const RECORDINGS: &str = "evals/recordings";
const BASELINE: &str = "evals/baseline.toml";

/// What one case resolved to, or why it did not.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Decided { kind: CommitType, read: Vec<&'static str> },
    Refused { predicate: &'static str },
    Unrecorded,
}

impl Outcome {
    fn label(&self) -> String {
        match self {
            Outcome::Decided { kind, .. } => kind.as_str().to_string(),
            Outcome::Refused { predicate } => format!("refused:{predicate}"),
            Outcome::Unrecorded => "unrecorded".to_string(),
        }
    }
}

fn recording_path(name: &str) -> String {
    format!("{RECORDINGS}/{name}.json")
}

fn answers_from_recording(name: &str) -> Option<Answers> {
    let raw = std::fs::read_to_string(recording_path(name)).ok()?;
    let response: jev::Response = serde_json::from_str(&raw).ok()?;
    Some(
        response
            .answers
            .into_iter()
            .map(|(id, answer)| (id, answer.noul))
            .collect(),
    )
}

fn score(case: &Case, answers: Option<&Answers>, band: Band) -> Outcome {
    let Some(answers) = answers else {
        return Outcome::Unrecorded;
    };
    match intent::decide(answers, intent::has_measurement(case.body.as_deref()), band) {
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
fn tally(cases: &[Case], answers: &BTreeMap<String, Answers>, band: Band) -> (usize, usize, usize) {
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

fn read_baseline() -> BTreeMap<String, String> {
    std::fs::read_to_string(BASELINE)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (name, label) = line.split_once(" = ")?;
            Some((
                name.trim().to_string(),
                label.trim().trim_matches('"').to_string(),
            ))
        })
        .collect()
}

fn main() {
    let live = std::env::args().any(|a| a == "--live");
    let bless = std::env::args().any(|a| a == "--bless");

    let cases = match Case::load_dir(Path::new(CASES)) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if cases.is_empty() {
        eprintln!("no cases in {CASES}; run `just harvest` and label the output");
        std::process::exit(1);
    }

    if live {
        let transport = match jev::Http::from_env() {
            Ok(transport) => transport,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        };
        std::fs::create_dir_all(RECORDINGS).expect("create evals/recordings");
        for case in &cases {
            let state = jev::State {
                files: case.files.clone(),
                diff: case.diff.clone(),
            };
            let has_tests = case.files.iter().any(|f| f.contains("test"));
            let request = serde_json::to_string(&jev::Request {
                state: &state,
                model: jev::MODEL,
                questions: intent::questions(has_tests),
            })
            .expect("serialize");
            match jev::Transport::post(&transport, &request) {
                Ok(raw) => {
                    std::fs::write(recording_path(&case.name), &raw).expect("write recording");
                    println!("recorded {}", case.name);
                }
                Err(e) => eprintln!("{}: {e}", case.name),
            }
        }
    }

    let answers: BTreeMap<String, Answers> = cases
        .iter()
        .filter_map(|c| answers_from_recording(&c.name).map(|a| (c.name.clone(), a)))
        .collect();

    let mut correct = 0usize;
    let mut wrong = 0usize;
    let mut refused = 0usize;
    let mut unrecorded = 0usize;
    let mut baseline_correct = 0usize;
    let mut blame: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut current: BTreeMap<String, String> = BTreeMap::new();

    for case in &cases {
        if case.expected == CommitType::Refactor {
            baseline_correct += 1;
        }
        let outcome = score(case, answers.get(&case.name), intent::DEFAULT_BAND);
        current.insert(case.name.clone(), outcome.label());
        match &outcome {
            Outcome::Decided { kind, .. } if *kind == case.expected => correct += 1,
            Outcome::Decided { read, .. } => {
                wrong += 1;
                if let Some(last) = read.iter().rev().find(|id| **id != intent::CONSUMER_MUST_CHANGE) {
                    *blame.entry(last).or_default() += 1;
                }
            }
            Outcome::Refused { .. } => refused += 1,
            Outcome::Unrecorded => unrecorded += 1,
        }
    }

    let total = cases.len();
    let answered = correct + wrong;
    println!("\n{total} cases\n");
    println!(
        "{:<14}{:>9}{:>8}{:>10}{:>11}{:>11}",
        "", "correct", "wrong", "refused", "accuracy", "coverage"
    );
    println!(
        "{:<14}{:>9}{:>8}{:>10}{:>11.3}{:>11.3}",
        "jev",
        correct,
        wrong,
        refused,
        if answered == 0 { 0.0 } else { correct as f64 / answered as f64 },
        answered as f64 / total as f64,
    );
    println!(
        "{:<14}{:>9}{:>8}{:>10}{:>11.3}{:>11.3}",
        "mechanical",
        baseline_correct,
        total - baseline_correct,
        0,
        baseline_correct as f64 / total as f64,
        1.0,
    );
    if unrecorded > 0 {
        println!("\n{unrecorded} cases have no recording; run `just eval-live`");
    }
    if !blame.is_empty() {
        println!("\nwrong by predicate");
        for (predicate, count) in &blame {
            println!("  {predicate:<32}{count}");
        }
    }

    println!("\ndead band sweep");
    println!("  {:<14}{:>10}{:>11}", "band", "coverage", "accuracy");
    for (low, high) in [(0.10, 0.90), (0.20, 0.80), (0.25, 0.75), (0.35, 0.65), (0.40, 0.60)] {
        let (c, a, t) = tally(&cases, &answers, Band { low, high });
        println!(
            "  {:<14}{:>10.3}{:>11.3}",
            format!("{low:.2}-{high:.2}"),
            a as f64 / t as f64,
            if a == 0 { 0.0 } else { c as f64 / a as f64 },
        );
    }

    if bless {
        let rendered: String = current
            .iter()
            .map(|(name, label)| format!("{name} = \"{label}\"\n"))
            .collect();
        std::fs::write(BASELINE, rendered).expect("write baseline");
        println!("\nbaseline rewritten with {} entries", current.len());
        return;
    }

    let expected = read_baseline();
    if expected.is_empty() {
        eprintln!("\nno baseline at {BASELINE}; run `just eval-bless` once the report looks right");
        std::process::exit(1);
    }
    let drift: Vec<_> = current
        .iter()
        .filter(|(name, label)| expected.get(*name) != Some(*label))
        .collect();
    if drift.is_empty() {
        println!("\nbaseline matches");
        return;
    }
    eprintln!("\nbaseline drift:");
    for (name, label) in drift {
        eprintln!(
            "  {name}: baseline {:?}, now {label:?}",
            expected.get(name).map(String::as_str).unwrap_or("(absent)")
        );
    }
    eprintln!("fix the cascade, or run `just eval-bless` if the new result is correct");
    std::process::exit(1);
}
```

- [ ] **Step 2: Verify it compiles and reports the missing recordings**

Run: `cargo run --example eval`
Expected: the report prints, every case shows as unrecorded, and it exits `1` with "no baseline at evals/baseline.toml".

- [ ] **Step 3: Commit**

```bash
cargo run --
```

---

## Task 11: Record against live jev and set the baseline

This is the only task that needs the network and a key.

**Files:**
- Create: `evals/recordings/*.json`
- Create: `evals/baseline.toml`

- [ ] **Step 1: Confirm the key is present**

Run: `test -n "$TYPESAFE_API_KEY" && echo present`
Expected: `present`. If it is absent, get a key from https://console.typesafe.ai/keys and export it. Do not proceed without it; there is nothing to record.

- [ ] **Step 2: Record every case**

Run: `cargo run --example eval -- --live`
Expected: one `recorded <name>` line per case, then the full report with real numbers.

- [ ] **Step 3: Read the report before blessing anything**

Look at three things and write what you find in the commit body:
1. jev's accuracy against the mechanical baseline. If jev does not clearly beat the always-`refactor` floor, stop and report that — the Decision's premise is in question and that is a finding, not a failure to work around.
2. The `wrong by predicate` column. The Decision predicts `contradicted_stated_contract` dominates.
3. The sweep. If a band other than `0.25-0.75` is clearly better, change `DEFAULT_BAND` in `src/intent.rs` to match and re-run, per Decision clause 15.

- [ ] **Step 4: Write the baseline**

Run: `cargo run --example eval -- --bless`
Expected: `baseline rewritten with N entries`, and `evals/baseline.toml` now exists.

- [ ] **Step 5: Confirm replay is green and offline**

Run: `env -u TYPESAFE_API_KEY cargo run --example eval`
Expected: the report, then `baseline matches`, exit `0`.

- [ ] **Step 6: Commit**

```bash
cargo run --
```

Put the three findings from Step 3 in the commit body.

---

## Task 12: Wire the recipes into `just`

**Files:**
- Modify: `justfile`

- [ ] **Step 1: Add the recipes**

In `justfile`, add a new group after the `tests` group:

```just
#
# eval group recipes
#

# Score the cascade against recorded jev responses; no network
[group('eval')]
eval:
    cargo run --quiet --example eval

# Call jev, rewrite the recordings, then score; needs TYPESAFE_API_KEY
[group('eval')]
eval-live:
    cargo run --quiet --example eval -- --live

# Replay and overwrite evals/baseline.toml with the current result
[group('eval')]
eval-bless:
    cargo run --quiet --example eval -- --bless

# Propose unlabelled fixture candidates from a repository's history
[group('eval')]
harvest repo="." count="20":
    cargo run --quiet --example harvest -- {{repo}} {{count}}
```

- [ ] **Step 2: Add `eval` to the gate**

Change the `check` recipe (`justfile:90`) to:

```just
# Run every gate: formatting, lint, tests, eval replay
[group('checks')]
check: format-check lint test eval
```

- [ ] **Step 3: Verify the whole gate is green offline**

Run: `env -u TYPESAFE_API_KEY just check`
Expected: PASS end to end, including `baseline matches`.

- [ ] **Step 4: Commit**

```bash
cargo run --
```

---

## Task 13: Documentation

Decision Consequences: `README.md` gains the flag, exit code `4`, and the determinism caveat; the change is user-facing so `CHANGELOG.md` gets an entry.

**Files:**
- Modify: `README.md:3-4`, `README.md:11-32`, `README.md:34-39`, `README.md:86-93`
- Create: `CHANGELOG.md`

- [ ] **Step 1: Correct the opening claim**

Replace `README.md` lines 3-4 with:

```markdown
Deterministically commits dirty files as one agent-commits conventional commit
per logical group, inferred from the paths. With `--jev`, the source commit's
type is instead decided by TypeSafe's jev model; that path is opt-in and is not
deterministic.
```

- [ ] **Step 2: Document the flag**

In the usage block, after the `--dry-run` line:

```text
  --jev                  ask jev to decide the source commit's intent;
                         needs TYPESAFE_API_KEY. Conflicts with --type,
                         --also, --breaking, --single, --agent-prompt
```

- [ ] **Step 3: Document exit code 4**

Add to the exit-code list:

```markdown
- `4` — `--jev` could not decide: no key, no source commit, transport failure,
  a diff over budget, or an answer too close to call. Nothing committed.
```

- [ ] **Step 4: Document how the flag decides**

Add a section after "How groups become commits":

```markdown
### How `--jev` decides

`feat`, `fix`, `perf`, and `security` describe intent, which a path scan cannot
read. `--jev` sends the source group's diff to
[jev](https://docs.typesafe.ai/) as seven yes/no questions in one request, then
resolves the answers through a fixed cascade in `src/intent.rs`:

| Order | Predicate | Type |
|---|---|---|
| 1 | `named_vulnerability` | `security` |
| 2 | `observable_delta` and `contradicted_stated_contract` | `fix` (Rule N) |
| 3 | `observable_delta` and `adds_capability` | `feat` (Rule N) |
| 4 | `observable_delta` | `fix` |
| 5 | `test_expectation_changed` | `fix` (Rule F) |
| 6 | `measured_resource_change` and a number in `--body` | `perf` (Rule P) |
| 7 | none of the above | `refactor` |

`consumer_must_change` appends `!`. The model answers only the seven questions;
the ordering, Rule P's measurement check, and every trailer requirement stay in
code. An answer between 0.25 and 0.75 on a predicate the cascade reads is a
refusal, not a guess.

`just eval` scores that cascade against a labelled corpus in `evals/`, offline,
and runs as part of `just check`. `just eval-live` refreshes the recordings.
```

- [ ] **Step 5: Add the development commands**

In the Development section, after `just check`:

```bash
just eval     # score the cascade against recorded jev responses, offline
just harvest  # propose new fixture candidates from a repo's history
```

- [ ] **Step 6: Create the changelog**

Create `CHANGELOG.md`:

```markdown
# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `--jev` asks TypeSafe's jev model to decide the source commit's intent,
  resolving seven yes/no answers through the agent-commits precedence cascade
  to pick `security`, `fix`, `feat`, `perf`, or `refactor`. Opt-in; requires
  `TYPESAFE_API_KEY`.
- Exit code `4`, meaning `--jev` could not decide.
```

- [ ] **Step 7: Verify the documented behavior matches**

Run: `cargo run -- --help`
Expected: the `--jev` line appears with the text from Step 2's intent, and the conflicts are listed.

- [ ] **Step 8: Commit**

```bash
cargo run --
```

Expect one `docs` commit covering `README.md` and `CHANGELOG.md`.

---

## Task 14: Retire the Plan

Every plan's last task, always.

- [ ] **Step 1: Confirm the work is done**

Run: `env -u TYPESAFE_API_KEY just check`
Expected: PASS, including `baseline matches`.

- [ ] **Step 2: Remove the Plan file**

```bash
git rm PLAN.md
```

- [ ] **Step 3: Commit removal of the Plan**

```bash
git commit -m "plan: done 2026-09-26-infer-commit-intent-with-jev-nouls" -- PLAN.md
```

- [ ] **Step 4: Record what shipped**

The `CHANGELOG.md` entry landed in Task 13, so nothing further is owed. The
Decision's status stays `accepted`; Conventional Docs defines no `implemented`
status and no `decision: implement` commit.
