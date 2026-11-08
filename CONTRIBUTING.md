# Contributing to nomenclator

nomenclator is MIT-licensed. This page describes how to send a change.

## What's welcome

- Naming cases: a file or folder name that nomenclator reads wrong. Open an issue with the naming case template, or a pull request with a failing corpus case.
- Bug fixes in the crates under `crates/`.
- Vocabulary words and language packs under `crates/nomenclator/data/`.
- Documentation fixes.

A bug fix or a corpus case that follows `docs/decisions.md` can go straight to a pull request. Open an issue first if the change adds public API (a crate export, an overlay vocabulary, a CLI command or a wire field), changes a decision, or adds a convention. A maintainer agrees on the shape before you build it.

## Setup

Install Rust with rustup, and `just`. `rust-toolchain.toml` pins the toolchain, and rustup installs it on the first build.

## Coding agents

This repository is built for coding agents. `AGENTS.md` is their guide, and `CODING_STANDARDS.md` holds the code rules. Point your agent at both.

An agent-written pull request meets the same bar as any other. A person who understands the change stands behind it: that person reads the diff, runs it and answers review questions.

## Before you open a pull request

Run the gate from the repository root. It formats, lints and runs every test:

```sh
just check
```

A change in naming behaviour starts as a failing case in `corpus/` (format in `corpus/README.md`). A bug fix comes with a case that fails without the fix.

Depending on what you changed:

- **Naming behaviour.** Add an entry under "Naming behaviour" in `CHANGELOG.md`. If conventions disagree, follow `docs/decisions.md`, or propose a new decision in the pull request.
- **Public API, wire contract or CLI.** Add an entry under "Library API", "Wire contract" or "CLI" in `CHANGELOG.md`.
- **Dependencies.** Run `just deps` (needs `cargo-deny`). Only permissive licenses are allowed.
- **Wire contract.** Run `just proto` (needs `buf`). `nomenclator.v1` changes only compatibly.
- **Corpus inputs.** Write each input yourself. Do not copy names or expectations from the tests of another project. Code, tests and corpus files do not name other media servers or projects.

## Opening the pull request

- Keep it small and focused on one change.
- Say what was wrong and what you changed, and link the issue if there is one.
- Leave **Allow edits by maintainers** on.
- Open it as ready for review.

CI runs `just check` on Linux, macOS and Windows, and checks the dependencies, the wire contract and the public API. On your first pull request, GitHub may hold CI until a maintainer approves the run.

## What happens next

1. A maintainer reads the full diff.
2. The maintainer merges it, asks for changes, or finishes small things on your branch.
3. You do not need to rebase. A maintainer merges `main` into your branch when it falls behind.
4. Pull requests are squash-merged. You stay the author.

## License

There is no CLA and no sign-off. By contributing, you agree that your contribution is licensed under the [MIT License](LICENSE) that covers this repository.

## Security

Do not report a vulnerability in a public issue or pull request. Follow [SECURITY.md](SECURITY.md).
