# Coding standards

This file lists the rules for Rust code in `crates/`. `AGENTS.md` states the principles that these rules apply and the writing rules for comments, names and messages. This file does not repeat them. If this file and `AGENTS.md` disagree, `AGENTS.md` wins, and this file is corrected.

Tooling enforces these rules:

- `just check` runs formatting (`cargo fmt`), the lints and the tests. The lints are clippy with warnings as errors, the workspace lints in `Cargo.toml` (`unsafe_code`, `missing_docs`, `must_use_candidate`, `doc_markdown` and others), and the purity lints of the core crate in its `lib.rs` (no printing, exiting or `unwrap`).
- `just deps` runs `cargo deny check` against `deny.toml`: permissive licenses only, no security advisories, crates.io as the only source.
- `just proto` lints the wire contract with buf (`buf.yaml`) and checks it for breaking changes against `main`.
- `just semver <tag>` checks the core crate's public API for breaking changes against a release tag.

CI runs all four. The rules below are the ones that tooling does not check.

## Purity

- The `nomenclator` crate does not use `std::fs`, `std::env`, `std::time`, `std::net`, `std::process`, threads, randomness or the process locale.
- The core has no mutable global state. A `static` or `LazyLock` holds only a value that is computed from constants, such as a compiled `Regex` or a built-in table.
- The core does not print or log. It returns diagnostics as values.
- The front ends (`nomenclator-cli`, `nomenclator-grpc`) do the I/O. They read files, scan folders and write output, then call the core.

## Determinism

- Use `BTreeMap` and `BTreeSet`. Do not use `HashMap` or `HashSet` in the core, because their iteration order varies between runs.
- A result does not depend on the order of the input entries. If the code collects entries, it sorts them by a total order before it decides anything.
- Fold case with `lexical::fold_case` or `lexical::fold_case_text` when two names are compared. Do not compare with `to_lowercase` or `to_uppercase`. `str::to_lowercase` is allowed only to produce output text, such as an extension under `ExtensionCase::Lower`.
- Compare names in NFC through the lexical layer (decision X3).
- A span is a pair of UTF-8 byte offsets into the input name. The end is exclusive.

## Layering

- `lexical` and `data` never take or branch on `LibraryType`.
- The movie pipeline (`interpret/movies.rs`) and the TV pipeline (`interpret/tv.rs`) do not call each other. Code that both need goes into the shared lexical layer or a parser.
- A parser reads one name and does not call the interpreter, the matcher, the renderer or the validator.
- The front ends call the core for every rule. A front end does not match a vocabulary word, a pattern or an extension itself.
- An operation is a method on `Rules`. The free function of the same name calls it with `Rules::builtin()`.

## Data

- A vocabulary word, an extension or an output name is defined once, in `data.rs` or in a file under `crates/nomenclator/data/`. Logic code does not contain vocabulary as string literals.
- A canonical name is in the canonical section of `data.rs` (decision X25). The reader accepts every canonical name that the writer writes.
- Entries in `data.rs` are lowercase.
- A language's words go in its pack under `data/languages/`, not in the shared base.
- Code reads a YAML or JSON document only with `parse_document`, so every document has the same dialect (decision X27).

## Outcomes and errors

- A fact that can be ambiguous is a `Finding`, with its evidence and span. Do not use `Option` for such a fact, because `None` cannot say why the fact is missing.
- A reason is a variant of a `*Reason` enum with `#[serde(rename_all = "snake_case")]`. It is never a free-form string. Message text comes from `data/messages/`.
- Input never causes a panic. Invalid input returns an `invalid`, `unknown` or `ambiguous` outcome, or a diagnostic.
- `expect` and `unreachable!` are allowed only for an internal invariant, and the message states the invariant: `expect("every extra kind has an output folder")`. Library code outside tests does not call `unwrap`.

## Public API

- `lib.rs` lists the public surface with `pub use`. A new module is private or `pub(crate)` by default.
- Every public item has a doc comment that follows the doc comment rules in `AGENTS.md`.
- A type that is written to JSON, YAML or the wire derives `Serialize` with `rename_all = "snake_case"`.
- The gRPC contract `nomenclator.v1` changes only compatibly: add fields and never renumber or reuse them. The wire has one field per library field.
- A change in naming results gets an entry under "Naming behaviour" in `CHANGELOG.md`, the one changelog of every package. A change in the API, the wire contract or the CLI gets an entry in its own section there.
- Every crate inherits the one version from `[workspace.package]`. A crate does not set its own version.

## Tests

- A behaviour starts as a failing corpus case (`corpus/README.md`). A Rust test under `tests/` is for a property, a benchmark or a front end, not for one naming example.
- A test name follows the test name rules in `AGENTS.md`.
- Corpus inputs are written for this project. They are not copied from the tests of another project.
- Code, tests and corpus files do not name other media servers or projects.

## Dependencies

- The core crate keeps few dependencies. Add a dependency only if it replaces code that is hard to get right, and give the reason in the commit message.
- A dependency is MIT, Apache-2.0 or a similarly permissive license, because the crates are MIT. `deny.toml` lists the allowed licenses. Add a license there only if it is permissive.
