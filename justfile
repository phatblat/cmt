set ignore-comments
set script-interpreter := ['bash', '-eu']
set unstable

[default]
_default:
    @just --list

#
# configuration group recipes
#

# Install pinned tools and fetch dependencies
[group('configuration')]
deps:
    mise install
    cargo fetch

# Format source, mise config, and the justfile
[group('configuration')]
format:
    cargo fmt
    mise fmt
    just --fmt

# Remove build output
[group('configuration')]
clean:
    cargo clean

# Report tools and dependencies with newer versions available
[group('configuration')]
outdated:
    -mise outdated --local --bump
    -cargo update --dry-run

# Upgrade pinned tools and refresh the lockfile within existing ranges
[group('configuration')]
upgrade:
    mise upgrade --local --bump --yes
    cargo update

#
# build group recipes
#

# Build the project
[group('build')]
build:
    cargo build

# Run the application
[group('build')]
run:
    cargo run

# Build an optimized binary at target/release/cmt
[group('build')]
release:
    cargo build --release

# Build and install cmt into cargo's bin directory ($CARGO_HOME/bin, ~/.cargo/bin by default)
[group('build')]
install:
    cargo +stable install --path . --locked

# Remove the installed cmt binary from cargo's bin directory
[group('build')]
uninstall:
    cargo uninstall cmt

#
# checks group recipes
#

# Verify formatting without writing changes
[group('checks')]
format-check:
    cargo fmt --check
    mise fmt --check
    just --fmt --check

# Lint with clippy, denying warnings
[group('checks')]
lint:
    cargo clippy --all-targets -- -D warnings

# Run every gate: formatting, lint, tests, eval replay
[group('checks')]
check: format-check lint test eval

#
# tests group recipes
#

# Run the test suite
[group('tests')]
test:
    cargo test

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
    cargo run --quiet --example harvest -- {{ repo }} {{ count }}
