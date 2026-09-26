# Infer commit intent with jev Nouls

## Issue

`cmt` infers a commit type from paths alone. `README.md` states that `feat`,
`fix`, `perf`, and `security` are never inferred, because they describe intent
and a path scan cannot read it. Every change set containing product source is
therefore committed as `refactor` unless a human or an agent supplies `--type`.
The existing escape hatch, `--agent-prompt`, renders a prompt for a generative
model and asks it to answer with a `cmt` command line — a round trip through
free text to recover one categorical value.

TypeSafe's jev is a System One model: it answers typed questions — yes/no
(Noul), one-of-N (Choice), rubric level (Score) — and returns calibrated
probabilities rather than prose. That is the shape of the missing judgment.

## Status

This is a proposal that is **awaiting review**.

## Assumptions and Constraints

- No Rust SDK exists; TypeSafe ships Python and JavaScript only. The client is
  written by hand against `POST https://api.typesafe.ai/v1/systemone`.
- `jev-1.13` does not count reliably, is not trained to generate text, and
  loses accuracy as `state` grows with material unrelated to the question.
- Noul answers carry `type` and `noul` only. `confidence` exists on Choice and
  Score answers, not on Noul.
- Context budget is 64k tokens per request, and 32k for `state` plus the single
  longest question.
- `README.md` documents exit codes 0–3 as a contract. Behavior with the new
  flag absent must stay byte-identical, including that contract.
- `classify::group` marks exactly one group `source: true`: the `refactor`
  group built from product source and its tests. Every other group's type is
  settled by path with certainty.
- The agent-commits type rules already exist in `src/prompt.rs` as an ordered
  cascade with named rules N, F, and P. They are a decision procedure, not a
  judgment.
- `cmt` has one dependency, `clap`. It is a synchronous process that shells out
  to `git`; it has no async runtime and needs none.

## Argument

Of the three judgments a commit split requires — how many commits, which type
each carries, which changes belong to each — only the type maps onto a jev
primitive. A commit count is a count, and jev does not count. A partition of
files into commits is a generated artifact, and jev does not generate. This
decision covers the type alone; file grouping is a separate decision that can
be taken later without revisiting this one.

**A cascade of atomic Nouls, resolved in code. Chosen.** Each rule the
convention names is a single yes/no reading of the diff, and the precedence
between them is already written down and fully deterministic. Asking one
question per reading and walking the ladder in Rust keeps every judgment
atomic, keeps the ordering out of the model, and makes a wrong commit type
attributable to a named predicate rather than to an opaque verdict.

**One Choice over the five reachable types. Rejected.** It packs the cascade
and rules N, F, and P into a single question, which is the failure mode jev's
own guidance names first, and it spreads probability mass across five options
so `confidence` falls on changes that are in fact unambiguous.

**A hybrid Choice plus companion Nouls. Rejected.** It retains a native
`confidence` but requires a resolution rule for Choice–Noul disagreement, and
the two numbers are documented as not comparable.

The model is asked only what a model can read. Everything else — precedence,
the `--body` measurement check, the dead band, the type-to-trailer
requirements — stays in code, where it is testable without a network.

## Architectural Decision

### Surface

1. A new flag, `--jev`, asks jev to decide the source commit's intent. It
   requires `TYPESAFE_API_KEY` in the environment.
2. `--jev` conflicts with `--type`, `--also`, `--breaking`, `--single`, and
   `--agent-prompt`. Each either supplies the answer being asked for, or
   destroys the filtered state the answer depends on.
3. `--jev` is compatible with `--subject`, `--scope`, `--body`, `--advisory`,
   `--bumps`, `--dry-run`, and every provenance flag. jev never produces a
   subject; subjects stay mechanical.
4. Without `--jev`, `cmt` behaves exactly as it does today.
5. A new exit code, `4`, means "jev could not decide". It covers a missing key,
   a transport failure, a `state` over budget, and a dead-band refusal.
   `README.md`'s exit-code table gains this row.
6. `--jev` refuses with exit `4` when the change set has no `source: true`
   group, because there is nothing jev can contribute.

### Request

7. The model is pinned to `jev-1.13.0`, not the `jev-latest` alias, because the
   dead band in clause 13 is tuned against a specific version's calibration.
8. `state` is a JSON object built from the source group alone:

   ```json
   { "files": ["src/classify.rs", "tests/cli.rs"], "diff": "@@ …" }
   ```

   `diff` is `git diff --cached` restricted to that group's paths. Lockfiles,
   docs, CI, and build changes are excluded: their types are already settled,
   and they act as distractors.
9. A `state` estimated over 24,000 tokens is a refusal with exit `4`, not a
   truncation. Silently trimming a diff changes the answer invisibly, which
   contradicts clause 5.
10. Seven Nouls are sent in one request. The ids are contract:
    `named_vulnerability`, `observable_delta`, `adds_capability`,
    `contradicted_stated_contract`, `test_expectation_changed`,
    `measured_resource_change`, `consumer_must_change`.
11. `observable_delta` is worded to cover output, accepted input, and errors,
    and to exclude timing and resource use, so it cannot overlap
    `measured_resource_change`.
12. `test_expectation_changed` is omitted from the request when no test file is
    present in the source group. Whether a test file moved is something
    `classify::category` already knows.

### Resolution

13. Predicates are read in cascade order and the first match wins:

    1. `named_vulnerability` → `security`
    2. `observable_delta` and `contradicted_stated_contract` → `fix` (Rule N,
       broken promise)
    3. `observable_delta` and `adds_capability` → `feat` (Rule N, nothing
       claimed it)
    4. `observable_delta` → `fix`
    5. `test_expectation_changed` → `fix` (Rule F)
    6. `measured_resource_change`, and `--body` contains a number → `perf`
       (Rule P)
    7. otherwise → `refactor`

    `consumer_must_change` appends `!` to whichever type wins.
14. A Noul is true above `0.75` and false below `0.25`. A value inside that
    band refuses with exit `4` — but only when the cascade actually branched on
    that predicate. A predicate the cascade never reached cannot cause a
    refusal.
15. The band is a named const in `intent.rs`, chosen from the eval in clause 20
    rather than by feel.
16. Trailer requirements are not re-implemented. A jev-chosen `security`
    without `--advisory` fails the existing `Message::validate()` and exits `2`.
17. The resolved intent is applied at the point `apply_intent` writes today, to
    the primary group only.

### Modules

18. `src/jev.rs` holds wire types, a `Transport` trait, and its `ureq`
    implementation. It knows nothing about commits. `src/intent.rs` holds the
    seven question definitions and the cascade as a pure function over answers.
    It knows nothing about HTTP.
19. `src/lib.rs` is added and `src/main.rs` becomes a thin wrapper over it.
    `tests/cli.rs` drives the built binary as a subprocess, so no target can
    currently reach `intent::decide` directly; the eval harness must.
20. New dependencies: `ureq` 3, `serde` 1, `serde_json` 1. `ureq` is blocking
    and brings no async runtime.

### Eval

21. Fixtures live at `evals/cases/<name>.toml`: a diff, the expected type, and
    the labeller's one-line rationale. They are the ground truth; the labels are
    human, not harvested.
22. `utils/harvest.rs` walks a repository's history, reconstructs each commit's
    source-group diff through `classify::group`, and emits unlabeled fixture
    candidates for a human to label. It bootstraps the corpus and stays, so the
    corpus can be grown from any repository later.
23. `utils/eval.rs` scores the corpus. Both tools are declared in `Cargo.toml`
    as `[[example]]` targets with explicit `path`, so `cargo install --path .`
    never ships them.
24. A live run writes each raw jev response to `evals/recordings/<name>.json`.
    Replay mode scores the corpus from those recordings with no network, so a
    cascade change is gated offline. `just check` runs replay; `just eval-live`
    calls jev and rewrites the recordings.
25. The report scores jev and the mechanical classifier against the same labels,
    reporting accuracy, coverage, per-predicate error counts, a dead-band sweep,
    and token usage. The mechanical classifier is the always-`refactor`
    baseline and is reported as such: it is the floor a network call must clear.

## Consequences

- `README.md` gains the `--jev` flag, exit code `4`, and a note that the
  deterministic guarantee in its first line holds only without the flag.
- `CHANGELOG.md` does not yet exist in this repository; the flag is
  user-facing and needs an entry when it ships.
- `--jev` stages before it can fail, exactly as `--dry-run` and
  `--agent-prompt` already do. A refusal leaves the working tree staged.
- `contradicted_stated_contract` asks about the parent commit's claims while
  clause 8 sends only the diff. It is the predicate most likely to misread.
  Clause 25's per-predicate breakdown is what will show whether adding parent
  context is worth a second decision; that is deliberately not guessed now.
- The cascade makes `ROADMAP.md`'s `Also:` item mechanically reachable — the
  type that loses the cascade is visible rather than discarded. Emitting it is
  out of scope here and its roadmap line stays.
- `--agent-prompt` is unaffected and stays. It serves a different caller: an
  agent already in the loop, which `--jev` is not.

## Positions

**Ask jev for the number of commits.** Rejected. `jev-1.13` does not count
reliably, and its own guidance says an error grows with the size of the thing
counted. A commit count must fall out of grouping done in code.

**Ask jev which files or hunks belong in each commit.** Rejected here, deferred.
A partition is a generated artifact. The jev-shaped form is pairwise — "do these
two changes belong in the same commit?" — with clustering in code, and hunk
granularity additionally requires replacing `cmt`'s file-level staging with
synthesized patches. Both belong to a later decision.

**Composite scoring.** Rejected for this decision. Commit types are categorical,
not rankable, so there is nothing to weight. The pattern applies to the pairwise
cohesion question above, not to this one.

**Enable jev automatically when `TYPESAFE_API_KEY` is set.** Rejected. It gives
the best ergonomics inside an agent loop, but identical invocations would then
diverge between a developer's machine and CI, and `README.md`'s first word stops
being true.

**Fall back to the mechanical classifier on failure or low confidence.**
Rejected. It always produces a commit, but the same command yields a different
commit type depending on network weather, and a warning on stderr is easy to
miss inside an agent loop. A refusal that names the missing flag is more useful
than a quiet `refactor`.

**Harvest eval labels from commit subjects instead of labelling by hand.**
Rejected as ground truth, kept as a bootstrap. A harvested label is only as
trustworthy as the original committer, and the types this decision exists to
recover are exactly the ones historical commits get wrong. `utils/harvest.rs`
proposes candidates; a human labels them.

**`reqwest` for transport.** Rejected. It brings tokio and hyper into a binary
whose entire I/O model is to spawn `git` and wait.

## References

- [TypeSafe API reference](https://docs.typesafe.ai/api) — request and response
  shapes for `POST /v1/systemone`
- [Jev 1.13 jaggedness](https://docs.typesafe.ai/model-jaggedness/jev-1.13) —
  the counting, generation, large-state, and structural-invariant failure modes
  that shape clauses 8 through 14
- [Speculative fan-out](https://docs.typesafe.ai/patterns/fan-out) — why seven
  questions cost tokens rather than latency
- [Models](https://docs.typesafe.ai/models) — context budget, rate limits, and
  the guidance to pin a version when thresholds are tuned
- [agent-commits](https://github.com/phatblat/agent-commits) — the type rules
  the cascade in clause 13 implements
