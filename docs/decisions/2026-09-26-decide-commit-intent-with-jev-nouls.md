# Decide source-commit intent with jev atomic nouls

## Issue

`cmt` infers commit types from paths. `README.md:44-46` states the limit plainly:
`feat`, `fix`, `perf`, and `security` are never inferred, because they describe
intent and a path scan cannot read intent. Every change set that is really one of
those four requires the caller to supply `--type`, and the only assistance `cmt`
offers is `--agent-prompt` (`src/prompt.rs:139-163`), which renders a prompt for
an agent to read and answer with a `cmt` command line. That is a full round trip
through a generative model on every commit.

TypeSafe's jev is a System One model: it answers typed questions with calibrated
probabilities rather than generating text. The intent read that `--agent-prompt`
currently delegates is a small set of yes/no judgments about a diff, which is the
shape jev is built for.

This decision covers the source commit's type only. Deciding how many commits a
change set should become, and which files or hunks belong in each, are separate
problems with different failure modes; they are out of scope here and are named in
Positions.

## Status

This is a proposal that is **awaiting review**.

## Assumptions and Constraints

- jev has no Rust SDK. The documentation index at `https://docs.typesafe.ai/llms.txt`
  lists Python and JavaScript clients only, so the HTTP call is hand-rolled against
  `POST https://api.typesafe.ai/v1/systemone` with a bearer token.
- jev does not generate text, does not count reliably, and its score levels are not
  numerically calibrated. Per `https://docs.typesafe.ai/model-jaggedness/jev-1.13`,
  any output that is a partition, a count, or an interpolated number must be
  computed in code, not asked for.
- jev accuracy falls as `state` grows with material unrelated to the question.
- `jev-1.13.0` limits: 64k tokens per request, 32k for `state` plus the longest
  question, 1,200 requests per minute. Input is charged at $42/Btok; output is free.
- A Noul answer carries only `type` and `noul`. `confidence` is returned for Choice
  and Score answers only, so a Noul gate must be built from the value itself.
- `README.md:3` describes `cmt` as deterministic. The default invocation must remain
  byte-identical to today's and must not touch the network.
- The exit codes in `README.md:34-39` are a consumer contract. Adding one is a
  consumer-visible change.
- `src/classify.rs:566` sets `source: true` on exactly one group — the one built
  from source changes, always typed `CommitType::Refactor`. Every other group's type
  is settled by path with certainty.
- `just check` must pass with no network access and no API key.
- Current dependencies are `clap` and, for tests, `tempfile`. There is no lib target;
  `tests/cli.rs:40-48` drives the built binary as a subprocess.

## Argument

The agent-commits type rules are already a decision procedure. `src/prompt.rs:11-34`
states them as an ordered cascade — "read top to bottom; the first match is the
type" — with three named tiebreakers (Rules N, F, P) and an explicit precedence
clause. Ordering and precedence are a `match` statement. The only parts that need a
model are the semantic reads underneath: whether a user can observe a difference,
whether the project already claimed the behavior, whether a named vulnerability is
being closed.

**Chosen.** Ask one Noul per predicate, batched into a single request, and resolve
the cascade in Rust. This matches jev's documented guidance to give the model one
judgment per question and keep arithmetic and ordering in code. It also makes a
wrong commit type debuggable: the failing predicate and its probability are visible,
rather than an opaque vote. And because every branch is evaluated, the type that
lost the cascade is visible, which closes `ROADMAP.md:16-17` (`Also:`) as a byproduct
rather than as new work.

**Rejected: one Choice over the five reachable types.** Smaller and it would supply a
native `confidence`, but it packs the cascade and Rules N, F, and P into a single
judgment — the "hiding several judgments inside one question" failure the jaggedness
page names — and spreading probability across five options depresses confidence even
on clear-cut changes. Full reasoning in Positions.

**Rejected: a hybrid Choice plus companion Nouls.** Retains native confidence but
requires a stated resolution for Choice/Noul disagreement, and the jaggedness page is
explicit that the two are not numerically comparable. Full reasoning in Positions.

## Architectural Decision

### Invocation

1. A new flag `--jev` asks jev to decide the source commit's intent. Without it,
   `cmt` behaves exactly as it does today: no network, no key required, no change in
   output.

2. `--jev` conflicts with `--type`, `--also`, `--breaking`, `--single`, and
   `--agent-prompt`. The first three supply the answer jev is being asked for;
   `--single` collapses the groups and so destroys the filtered state clause 5
   depends on; `--agent-prompt` is the alternative delegation path. `--jev` is
   compatible with `--subject`, `--scope`, `--body`, `--advisory`, `--dry-run`, and
   every provenance flag.

3. jev never supplies a subject or a scope. Generating text is outside what the model
   does; subjects and scopes stay mechanical (`src/subject.rs`, `src/scope.rs`).

4. `--jev` resolves the target group with the same predicate `grouped()` uses,
   `groups.iter().position(|g| g.source)` (`src/main.rs:247-250`). If the change set
   has no source group, `--jev` fails per clause 12 rather than asking a question
   whose answer the path already settles.

### State

5. `state` is a JSON object carrying only the target group:

   ```json
   {
     "files": ["src/classify.rs", "tests/cli.rs"],
     "diff": "<git diff --cached -- <target group paths>>"
   }
   ```

   Lockfiles, docs, CI, and build paths are excluded; their groups are already typed
   and their content is a distractor. Fields are named so instructions can refer to
   them directly.

6. If the estimated token cost of `state` exceeds the budget, `cmt` refuses per
   clause 12. It does not truncate. A trimmed diff changes the answer invisibly,
   which contradicts clause 11. The estimate is a conservative byte-based bound;
   no tokenizer is vendored.

### Questions

7. Seven Noul questions are sent in one request. jev ingests `state` once and
   evaluates questions in parallel, so the count costs input tokens, not latency.

   | id | judgment |
   |---|---|
   | `named_vulnerability` | The diff remediates a specific, named security vulnerability. |
   | `observable_delta` | After this change a user sees different output, different accepted input, or different errors. Timing and resource use are excluded; `measured_resource_change` owns those. |
   | `adds_capability` | The diff makes something reachable that a user could not reach before. |
   | `contradicted_stated_contract` | Before this change the project already stated this behavior — in documentation, a type signature, a test, or help text — and the code did not match that statement. |
   | `test_expectation_changed` | The diff alters what a test asserts, rather than adding coverage for behavior already asserted. |
   | `measured_resource_change` | The diff changes time, memory, or I/O consumed, without changing output. |
   | `consumer_must_change` | A user must change something on their side to keep working after this change. |

8. `test_expectation_changed` is sent only when the target group contains a path that
   `classify::category` returns `Category::Test` for. Whether a test file moved is
   something code already knows.

### Cascade

9. The type is resolved in code, in this order. The first match wins.

   1. `named_vulnerability` → `security`
   2. `observable_delta`:
      - and `contradicted_stated_contract` → `fix` (Rule N: a broken promise)
      - and `adds_capability` → `feat` (Rule N: nothing claimed it)
      - otherwise → `fix`
   3. `test_expectation_changed` → `fix` (Rule F)
   4. `measured_resource_change` **and** `--body` contains a digit → `perf` (Rule P)
   5. otherwise → `refactor`

   Rule P's measurement requirement is a code check on `--body`, so `perf` is
   structurally unreachable without a measurement rather than dependent on the model
   honoring the rule.

10. `!` is appended when `consumer_must_change` passes the gate. When a branch below
    the winning one would also have fired, its type is emitted as `Also:`
    (`ROADMAP.md:16-17`). `Also:` is emitted for at most one type, the highest-ranked
    loser.

### Gating and failure

11. A predicate is *load-bearing* for a given change set when the cascade actually
    branched on it. `cmt --jev` refuses when any load-bearing predicate falls inside
    the dead band. Predicates the cascade never consulted are ignored; refusing on
    them would produce noise.

12. The dead band is a named constant in `src/intent.rs`, initially `0.25..=0.75`,
    chosen from eval data per clause 20 rather than by feel.

13. A new exit code `4` means "jev could not decide". It covers a missing
    `TYPESAFE_API_KEY`, a transport or HTTP failure, an over-budget state, no source
    group in the change set, and a dead-band refusal. The message names the cause and
    tells the caller to pass `--type`. Existing codes are unchanged: a jev-chosen
    `security` with no `--advisory` still fails `Message::validate()` and exits `2`,
    with no new code.

14. `--jev` stages before it can fail, exactly as `--dry-run` and `--agent-prompt`
    already do (`src/main.rs:186`). On exit `4` the working tree is staged and
    nothing is committed.

### Structure

15. Two new modules, flat, matching the existing `src/*.rs` layout:

    - `src/jev.rs` — wire types, a `Transport` trait, and the `ureq` implementation.
      Knows nothing about commits.
    - `src/intent.rs` — the seven question definitions, the dead band, and
      `decide()`, a pure function from answers plus the `--body` measurement flag to
      either an intent or a refusal. Knows nothing about HTTP.

16. `src/lib.rs` is added, exporting the existing modules; `src/main.rs` becomes a
    thin wrapper over it. This is required — there is no lib target today, so nothing
    outside the binary can call `intent::decide`, which the eval harness must do
    directly. Every module becomes unit-testable as a side effect.

17. `Transport` is the test seam. Unit tests and the replay eval supply canned
    responses; no test in `just check` opens a socket.

18. New dependencies: `ureq 3.4.2` (blocking, rustls, no async runtime),
    `serde 1.0.229`, `serde_json 1.0.151`. `reqwest` is not used; it would pull tokio
    and hyper into a binary whose entire I/O model is spawning `git` and waiting.

19. The `model` field is pinned to `jev-1.13.0`, not the `jev-latest` alias. The
    dead band in clause 12 is tuned against a specific version, which is the case
    TypeSafe's own documentation names for pinning, and it matches this repository's
    pinning of its toolchain in `mise.toml` and `--locked` in `just install`.

### Eval

20. An eval harness at `examples/eval.rs` scores the jev cascade and the mechanical
    classifier against hand-labeled cases. It reports, per path: correct, wrong,
    refused, accuracy over answered cases, and coverage; a breakdown of wrong answers
    by the predicate that misread; a sweep of accuracy against coverage across
    candidate dead bands; and total input tokens with cost.

21. The mechanical classifier is scored as a baseline, not as a comparison target. It
    always types the source group `refactor` (`src/classify.rs:565`), so its score is
    the floor that a network call has to clear, not a second opinion.

22. Labeled cases live at `evals/cases/<name>/`, each holding `diff` and
    `expected.toml`. `expected.toml` records the expected type, the expected `!` and
    `Also:` if any, and a one-line rationale.

23. A live run records each raw jev response to `evals/recordings/<name>.json`.
    Replay mode reads those recordings, so the cascade can be changed and re-scored
    offline and deterministically. Replay runs inside `just check`; a cascade
    regression is caught with no key and no network.

24. `--harvest <repo> --since <ref>` is a kept mode of the eval harness. It walks a
    repository's history, reconstructs each commit's source-group diff, takes the
    label from the commit subject, and scores against it. Its output is advisory:
    harvested labels are only as accurate as the original commit author.

25. `--harvest` lives on the eval harness, not on the `cmt` binary. `cmt`'s CLI takes
    positional pathspecs, so adding a subcommand would restructure the shipped
    interface to host a development tool. `examples/eval.rs` is maintained and
    tested but is not installed by `cargo install`.

26. New recipes: `just eval` (replay, offline) and `just eval-live` (calls jev,
    rewrites recordings). `just check` gains `just eval`.

## Consequences

- `README.md` needs three updates: the `--jev` flag in the usage block, exit code `4`
  in the exit-code list, and a note that `--jev` makes a network call and is
  therefore the one mode that is not deterministic.
- The repository has no `CHANGELOG.md`. `--jev` and exit code `4` are user-facing, so
  one is created per the `recording-changes` skill when this ships.
- `ROADMAP.md:16-17` (`Also:`) is closed by clause 10.
- `ROADMAP.md:12-15` — running Rule F mechanically by applying staged tests to the
  parent in a temporary worktree — is **not** superseded. Clause 9 step 3 is a
  probabilistic read of the same rule. The mechanical experiment remains the
  deterministic alternative and stays on the roadmap; the eval will show which is
  worth having.
- Clause 16 touches every module path in `src/main.rs`. It is mechanical but it is
  not small, and it lands before any jev code.
- `evals/recordings/` is committed. It grows with the corpus and is regenerated only
  by `just eval-live`.
- Running `just eval-live` costs money and requires a key. It is never run by CI.

## Positions

- **One Choice over the five reachable types.** *Rejected.* It would be less code and
  would supply a native `confidence`. But agent-commits is an ordered cascade with
  three conditional tiebreakers, and collapsing it into one option list asks the
  model to perform several judgments at once — the failure mode the jaggedness page
  names directly. Spreading probability across five options also depresses
  `confidence` on changes that are unambiguous, which would make the gate in clause
  11 refuse cases it should answer. It also cannot produce `Also:`: a Choice runner-up
  is a relative artifact of one ranking, not a second true type.
- **Hybrid: a Choice for the type plus companion Nouls for the trailers.**
  *Rejected.* It keeps a native `confidence` and mirrors TypeSafe's own skill-
  suggestion cookbook. But it requires a stated rule for the case where the Choice
  and a Noul disagree, and the jaggedness page states that Choice probabilities and
  Noul values are not comparable quantities. Two mechanisms for one decision, with an
  unprincipled reconciliation between them.
- **The whole staged diff as `state`.** *Rejected.* It is what `--agent-prompt`
  already emits (`src/prompt.rs:157-160`) and would take the least code, but it feeds
  lockfile churn and doc prose to a model documented to lose accuracy on unrelated
  context.
- **Diff plus parent context in `state`.** *Rejected for now, explicitly revisitable.*
  Clause 9 step 2 asks `contradicted_stated_contract`, which is a question about the
  parent commit, while clause 5 sends only the diff. This is the known weak point and
  it is deliberate: clause 20's per-predicate breakdown measures exactly how much it
  costs. `src/git.rs:115-121` already resolves `HEAD:<path>`, so adding parent context
  is cheap once there is evidence it pays. Doing it now would mean tuning two
  variables against no baseline.
- **Auto-enable jev whenever `TYPESAFE_API_KEY` is set.** *Rejected.* Best ergonomics
  inside an agent loop, but identical invocations would diverge between a developer's
  machine and CI, and `README.md:3` would stop being true.
- **Soft fallback to the mechanical classifier on failure or low confidence.**
  *Rejected.* It always produces a commit, but the same command would yield different
  types depending on network weather, and a stderr warning is easy to miss inside an
  agent loop. Clause 13 fails loudly instead.
- **Hand-written fixtures only, or a throwaway harvester.** *Rejected.* Hand-written
  cases have the highest signal per case but reach usable volume too slowly for the
  dead-band sweep in clause 20 to mean anything. A throwaway harvester would bootstrap
  the corpus and then be lost. Clause 24 keeps it so jev can be scored against unseen
  history on demand, accepting that those labels are noisier than curated ones.
- **Deciding the number of commits.** *Out of scope; separate decision.* jev does not
  count reliably, so a count cannot be asked for — it has to fall out of clustering
  changed files in code, driven by pairwise cohesion questions. That is a different
  design with a different failure mode.
- **Deciding which hunks belong in each commit.** *Out of scope; separate decision.*
  `cmt` stages with `git add -A` (`src/git.rs:47-49`) and commits with
  `git commit -- <paths>` (`src/git.rs:127-132`). File granularity is end to end;
  hunk assignment requires synthesizing patches through `git apply --cached`, which
  is a new staging mechanism rather than a new classifier.

## References

- `https://docs.typesafe.ai/api` — the `POST /v1/systemone` request and response shapes
- `https://docs.typesafe.ai/model-jaggedness/jev-1.13` — documented failure modes:
  generation, counting, score interpolation, large state, structural invariants
- `https://docs.typesafe.ai/models` — context, rate, and price limits; version pinning guidance
- `https://docs.typesafe.ai/patterns/fan-out` — batching every question into one request
- `https://docs.typesafe.ai/patterns/composite-scoring` — considered; see Positions, it applies
  to pairwise file cohesion rather than to a categorical type decision
- `https://github.com/phatblat/agent-commits` — the convention encoded in `src/prompt.rs:11-34`
