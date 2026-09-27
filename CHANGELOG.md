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
