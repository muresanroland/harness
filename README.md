# Harness

Drives a beads epic through a fixed multi-agent pipeline, from tickets to open pull requests, with every agent visible in herdr.

## Install

Download `harness-<target>` from the [latest GitHub release](../../releases/latest) (`aarch64-apple-darwin` or `x86_64-unknown-linux-gnu`), make it executable and put it on your PATH as `harness`. Or build from source:

```bash
cargo install --path .
```

`harness --version` prints `v1.0.0` for a release build and `v1.0.0-dev` for a local one.

### Releasing

Cargo.toml's `version` is the source of truth; pushing the matching `vX.Y.Z` tag builds both binaries and creates the release. Feature tickets bump minor, fixes bump patch.

## Build

See the build and test commands in [CLAUDE.md](CLAUDE.md).
