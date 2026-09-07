# Roadmap

What's next, in order. This is a living document — check items off in place rather
than filing a separate issue for each one, and delete anything that no longer
applies.

## Inference

- [ ] Infer `Bumps:` from JSON and YAML lockfiles (`package-lock.json`,
      `bun.lock`, `pnpm-lock.yaml`, `yarn.lock`, `go.sum`); today only TOML
      `[[package]]` lockfiles (`Cargo.lock`, `uv.lock`, `poetry.lock`) are read.
- [ ] Run Rule F mechanically: apply only the staged test files to the parent
      commit in a temporary worktree and run them; red means `fix`, green means
      `refactor`. It is the one judgment the convention defines as an
      experiment rather than a reading.
- [ ] Emit `Also:` when a change set has a second true type that lost the
      cascade, such as a dependency move inside a `security` commit.

## Events

- [ ] `deploy: <environment> v<version>` — no file convention announces a
      deployment, so it stays `--type deploy --subject "<env> v<version>"`;
      revisit once conventional-docs names one.
