# nomenclator

A reusable media naming toolkit: names and limited folder context in, structured naming facts out. Callers keep scanning, probing, their catalog and their database.

## Principles

- **Pure.** No filesystem, network, clock, process locale or global state. Inputs are root-relative path components; case folding and Unicode handling are explicit and locale-independent, so results are identical on every OS.
- **Library type is an input.** Movies and TV only. Each has its own pipeline over a shared lexical layer (years, provider-ID tags, release words, extras vocabulary, subtitle flags, ignore rules). The lexical layer never branches on library type, and the pipelines never call each other.
- **Explicit ambiguity.** Conflicting or insufficient evidence returns `ambiguous` or `unknown` with reasons. A result never silently guesses.
- **Simple first.** The most widespread conventions, simplified, are the baseline. A rarer convention is added only when it earns its place. Extension happens through data (language vocabularies, external ignore rules), never through pluggable code.
- **Deterministic.** The same inputs and rules version give the same result, regardless of enumeration order.
- **One schema, read broadly.** Each rule, vocabulary, format and default is defined once. The writer writes one canonical form of each fact, and the reader accepts that form because both use the same definition, together with the other spellings it reads. A canonical name is in the `data.rs` canonical section, and an overlay cannot remove it. Front ends (CLI, gRPC, tests) call the core and do not re-implement its rules.
- **Provider identification is generic.** It scores caller-supplied candidates and never knows TMDB, IMDb or any other provider's API.

## Architecture

The crate consists of independently usable modules (parsers, interpreter, matcher, validator, renderer/planner) over a shared lexical core and data tables. Evidence is preserved as spans before any decision. The interpreter runs in explicit passes, never as a first-match resolver chain. Outcomes stay distinct: resolved, unknown, ambiguous, invalid input, incomplete, excluded. The crate documentation in `crates/nomenclator/src/lib.rs` lists the modules. Build a feature as a vertical slice: corpus cases, then code in every module that the feature needs.

## Working

- `CODING_STANDARDS.md` lists the rules for Rust code that tooling does not check. Read it before you change code in `crates/`.
- Test-driven. Every behaviour starts as a failing case in `corpus/` (format in `corpus/README.md`), run by `cargo test`.
- `docs/decisions.md` settles where conventions disagree; a case that contradicts it is rewritten to match. Decide new disagreements yourself when one answer is clearly right, and record them there. Put only real trade-offs to the owner, with what each established convention does and an option of our own where it beats them.
- A case marked `pending: true` describes behaviour not built yet. `just check` fails on any other failing case and lists pending cases that now pass, so their mark can be removed.
- Corpus files, case names and code never name other media servers or projects.
- `references/` (gitignored, see `references/SOURCES.md`) holds the raw source tests, plus `reports/decision-catalogue.md`, the backlog of agreed and contested decisions.
- `just check` is the gate before review: formatting, clippy with warnings as errors, tests. The toolchain is pinned in `rust-toolchain.toml`; mise reads it too.

## Writing

These rules apply to all text in the repository: doc comments, inline comments, test names, CLI
help text, log and error messages, the README, commit messages and PR descriptions.

Use direct, literal language. State the action, the fact or the condition. Do not use metaphor,
personification, decorative phrasing or rhetorical contrast.

### Sentences

- Put the subject first and the verb next. Write one fact per sentence.
- Name the real actor: "the function returns", "the loader fails", "the edit is skipped". Do not
  give actions to things that do not act. A table does not "name" a hash, a module does not
  "reach" a bin, and a diagnostic does not "say" something.
- State a condition with "if" or "when" and an explicit subject: "Returns `None` if no archive
  contains the chunk."
- State an outcome with a standard verb: returns, fails, skips, reads, writes, resolves, falls
  back to, defaults to.
- Use the established technical term: index, cache, resolve, fallback, missing, stale, lower,
  validate, dry-run. Do not describe around a term that exists.
- Keep "that" and "which" in relative clauses. Write "the first archive that contains the chunk",
  not "the first archive holding it" or "an entry no chunk declares".
- End a sentence with a specific predicate. Do not end with "is a problem", "says nothing",
  "names nothing" or "is as it was". Say what the code does in that case.
- Do not use literary or idiomatic phrases: "for want of", "came to", "the one that remains",
  "as the game has it".

| Do not write | Write |
| --- | --- |
| The chunks `module` edits, each with its edits. An entry no chunk declares is a problem. | Lowers the `module` representation into a deterministic program. |
| Opens the table of game paths of `store`. A cache without it names nothing. | Opens the `game` table of `store`. If the table is not installed, returns a `WadPaths` that resolves no hash. |
| The game's copy of `chunk`: the one in the first archive that holds it. | Reads `chunk` from the first archive that contains it. |
| Logs what an application came to. | Logs a summary of `outcome`. |
| No edit applied to X. It is as the game has it. | No edit was applied to X. It is unchanged. |
| Whether a property edit was skipped for want of a type. | `true` if any property edit was skipped with the `untypable` reason. |

### Doc comments

- Function: the first sentence starts with a third-person present-tense verb and says what the
  function does: "Returns", "Reads", "Lowers". Each condition, fallback and failure follows as
  its own sentence: "Returns `None` if ...", "Fails if ...".
- Type, field or constant: a noun phrase that names what it is. Add a sentence only for a
  constraint, a unit or an invariant that the name does not show.
- Module: the first sentence says what the module does or provides.
- Do not restate the signature. Do not describe how the code changed over time.

### Inline comments

Write an inline comment to give the reason for the code, or a fact the code depends on. Put the
identifiers it refers to in backticks.

### Test names

Name a test `subject_behavior` or `subject_behavior_condition` in snake case, using the
identifiers from the code: `chunk_reads_from_first_archive_containing_it`,
`read_inside_rejects_path_outside_dir`.

Corpus case names are sentences that state the behaviour (`corpus/README.md`); these rules
apply to their wording.

### Messages and help text

- A log or error message says what happened and to which object, then what the user can do:
  "`overlay.yaml` line 3: unknown vocabulary `edition_word`. Use a vocabulary listed in decision
  X10".
- Help text for an argument says what the value is, then its default.

### Characters

- Do not use em dashes or en dashes. Use a hyphen with spaces (` - `) or restructure the
  sentence. A numeric range uses a bare hyphen (`1-5`).
- Do not use the section sign. Write "section 6".
