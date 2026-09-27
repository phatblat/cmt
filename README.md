# cmt

Deterministically commits dirty files as one agent-commits conventional commit
per logical group, inferred from the paths. With `--jev`, the source commit's
type is instead decided by TypeSafe's jev model; that path is opt-in and is not
deterministic.

## Usage

```text
cmt [OPTIONS] [PATHS]...
  PATHS                  git pathspecs to stage and commit; default: everything dirty

Intent flags — any of these switches to single-commit mode:
  --type <TYPE>          commit type; parsed via CommitType::from_str
  --scope <SCOPE>        scope; "" removes it
  --subject <TEXT>       subject
  --body <TEXT>          body paragraph (required for perf)
  --breaking             append ! to the header
  --also <TYPE>          Also: trailer (CommitType::from_str)
  --reverts <SHA>        Reverts: trailer
  --advisory <ID>        Advisory: trailer (repeatable)

Group-targeted:
  --bumps <SPEC>         Bumps: trailer (repeatable); attaches to the deps group

Provenance — applied to every commit:
  --generator <SPEC>     Generator: trailer
  --assisted-by <SPEC>   Assisted-by: trailer
  --reviewed-by <NAME>   Reviewed-by: trailer (repeatable)
  --tested-by <NAME>     Tested-by: trailer (repeatable)

  --dry-run              stage and print every message; commit nothing

  --jev                  ask jev to decide the source commit's intent;
                         needs TYPESAFE_API_KEY. Conflicts with --type,
                         --also, --breaking, --single, --agent-prompt

  -h, --help             print help
  -V, --version          print version
```

Exit codes:

- `0` — every group committed, or `--dry-run`
- `1` — git failure; earlier groups may already be committed
- `2` — validation failed; nothing committed
- `3` — nothing to commit
- `4` — `--jev` could not decide: no key, no source commit, transport failure,
  a diff over budget, or an answer too close to call. Nothing committed.

Messages follow the [agent-commits](https://github.com/phatblat/agent-commits)
convention plus the [conventional-docs](https://github.com/phatblat/conventional-docs)
lifecycle events (`decision`, `plan`, `todo`, `release`, `deploy`).
`feat`, `fix`, `perf`, and `security` are never inferred — they describe intent,
which a path scan cannot read — so pass `--type`, which switches to a single
commit over the whole selection.

### How a path is categorised

Top-down, first match wins.

| # | Category | Predicate |
|---|---|---|
| 1 | todo | path is `TODO.md` |
| 2 | plan | path is `PLAN.md` and it was added or deleted |
| 3 | decision | added `docs/decisions/YYYY-MM-DD-slug.md` |
| 4 | ignore | basename ends with `ignore`, or is `.gitattributes` |
| 5 | ci | under `.github/workflows/` or `.circleci/`; `.gitlab-ci.yml`, `.travis.yml`, `azure-pipelines.yml`, `Jenkinsfile` |
| 6 | lock | `Cargo.lock`, `package-lock.json`, `bun.lock`, `bun.lockb`, `yarn.lock`, `pnpm-lock.yaml`, `uv.lock`, `poetry.lock`, `go.sum` |
| 7 | manifest | `Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod` |
| 8 | test | under `tests/`, `test/`, `__tests__/`, `spec/`; `*_test.*`, `*.test.*`, `*.spec.*`, `test_*.py`, `*Tests.swift` |
| 9 | docs | under `docs/`; `.md`, `.mdx`, `.rst`, `.txt` |
| 10 | build | `mise.toml`, `.tool-versions`, `justfile`, `Makefile`, `build.rs`, `.editorconfig`, `rustfmt.toml`, `clippy.toml`, `commitlint.config.*`, `Dockerfile*`, `*.dockerfile`, `tsconfig*`, `.prettierrc*`, `.husky/` |
| 11 | source | anything else |

### How groups become commits

One commit per group, in this order, so the tree builds after each one.

| Order | Type | Members |
|---|---|---|
| 1 | `decision: propose <id>` | one commit per added decision record |
| 2 | `plan: start <id>` / `plan: done <id>` | one commit per `PLAN.md` add/delete; `<id>` is the first decision link inside it |
| 3 | `ignore` | every ignore path |
| 4 | `build` | every build path, plus manifests when no lockfile moved |
| 5 | `deps` | every lockfile, plus manifests, when a lockfile moved (needs `--bumps`) |
| 6 | `refactor[(scope)]` | every source path, plus tests when source changed; scope is the shared directory under `src/`, `crates/`, ... |
| 7 | `test` | every test path, when no source changed |
| 8 | `docs` | every docs path |
| 9 | `ci` | every ci path |
| 10 | `todo: sync` / `todo: clear` | one commit per `TODO.md` change |

Subjects are mechanical: `add|update|remove|rename <path>` for one file,
`<verb> <N> files in <dir>` for several.

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
refusal, not a guess. `TYPESAFE_ENDPOINT` overrides the API endpoint, for
testing against a stub.

`just eval` scores that cascade against a labelled corpus in `evals/`, offline,
and runs as part of `just check`. `just eval-live` refreshes the recordings.

## Development

```bash
just deps     # install pinned tools and dependencies
just check    # formatting, lint, types, tests
just eval     # score the cascade against recorded jev responses, offline
just harvest  # propose new fixture candidates from a repo's history
```

`just --list` shows every recipe.

## License

MIT © Ben Chatelain
