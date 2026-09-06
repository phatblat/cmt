# Repository Guidelines

## Toolchain

`mise.toml` pins every tool; `just` is the only command surface. Prefer adding a
recipe over documenting a raw command, and run tools through the recipes so the
pinned versions are the ones that execute.

## Commands

- `just deps` — install pinned tools and project dependencies
- `just check` — the full gate: formatting, lint, types, tests
- `just test` — tests only
- `just outdated` / `just upgrade` — report, then apply, tool and dependency updates

## Conventions

- Formatting is owned by the formatter. Run `just format`; never hand-format.
- Add a test with every behavior change; `just check` must be green before pushing.
- Commit with the binary itself: `cargo run --` stages every dirty path and makes one agent-commits commit per logical group; pass `--type feat`/`fix` when the change is consumer-observable.
