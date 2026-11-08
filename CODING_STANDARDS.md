# Coding standards

This file lists the rules for Rust code in `crates/`. `AGENTS.md` states the principles and the writing rules. This file does not repeat them. If the two disagree, `AGENTS.md` wins and this file is corrected.

Tooling checks formatting, clippy with warnings as errors, the workspace lints in `Cargo.toml`, the purity lints in the core's `lib.rs` (no printing, exiting or `unwrap`), the dependency licenses (`just deps`), the wire contract (`just proto`), the public API against a release tag (`just semver`) and the tests (`just check`). The rules below are the ones that tooling does not check.

## One definition

- A list that code enumerates - vocabularies, operations, diagnostic rules, reasons, presets, template slots - is one table: a constant slice, an enum with derives, or a data file. Code iterates the table. No second `match` over its members exists elsewhere. A member that needs a property gets a column in the table.
- Adding a member of such a list changes the table and the code that gives the member its behaviour, and nothing else. If adding one needs edits in three files, the table is missing.
- A value that another field determines is not stored. The reader computes it, or a method returns it.
- A type crosses a boundary (CLI, wire, corpus, YAML) in its serde form. No hand-kept schema mirrors a core type.
- A vocabulary word, an extension or an output name is in the rules or in a file under `data/`. Logic code contains none of them. A regex contains shapes (digits, repetition, a separator class from the rules), never words.
- A canonical name is in the canonical section of `data.rs` (decision X25). The reader accepts every canonical name that the writer writes.
- Code reads a YAML or JSON document only with `parse_document` (decision X27).
- Each operation has one entry point: a method on `Rules`. No alias of it exists.

## One mechanism

- One way does each job. A second mechanism for a job that one mechanism already does (a second tokenizer, a second title cut, a style option beside a template) replaces the first in the same change, or it is not added.
- A name is tokenized once per parse. Parsers consume the token stream and its spans. They do not scan the raw string again.
- Every pattern is compiled in one place, `rules::Patterns`, from the rules. A parser file holds no `LazyLock<Regex>`.
- A fact is computed once per call. Validation and planning read the interpretation and share the rendered targets. Neither parses a name again.
- A step that both pipelines need (attaching sidecars, building versions, settling hints) lives in a shared module that both call. The pipelines never call each other. `lexical` and `data` never take or branch on `LibraryType`. A parser reads one name and calls no interpreter, matcher, renderer or validator.
- One dispatch maps an operation name and its input to a result. The corpus runner, the CLI and any server call it. A front end does not match a word, a pattern or an extension itself.

## Earning a place

- Every public item has a caller in a front end or a corpus operation. An item with neither is deleted.
- Every behaviour has a corpus case that fails without it. An overlay vocabulary has a case that changes it. A preset has a case that sets it.
- When a behaviour is replaced, the old code, its cases and its entry in `docs/decisions.md` change in the same commit.
- A dependency is added only to replace code that is hard to get right, with the reason in the commit message. Its license is in `deny.toml`.

## Purity and determinism

- The core crate uses no `std::fs`, `std::env`, `std::time`, `std::net`, `std::process`, threads, randomness or process locale. A `static` holds only a value computed from constants.
- The core returns diagnostics as values. The front ends do the I/O.
- Use `BTreeMap` and `BTreeSet`. A result does not depend on the order of the input entries: collected entries are sorted by a total order before any decision.
- Compare names through the lexical layer: `vocabulary_form` for vocabulary entries and lookups, `fold_case_text` and NFC for name comparison (decisions X2, X3). `str::to_lowercase` produces output text only.
- A span is a pair of UTF-8 byte offsets into the input name. The end is exclusive.

## Outcomes

- A fact that the name can leave open is a `Finding` with its evidence and span. `Option` is for a fact that is present or absent, never for one that can be ambiguous.
- Several candidates for one fact give `Finding::Ambiguous` with the candidates. A choice between candidates is a decision in `docs/decisions.md`, and the code cites it.
- A reason is a variant of a `*Reason` enum with `#[serde(rename_all = "snake_case")]`, never a free-form string.
- Input never causes a panic. `expect` and `unreachable!` state an internal invariant: `expect("every extra kind has an output folder")`.

## Public API

- `lib.rs` lists the public surface with `pub use`. A new module is private by default.
- Every public item has a doc comment that follows `AGENTS.md`.
- A type that is written to JSON, YAML or the wire derives `Serialize` with `rename_all = "snake_case"`. A public enum of facts is `#[non_exhaustive]`.
- A change in naming results gets an entry under "Naming behaviour" in `CHANGELOG.md`. A change in the API, the wire contract or the CLI gets an entry in its own section there.
- Every crate inherits the one version from `[workspace.package]`.

## Tests

- A behaviour starts as a failing corpus case (`corpus/README.md`). A Rust test under `tests/` is a property test, a benchmark or a front-end test, never one naming example.
- A corpus input struct has `deny_unknown_fields`, so a misspelt key fails the case.
- A test name follows the test name rules in `AGENTS.md`.
- Corpus inputs are written for this project and name no other media server or project.
