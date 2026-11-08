# Changelog

This file lists every notable change, for every package that the project publishes. Every package has the same version, so one entry covers all of them. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). While the version is 0.x, a minor version may change naming results (README, "Versioning").

Each version lists its changes in these sections, in this order. A section without changes is left out.

- **Naming behaviour:** changes in the facts that names resolve to. They apply to every package.
- **Library API:** the `nomenclator` crate.
- **Wire contract:** `nomenclator.v1`, which changes only compatibly.
- **CLI:** `nomenclator-cli`.
- **Packages:** packaging, dependencies, platforms and the minimum Rust version.
- **Performance**

## [Unreleased]

### Naming behaviour

First release. It implements the conventions in `docs/decisions.md`, and the corpus pins them:

- Movies (M1-M15): folder ownership, title source, years, release words, brackets, separators, provider tags and precedence, editions, versions, parts, 3D, disc structures, stub files.
- TV (T1-T21): series folders and names, season folders, bare numbers, `SPnn`, dates, absolute numbering, multi-episode files, episode versions, parts and titles, recording channel tags.
- Extras (E1-E11): vocabulary folders, suffixes, exact names, samples, opening/ending videos, ownership and attachment.
- Sidecars and assets (S1-S9): subtitles with language and flags, other sidecars, video stills, artwork, NFO names.
- Ignore (I1-I11): `.ignore` and `.nomedia` in gitignore syntax, nesting, built-in hidden, junk and partial-download exclusions, unreadable rule files.
- Cross-cutting (X1-X8): root-relative inputs, locale-independent case folding, NFC comparison, non-UTF-8 names, years 1900-2099, per-fact ambiguity.
- Rendering, planning and validation (R1-R9) under the default output profile:
  - every diagnostic has a severity (`error`, `warning`, `info`), and the profile's `severity` map can override it or set it to `off` (R7);
  - `folder_mismatch` (title, year, edition, series title, series year, season) and `overlapping_episodes` (R8);
  - `missing_year`, `missing_series_year`, `missing_provider_id`, `missing_episode_title` (R9);
  - extras keep a group subfolder of their extras folder (R1), and the library reads the separators of all extras in one folder the same way (M6);
  - proposals for files that no item claims are marked `unclaimed` (R6).
- Episode titles keep a leading ellipsis, an acronym's final dot and a part number such as `(1)` (T19).
- Provider matching (P1-P6): scoring of caller-supplied candidates.
- A season that is a year follows the `latest_plausible_year` preset (T16, X12). Under a lowered preset, `Season 2050` and `S2050E01` state no season.
- A lone `UHD` in a release name is a 2160p resolution (X19). An explicit height in the same name takes precedence, as it does over `4K`. Version labels and release facts read resolutions with one function.

- `{key-value}` tags with an unknown key are reported in `tags` and no longer reach a title or a version label (X29).
- The built-in rules read episode words of every language pack, such as `Folge` and `Capitulo`, and a season word before an episode word (X30).
- `clearart` and `discart` images have their own artwork roles, and `Openings` and `Endings` folders hold extras of their own kinds (X31).
- `【】` is a bracket pair (X34).
- In a TV library, an episode's disc image and disc folder are versions of their episode (T18). Before, they stayed unclaimed.
- Without declared languages, the rules contain every word of every language pack (X11): localized artwork words such as `locandina`, spelled numbers, articles, ordinal suffixes and language names that only the packs held before, such as `sesong` and `deutsch`.

### Library API

- Reasons are enums: `model::AmbiguityReason` for `Finding::Ambiguous` and the ambiguity reasons of movies, episodes, versions and stacks, `MatchReason` for rankings, and `InputKind` for input diagnostics. Their written names are unchanged.
- `OverlayDiagnostic` and `ProfileDiagnostic` implement `Display`. `model::serde_name` returns the written name of a fact value. The core has no `clap` feature.

- Every operation is a method on `Rules`, and `Rules::builtin()` returns the built-in rules. Parsers: `parse_movie_name`, `parse_series_name`, `parse_season_folder`, `parse_episode_name`, `parse_provider_ids`, `parse_sidecar`, `classify_extra`, `classify_artwork`, `classify_nfo`. Whole libraries: `evaluate_ignore`, `group_parts`, `interpret`, `rank_candidates`, `render_names`, `plan_renames`, `validate`.
- The parser modules are private. The crate root exports their types. `model` contains the shared fact types.
- Templates (R14): `Template` renders facts into names.
- Reporting (R13): `Diagnostic.code` and `Diagnostic.message`, `Validation.summary` (`ValidationSummary`, `RuleCount`), `OutputProfile.suppressions` (`Suppression`) and `RenamePlan.removals` (`ProposedRemoval`) for release junk.
- Coverage (R12): `OutputProfile.required_assets` (`AssetItemKind`, `AssetRole`) and `OutputProfile.wanted_subtitles` (`WantedSubtitle`); the `missing_asset` and `missing_subtitle` rules.
- Duplicates (R11): the `duplicate_work` and `duplicate_episode` rules. `episode_not_in_reference` is renamed `unexpected_episode`.
- Alternate titles (M16): `MovieName.alternate_titles` (`AlternateTitle`) and `Movie.alternate_titles` from `AKA`, ` / ` and a trailing parenthesised title. A parenthesised original title is no longer part of the title.
- Localized artwork words (X11): the `de`, `fr`, `es`, `it`, `pt` and `nl` language packs add artwork role words.
- Release junk (X22): `Rules::is_release_junk`, `Interpretation.release_junk`, the `release_junk` diagnostic rule and four overlay vocabularies. Screenshot and bitrate images are no longer artwork.
- `parse_document` and `DocumentError` read a YAML or JSON document in the toolkit's dialect (X27). The YAML parser is `serde-saphyr`, which is written in safe Rust. `serde_yaml_ng` and the unmaintained `unsafe-libyaml` are no longer dependencies. An overlay that repeats a key is a syntax error.
- Release vocabularies (X26): the overlay vocabularies `release_words`, `audio_channel_words` and `release_site_tags` change what `parse_release` reads.
- `Rules::is_no_media_marker` tells a caller that scans folders which files are no-media markers (X14). The comparison ignores case.
- Library type suggestion (X21): `Rules::suggest_library_type` returns a `LibraryTypeSuggestion` with the videos as evidence.
- Category folders (R10): `Movie.collection`; planned paths keep the category folders above movies and series, so a plan no longer moves items to the library root.
- Fansub grammar (X20): `EpisodeName.special` (`Special`, `SpecialKind`), `release_version` and `is_final`; `PresentationKind::Special`; `Episode.special`; evidence rule `special_marker`. `S2 - 03` gives season 2, episode 3.
- Release facts (X19): `parse_release` reads resolution, source, codecs, bit depth, HDR, audio, language markers, release group, checksum, revision, streaming service and container, each with its span (`ReleaseFacts`, `ReleaseMarker`). Evidence rule `release_word`.
- Reference metadata (X18): `validate_with` compares an interpretation with `ReferenceMetadata` (movie years, episode lists) and reports `missing_episode`, `unexpected_episode` and `reference_year_mismatch`. `Validation.input_diagnostics` lists unused reference entries.
- Numbering and episode orders (X17): `CallerInputs.series` (`SeriesInputs`) holds a series' `EpisodeOrder` and its absolute and scene maps. Each episode records its `order` and `mapped_from`.
- Overrides (X16): `CallerInputs.overrides` merges videos into the versions of one item, splits videos into separate items, and sets an extra's kind or owner.
- Hints (T20): `interpret_with` takes `CallerInputs`. A `Hint` per path sets titles, years, provider IDs, series folders, seasons and episodes. Items record their `hints` (`AppliedHint`, `HintOutcome`). `Interpretation.input_diagnostics` lists the hints that no item took.
- Rules diff (X15): `diff_rules` and `diff_interpretations` list the per-path facts that differ between two rule sets.
- Built-in ignore groups (X13): an overlay can disable `hidden_entries`, `system_junk`, `partial_downloads` or `generated_metadata`. Ignore dialects (X14): `IgnoreFile.dialect` (`IgnoreDialect::Gitignore`, `IgnoreDialect::Glob`). The `no_media_markers` vocabulary lists the marker names.
- Presets (X12): an overlay's `presets` section selects catalogued alternatives for `latest_plausible_year`, `date_order`, `bare_numbers`, `ignore_nesting` and `samples`. `ExclusionReason::Sample` marks samples that a preset excludes.
- Language packs (X11): `Rules::for_languages` limits the rules to the shared base and the packs of the declared languages. The packs also add words (native language names, spelled numbers, ordinals). `Rules::language_packs` lists the packs. `RulesIdentity.languages` records the declared languages. Overlays can change `leading_articles`, `number_words` and `ordinal_suffixes`.
- Overlays (X10): `Overlay::parse` reads a YAML or JSON overlay. `Rules::from_overlays` applies several overlays together. It reports each problem as an `OverlayDiagnostic` with the line that caused it: conflicting changes, duplicate names, malformed vocabularies, duplicate entries, reserved tokens, and entries that collide with another word class. `RulesIdentity.overlays` lists each applied overlay with a digest of its changes.
- `Rules::run` runs an operation by name with its input and result as JSON values, and `Rules::operations` lists the names. The corpus, the CLI and the server share this dispatch. An input field that the operation does not read is an `OperationError::InvalidInput`.
- `Rules`: one value that holds every input vocabulary. `Rules::ignore_rules` compiles rule files together with the built-in exclusions. `Interpretation`, `RenamePlan`, `Validation` and `CandidateRanking` contain the `RulesIdentity` that they were produced under (X9).
- Overlays change every table that the reader compares a name against (X28): 19 new vocabularies, among them `part_words`, `subtitle_folders`, `format_3d_words` and `alternate_title_words`.
- An overlay that removes an entry that the built-in rules do not contain is rejected with `OverlayDiagnosticReason::IneffectiveRemoval` (X28).
- An overlay can remove the codec words `x264`, `x265`, `h264`, `h265`, `mpeg2`, `mpeg4`, `hdr10` and `hdr10+` from `technical_words`. These words are no longer `reserved_token`s.
- `OutputProfile.reserved_names` lists names that a path component must not use (R5).
- `Format3d` implements `Deserialize` and `Ord`. `SpecialKind` implements `Deserialize`.
- The overlay vocabulary `tags` replaces `provider_keys` and gives each key a typed fact (X29). Results have a `tags` map.
- Overlay vocabularies `episode_words`, `special_prefixes`, `animation_special_words`, `episode_tokens`, `separators` and `bracket_pairs` (X30, X34), and the preset `earliest_plausible_year` (X12).
- `ArtworkRole::ClearArt`, `ArtworkRole::DiscArt`, `AssetRole::ClearArt`, `AssetRole::DiscArt`, `ExtraKind::Opening` and `ExtraKind::Ending` (X31). The public enums of facts are `#[non_exhaustive]`.
- A user overlay overrides a language pack, and `replace` sets a vocabulary (X32).
- `OutputProfile.templates`, `TemplateSlot` and `Rules::check_profile` (X33).

### Wire contract

- Contract `nomenclator.v1`: batched `ParseNames`, `GroupParts`, `EvaluateIgnore`, `Interpret`, `RankCandidates`, `RenderNames`, `PlanRenames` and `Validate`, plus `GetVersion` (`rules_version`, `implementation_version`). Each result returns the caller handle of its query.
- `Diagnostic.severity`, `OutputProfile.severity` overrides, the `folder_mismatch`, `overlapping_episodes` and `missing_*` rules, `RenameProposal.unclaimed` and `ExtraFacts.group`.
- `nomenclator-server`: a stateless server that listens on `--tcp` or `--unix`.
- `Diagnostic.code`, `Diagnostic.message`, `ValidateResult.summary`, `PlanRenamesResult.removals` and `OutputProfile.suppressions` (R13).
- `OutputProfile.required_assets`, `OutputProfile.wanted_subtitles`, `AssetRole`, `WantedSubtitle`, `DIAGNOSTIC_RULE_MISSING_ASSET` and `DIAGNOSTIC_RULE_MISSING_SUBTITLE` (R12).
- `DIAGNOSTIC_RULE_DUPLICATE_WORK` and `DIAGNOSTIC_RULE_DUPLICATE_EPISODE`; `UNEXPECTED_EPISODE` replaces `EPISODE_NOT_IN_REFERENCE` (R11).
- `AlternateTitle`, `MovieName.alternate_titles` and `Movie.alternate_titles` (M16).
- The `release_junk` query of `ParseNames`, `InterpretResult.release_junk` and `DIAGNOSTIC_RULE_RELEASE_JUNK` (X22).
- The `SuggestLibraryType` call (X21).
- `Movie.collection` (R10).
- `Special`, `SpecialKind`, `EpisodeName.release_version`, `EpisodeName.is_final`, `Episode.special`, `PRESENTATION_KIND_SPECIAL` and `RULE_SPECIAL_MARKER` (X20).
- The `release` query of `ParseNames` and its `ReleaseFacts` result (X19); `RULE_RELEASE_WORD`.
- `LibraryTreeQuery.reference` (`ReferenceMetadata`), which `Validate` reads; the `missing_episode`, `unexpected_episode` and `reference_year_mismatch` rules; `ValidateResult.input_diagnostics` (X18).
- `CallerInputs.series` (`SeriesInputs`, `AbsoluteRange`, `SceneMapping`), `Hint.episode_order`, `Episode.order` and `Episode.mapped_from` (X17).
- `CallerInputs.overrides` (`Overrides`, `ExtraOverride`), X16.
- `LibraryTreeQuery.inputs` (`CallerInputs`, `Hint`); `hints` on `Movie`, `Series` and `Episode`; `InterpretResult.input_diagnostics` (T20).
- `IgnoreFile.dialect` (`IGNORE_DIALECT_GITIGNORE`, `IGNORE_DIALECT_GLOB`).
- `EXCLUSION_REASON_SAMPLE` for samples that a preset excludes (X12). Overlay documents contain the presets.
- `RulesSelection.languages` and `RulesIdentity.languages`. A request can declare the languages of its library (X11).
- `RulesSelection rules` on `ParseNames`, `EvaluateIgnore`, `Interpret`, `RankCandidates`, `PlanRenames` and `Validate` requests. The field holds overlay documents that apply to that call. If an overlay is rejected, the call fails with `INVALID_ARGUMENT` and lists the diagnostics. `RulesIdentity.overlays` lists the applied overlays.
- `RulesIdentity`: every response, including `GetVersion`, states the rules it was answered under in `rules`.
- `GroupPartsRequest.rules` selects the rules of a `GroupParts` call.
- `OutputProfile.reserved_names` and `OutputProfile.templates`.
- `Movie.title_source` and `Movie.year_source` are removed, and their field numbers are reserved.
- `ExtraQuery.ignore_files`: the context entries that the rule files exclude are not read.
- Every call runs its queries through the library's one operation dispatch. A query field that the operation does not read fails the call with `INVALID_ARGUMENT`.
- `tags` on `MovieName`, `SeriesName`, `EpisodeName`, `Movie`, `Series` and `Episode`.
- New values of `ExtraKind`, `ArtworkRole` and `AssetRole`.
- A request whose output profile does not read back fails with `INVALID_ARGUMENT` (X33).

### CLI

- `nomenclator run <operation> [FILE]` runs any library operation with its input fields from a YAML or JSON file or standard input.
- `nomenclator parse`, `interpret` and `plan` read names and real folders (with `.ignore`/`.nomedia`) and print a table or `--format json` output. They do not rename any file.
- `plan --target-filesystem posix` relaxes the portable character rules. `plan` marks proposals for unclaimed files.
- `nomenclator validate` prints errors and warnings, and `--all` adds info diagnostics. It exits 1 if a diagnostic reaches the `--fail-on` severity (`error` by default).
- Every command exits 2 if it cannot run.
- `validate` prints each diagnostic's code and a summary line; `plan` prints proposed removals (R13).
- `nomenclator suggest <dir>` prints the suggested library type (X21).
- `nomenclator release <name>` prints the release facts of a name (X19).
- Every command accepts `--inputs <file>` (hints, overrides, series numbering). `validate` accepts `--reference <file>` (movie years, episode lists).
- `nomenclator diff <dir> --library ... --rules builtin|FILE --rules builtin|FILE` prints the facts that differ between two rule sets. It exits 1 if any fact differs.
- `--rule-file NAME[:DIALECT]` is repeatable and names an additional rule file to read, in gitignore syntax or `glob`.
- Every command accepts `--language <tag>`. The option is repeatable and declares the library's languages (X11).
- Every command accepts `--overlay <file>`. The option is repeatable, and the overlays are applied together to the built-in rules. If an overlay is rejected, the command prints the file, line and reason, and exits 2.
- The commands read `.ignore` and every no-media marker of the rules in use, in any letter case. A marker that an overlay adds excludes its folder. `diff` reads the markers of both rule sets.

### Packages

- The minimum Rust version is 1.89, because `serde-saphyr` requires it.

### Performance

- Interpreting and planning take time in proportion to the library size. In release mode, a library of 20,000 movies (130,000 paths) interprets in 0.76 s and plans in 0.41 s.
