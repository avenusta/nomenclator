# nomenclator

A reusable media naming toolkit: names and limited folder context in, structured naming facts out. Callers keep scanning, probing, their catalog and their database.

## Commands

- `just check` - format, clippy with warnings as errors, tests. The gate before review.
- `just progress [file]` - list pending corpus cases that now pass; with a filter such as `movies/names`, also list that file's failures.
- `just promote` - remove the pending mark from corpus cases that now pass.
- `just bench` - interpret and plan synthetic libraries of 1k to 100k entries in release mode; writes `target/benchmarks.json`.
- `just release-benchmark` - compare `parse_release` with the release-name sets in `references/release-names/`.
- `just proto` - lint the gRPC contract and check it for breaking changes against `main` (needs buf).
- `just semver <tag>` - check the core crate's public API against a release tag (needs cargo-semver-checks).
- `just deps` - check dependency licenses and advisories (needs cargo-deny).
- Toolchain is pinned in `rust-toolchain.toml`; mise reads it too.

## Navigation

- `crates/nomenclator/` - the core library. `src/lib.rs` lists the modules; `src/data.rs` holds the canonical vocabularies.
- `crates/nomenclator/tests/corpus.rs` - runs every case in `corpus/`.
- `crates/nomenclator-cli/` - command-line front end.
- `crates/nomenclator-grpc/` - gRPC server and wire types; `proto/` holds the contract.
- `corpus/` - behaviour cases, one folder per module. Format in `corpus/README.md`. A case marked `pending: true` describes behaviour not built yet.
- `docs/decisions.md` - settles where naming conventions disagree. A case that contradicts it is rewritten to match; record new decisions there.
- `docs/release-facts.md` - the release-word facts the lexical layer recognises.
- `CODING_STANDARDS.md` - rules for Rust code that tooling does not check. Read before changing code in `crates/`.
- `CHANGELOG.md`, `RELEASING.md` - version history and the release procedure.
- `references/` (gitignored, see `references/SOURCES.md`) - raw source tests and `reports/decision-catalogue.md`, the backlog of agreed and contested decisions.

## Architecture

The crate consists of independently usable modules (parsers, interpreter, matcher, validator, renderer/planner) over a shared lexical core and data tables. Evidence is preserved as spans before any decision. The interpreter runs in explicit passes, never as a first-match resolver chain. Outcomes stay distinct: resolved, unknown, ambiguous, invalid input, incomplete, excluded. The crate documentation in `crates/nomenclator/src/lib.rs` lists the modules. Build a feature as a vertical slice: corpus cases, then code in every module that the feature needs.

## Principles

- **YAGNI.** Don't build it until you need it.
- **DRY.** Extract patterns after the second duplication, not before.
- **Fail fast.** Explicit errors beat silent failures.
- **Simple first.** Write the obvious solution, optimize only if needed.
- **Delete aggressively.** Less code = fewer bugs.
- **Semantic naming.** Name variables, parameters and API endpoints with verbose, self-documenting names that optimize for comprehension by humans and LLMs, not brevity (`wait_until_obs_is_saved=true`, not `wait=true`).
- Corpus files, case names and code never name other media servers or projects.
