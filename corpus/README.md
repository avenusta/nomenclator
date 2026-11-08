# Corpus

The corpus holds language-neutral naming cases. The cases are the test suite and the published spec examples.

Each case is in `corpus/<area>/<topic>.yaml`. `just check` runs all cases.

## Case

```yaml
- name: a file named Title (Year) gives its title and year   # the behaviour that the case pins, in plain words
  pending: true   # optional: behaviour not built yet; a failure is reported as progress, not as an error
  diverges: M3    # optional: a decision that contradicts this expectation; the case is rewritten or removed when the decision is settled
  overlays: [...]  # optional: overlay documents (YAML or JSON strings) applied together to the built-in rules for this case
  languages: [en]  # optional: the library's declared languages (X11); omit it for every language
  operation: parse_movie_name
  input: { name: Northern Passage (2021).mkv, kind: file }
  expect:
    title: Northern Passage
    year: 2021
```

`expect` is a subset. Only the listed fields are compared. Maps are compared key by key. Lists must match exactly and in order.

Every input in a case is written for this project. A case does not copy a name, a path or an expectation from the tests of another project, because those tests have their own licenses. Use well-known real titles, such as the films and series in IMDb's Top 250, with their real years. A placeholder such as `Northern Passage`, `Show` or `Movie` is fine when the title does not matter. A release group or a category folder gets an invented name.

Do not add a case that only restates one entry of a vocabulary, such as one case per video extension. One case per vocabulary and one case for each rule that applies to the entries, such as case-insensitive matching, are enough.

A finding is a field that the library reports as resolved, unknown or ambiguous. Findings have these shorthands:

- a plain value `title: Northern Passage` means `{ status: resolved, value: Northern Passage }`;
- `null` means unknown or absent;
- the full form `{ status: resolved, value: …, evidence: { rule: …, span: [start, end] } }` also asserts the evidence. Spans are UTF-8 byte offsets into the input name, and the end is exclusive.

Paths are root-relative strings separated by `/` (`Show/Season 01/Show S01E02.mkv`). The separator is always `/`, never the separator of the operating system.

## Operations

Each meaning has one field name across all operations. If a case needs a field that is not listed, add the field here first. `Rules::run` runs the operations by name. An input field that an operation does not read fails the case, and a test checks that this table lists exactly the operations of `Rules::run`.

| Operation | Input | Expect |
|---|---|---|
| `parse_movie_name` | `name`, `kind` (`file`/`directory`) | `title`, `year`, `tags`, `edition`, `edition_hint`, `format_3d`, `container`, `trusted`, `alternate_titles` (`[{ title, span }]`, M16) |
| `parse_series_name` | `name_or_path` (a series folder name, or a path whose series folder is read) | `title`, `year`, `season`, `provider_ids` |
| `parse_season_folder` | `name`, `parent_name`, `parent_is_series_folder` | `season`, `is_bare_season_folder`, `provider_ids` |
| `parse_episode_name` | `path`, `kind`, `season_folder_number` | `series_name`, `series_year`, `series_path`, `season`, `episodes`, `first_episode`, `last_episode`, `absolute`, `date`, `episode_title`, `alternate_identity` (`{ title, year }`), `provider_ids`, `ambiguous`, `is_episode`, `presentation_kind` (`episode`/`multi_episode`/`absolute`/`dated`/`special`), `special` (`{ kind (ova/oad/ona/recap), number }`), `release_version`, `is_final` (X20) |
| `suggest_library_type` | `paths` | `library` (`movies`, `tv`, ambiguous or `null`), `episode_videos`, `movie_videos` (X21) |
| `parse_release` | `name`, `kind` | `resolution`, `source`, `remux`, `screener`, `video_codec`, `bit_depth`, `hdr`, `audio_codecs`, `audio_channels`, `audio_languages`, `subtitle_languages`, `release_group`, `checksum`, `revision`, `streaming_service`, `container`. List facts are `[{ value, span }]` (X19) |
| `parse_provider_ids` | `text` | `provider_ids` |
| `classify_extra` | `path`, `kind`, `library`, `context_paths`, `ignore_files` | `extra`, `title`, `owner`, `series_path`, `supplemental_dir` |
| `classify_artwork` | `path`, `video_path` (optional) | `role`, `is_episode_still` |
| `classify_nfo` | `path`, `video_paths` (optional: videos in scope) | `role` (`video`/`movie`/`tvshow`/`season`, `null` when the name matches no role) |
| `parse_sidecar` | `name`, `video_name`, `kind` (`subtitle`/`audio`/`image`) | `matches`, `language`, `flags`, `title` |
| `group_parts` | `paths` | `stacks` (`[{ name, paths, ambiguous, ambiguity_reason }]`) |
| `evaluate_ignore` | `path`, `kind`, `library`, `ignore_files` (`[{ path, contents, dialect }]`, `contents` omitted when unreadable, `dialect` `gitignore` (default) or `glob`, X14) | `excluded`, `exclusion` (`{ path, reason, rule_file, line }`), `unreadable_rule_files`, `rule_diagnostics` (`[{ rule_file, line, reason }]`) |
| `interpret` | `library` (`movies`/`tv`), `paths`, `ignore_files`, `scope_group` (`{ root, folders }`: the files at the root and the root folders to interpret, X23), `inputs` (caller inputs: `{ hints: [{ path, authoritative, title, year, provider_ids, series_folder, season, episodes }], overrides: { merge: [[path, …]], split: [path], extras: [{ path, kind, owner }] }, series: [{ series, order, absolute: [{ season, first, last }], scene: [{ from: [season, episode], to: [season, episode] }] }] }`, T20, X16, X17; hints also take `episode_order`) | `items`, `extras_of`, `video_subtitles`, `excluded_entries`, `undecided_entries`, `invalid_entries`, `unclaimed_files`, `release_junk` (the unclaimed files that are release junk, X22), `rule_diagnostics`, `input_diagnostics` (`[{ input, path, reason, fact }]`, `reason` `unknown_path`, `not_applicable` or `duplicate`) |
| `render_names` | `facts` (`{ library, title, year, provider_ids, edition, version_label, part, container, season, episodes, absolute, date, episode_title, extra, sidecar }`), `profile` | `path`, `blocked_reasons` |
| `plan_renames` | `library`, `paths`, `ignore_files`, `inputs`, `profile` (the paths are interpreted, then planned with the same paths as the existing entries) | `proposals` (`[{ from, to, case_only, requires_temporary_step, moves_with, unclaimed }]`), `blocked` (`[{ from, reasons, to, conflicting_paths }]`), `removals` (`[{ path, reason }]`, R13) |
| `validate` | `library`, `paths`, `ignore_files`, `inputs`, `reference` (`{ movies: [{ path, provider_ids, year }], series: [{ series, episodes: [[season, episode]] }] }`, X18), `profile` (the paths are interpreted, then validated) | `diagnostics` (`[{ rule, severity, paths, reason, proposed_path, code, message }]`), `summary` (`{ errors, warnings, infos, suppressed, rules: [{ rule, code, severity, count }] }`, R13) |
| `end_to_end` | `library`, `paths`, `ignore_files`, `profile` | `interpretation` (the `interpret` result), `plan` (the `plan_renames` result), `validation` (the `validate` result): one library folder through every stage; these cases are in `corpus/end-to-end/` |
| `diff_rules` | `library`, `paths`, `ignore_files`, `a` and `b` (each `{ overlays, languages }`, `{}` for the built-in rules) | `paths` (`[{ path, facts: [{ fact, a, b }] }]`, only facts that differ, X15) |
| `render_template` | `template`, `facts` (field → value) | `path`, `missing_fields`, or `error` (`{ reason, offset }`) (R14) |
| `check_profile` | `profile` | `invalid` (`true` if the profile is not valid), `diagnostics` (`[{ reason, slot, fact, message }]`, X33) |
| `build_rules` | `overlays` (overlay documents, YAML or JSON strings), `languages` (optional) | `identity` (`{ version, languages, overlays: [{ name, version, digest }] }`, `null` when no rules are built), `diagnostics` (`[{ reason, overlay, line, vocabulary, entry, overlays, message }]`) |
| `rank_candidates` | `facts` (`{ kind, title, alternate_titles, year, date, provider_ids, season, episodes }`), `candidates` (`[{ handle, kind, title, alternate_titles, year, date, provider_ids, season, episode }]`) | `outcome`, `selected`, `reason`, `tied`, `candidates` (`[{ handle, score, components, contradictions }]`) |
| `rank_batch` | `queries` (`[{ handle, facts, candidates }]`, as for `rank_candidates`) | `results` (`[{ handle, ranking }]`, sorted by handle) (P9) |
| `search_queries` | `facts` (as for `rank_candidates`) | `queries` (`[{ reason, kind, title, year, provider, id }]`, in the order a caller sends them, P7) |
| `match_facts` | `library`, `paths`, `ignore_files`, `inputs` (the paths are interpreted) | `facts` (the `rank_candidates` facts of each item) |
| `convert_layout` | `library`, `paths`, `ignore_files`, `inputs`, `from_profile`, `to_profile` (the paths are read under `from_profile`, then planned under `to_profile`, R18) | as for `plan_renames` |
| `plan_import` | `library`, `paths` (the download folder), `ignore_files`, `inputs`, `library_paths` (the library's paths), `profile` (R17) | `proposals`, `left_in_place` (`[{ path, reason }]`), `blocked` |
| `is_video_file`, `is_audio_file`, `is_image_file`, `is_subtitle_file`, `is_release_junk` (X22) | `path`, `sibling_paths` | `result`, `unsupported_video`, `unsupported_video_reason` |

### Field meanings

- `kind` (input): `file` or `directory`. `library`: `movies` or `tv`. Paths are root-relative; directories end in `/`.
- `episodes`: every episode that a name covers, ranges expanded. `first_episode` / `last_episode`: the ends of the range, for cases that assert only one end. `last_episode` is `null` when the name contains no range.
- `season`: `0` for specials, `null` when unknown. `date`: `YYYY-MM-DD`.
- `absolute`: absolute episode numbers, counted across seasons (T14). They are never mixed into `episodes`.
- `parent_is_series_folder` (input): `true` if the caller knows that the season folder's parent is a series folder (T9).
- `season_folder_number` (input): the season of the entry's folder, when the caller already knows it.
- `series_path` / `path`: the folder of the series / of an item, `null` when there is no such folder.
- `provider_ids`: map of provider → id (`tmdb`, `imdb`, `tvdb`, `tvmaze`, `anidb`, `anilist`, `anisearch`; other keys as written).
- `extra`: `trailer`, `behind_the_scenes`, `deleted_scene`, `featurette`, `interview`, `scene`, `short`, `clip`, `blooper`, `sample`, `theme_song`, `theme_video`, `other`, `extra`, `opening`, `ending`, or `null`.
- `owner`: the folder whose title an extra belongs to. `supplemental_dir`: the owned extras folder that contains an extra, `null` for an extra identified by its name. `context_paths`: the other files in scope. Ownership is judged only from these files and the path itself.
- `unsupported_video_reason`: `disc_image`, `disc_structure`, `unsupported_container`, or `null`.
- `flags`: ordered subset of `default`, `forced`, `hearing_impaired`.
- `format_3d`: `side_by_side`, `half_side_by_side`, `full_side_by_side`, `top_and_bottom`, `half_top_and_bottom`, `full_top_and_bottom`, `mvc`, `unspecified`.
- `edition`: the raw text of an `{edition-X}` / `[edition-X]` tag. `edition_hint`: the raw edition words after the title (`Extended`, `Directors Cut`), dots shown as spaces. It is a hint, never an edition (M9).
- `ambiguity_reason` (stacks): `missing_part` or `duplicate_part`, `null` when the parts are numbered 1, 2, … or `a`, `b`, … without gaps.
- `role` (artwork): `poster`, `backdrop`, `banner`, `logo`, `thumb`, `clear_art`, `disc_art`, `episode_still`, `other`.
- `items[]`: `{ kind (movie/series/season/episode/extra), title, year, path, paths, series_name, series_year, series_path, season, episodes, first_episode, absolute, date, episode_title, provider_ids, alternate_identity, versions, extras, subtitles, audio, artwork, misplaced_series, nfo, ambiguous, ambiguity_reasons, … }`. A series' `alternate_identity` is `{ title, year }` (`year` is `null` when the text has no year). It is the text that the episodes in its season folders repeat (T2). `versions[]` are `{ paths, label, ambiguity_reason, edition, edition_hint, format_3d, nfo }` (`edition`, `edition_hint` and `format_3d` as for `parse_movie_name`, from the version's name, else its folder's, `null` for TV; `nfo` the NFO named after the version's video or parts, S9) and `extras[]` are `{ paths, extra, title, year }` (an episode's extra takes its title from the text after its episode code, E11). Several paths are ordered parts, and a disc structure is its folder path, which ends in `/`. `subtitles[]`, `audio[]` and `artwork[]` are single files: `{ path, language, flags, title, role }`. An item of kind `extra` is an extra that no item owns. It is `ambiguous` when several items could own it. `order` (episodes): `aired`, `dvd` or `absolute`, `null` if there are no series inputs and no order hint. `mapped_from`: `{ map (absolute/scene), season, numbers }` (X17). `hints` (movies, series, episodes): `[{ path, fact, value, outcome, name_value }]`, one entry per hinted fact with its outcome (`filled`, `agreed`, `contradicted`, `overrode`, T20). `misplaced_series`: an episode-coded video in a season folder of a movie library (M15). `nfo`: the path of the NFO attached to a movie, series, season or episode (S8, S9): for a movie or episode, its primary version's `nfo`, or else (movies only) its folder's generic NFO; `null` when there is no NFO. An attached NFO counts as claimed.
- `alternate_titles` (movie): the other titles of the name that gives the title (M16). `collection` (movie): the folder that contains the movie's folder, or its video if it has no folder, unless that folder is another movie's folder (R10). `path` (item): the folder that owns it, `null` for a video that is its own movie (never a file stem).
- TV items are a flat list: each series, then each of its seasons followed by that season's episodes, then the series' episodes that have no known season. Seasons and episodes carry `series_name` and `series_path`. An episode also carries `series_year`. A series' `path` is its folder (`null` for a series directly at the root). A season's `path` is its season folder (`null` if it has none). An episode's `path` is its disc or per-episode folder. An episode item represents one coverage: a multi-episode file is one item whose `episodes` lists all its episodes, and its videos are `versions`. An item's `path` and `series_path`, and the `extras_of` keys, have no trailing `/`.
- `extras_of`: an owner's extras, keyed by the owner: a movie's or episode's primary video path, or a series' or season's folder. `video_subtitles`: each video's attached subtitles, keyed by video path. A video without subtitles has no key.
- Interpretation outcomes besides `items`: `excluded_entries[]` `{ path, reason, matched_path, rule_file }` (an exclusion rule applies), `undecided_entries[]` `{ path, unreadable_rule_files }` (incomplete scope), `invalid_entries[]` `{ path, reason }` with `reason` `not_root_relative`, `invalid_component` or `duplicate` (invalid input), `unclaimed_files[]` (in-scope files that no item claimed: unrecognized names, orphan sidecars, other files).
- `ignore_files` (input): `[{ path, contents }]` as for `evaluate_ignore`.
- `ambiguous`: `true` when several interpretations compete.
- `excluded`: `null` when an unreadable rule file leaves the answer undecided. `exclusion.path`: the entry or the folder above it that matched; `exclusion.reason`: `hidden_entry`, `system_junk`, `partial_download`, `generated_metadata`, `no_media_marker`, `empty_rule_file`, `rule`, and in interpretations `sample` (the excluded-samples preset, X12).
- `rule_diagnostics[].reason`: `unclosed_bracket`, `unknown_character_class`, `trailing_backslash`, `empty_pattern`, `outside_library_root`; `line` is 1-based, `null` for the whole file.
- `facts` (input): the facts that a name is rendered from. `library` (`movies`/`tv`) selects the layout; `title` is the movie's or series' title, `year` its year; `container` the lowercase video extension. If `extra` (`{ kind, title, year }`, `kind` as `extra`) is set, the renderer renders that extra of the work instead of the work's video. If `sidecar` (`{ language, flags, title, extension }`) is set, the renderer renders a sidecar of the video (or extra) that the other facts name. For TV, `season` and `episodes` (or `date`) identify an episode. An extra with no `season` belongs to the series. An extra with a `season` and no `episodes` belongs to the season.
- `profile` (input): the output profile, every field optional: `movie_provider_order` (default `[tmdb, imdb]`), `series_provider_order` (default `[tvdb, tmdb]`), `target_filesystem` (`portable`, the default, or `posix`), `max_component_bytes` (default 255), `reserved_names` (default none), `severity` (rule → `error`, `warning`, `info` or `off`, R7), `required_assets` (item kind → roles), `wanted_subtitles` (`[{ language, flags }]`, R12) and `suppressions` (`[{ rule, paths }]`, R13). See R1-R5.
- `blocked_reasons` / `reasons`: `missing_title`, `missing_extension`, `missing_season`, `missing_episode_reference`, `absolute_without_season_mapping`, `missing_extra_title`, `reserved_name`, `component_too_long`, and for plans `ambiguous_identity`, `ambiguous_provider_id`, `ambiguous_parts`, `ambiguous_season`, `ambiguous_sidecar_language`, `misplaced_series`, `no_owner`, `disc_structure`, `destination_exists`, `destination_collision`, `destination_differs_only_in_case_or_normalization`, `moves_with_blocked_file` (R3, R5, R6). `path` / `to` is `null` if no name could be rendered.
- `proposals[]` are in execution order. `moves_with`: the video that a part, sidecar or folder file moves with, `null` for the video itself. `case_only`: `true` if the paths differ only in case or Unicode normalization. `unclaimed`: `true` if no item claims the file; the file moves with its folder (R6). `requires_temporary_step`: `true` if the proposal is part of a rename cycle. `conflicting_paths`: the unchanged paths, proposal sources or folders that a blocked destination conflicts with.
- `diagnostics[].rule`: `ambiguous_identity`, `conflicting_provider_ids`, `misplaced_series`, `orphan_extra`, `orphan_sidecar`, `orphan_artwork`, `unclaimed_file`, `missing_part`, `duplicate_part`, `clashing_version_labels`, `insufficient_facts`, `invalid_name_under_profile`, `folder_mismatch`, `overlapping_episodes`, `missing_year`, `missing_series_year`, `missing_provider_id`, `missing_episode_title`, `missing_episode`, `unexpected_episode`, `duplicate_work`, `duplicate_episode` (R11), `missing_asset`, `missing_subtitle` (R12), `reference_year_mismatch` (X18), `release_junk` (X22), `profile_violation` (R7-R9). A validation's `input_diagnostics` lists the reference entries that no item took (`reference_movie`, `reference_series`). `severity`: `error`, `warning` or `info` (R7). `reason`: the ambiguity reason, the provider, the blocked reason, or the fact that a `folder_mismatch` disagrees on (`title`, `year`, `edition`, `series_title`, `series_year`, `season`); `proposed_path`: the name that a profile violation's entry has under the profile.
- `diagnostics[].reason` (`build_rules`, X10): `syntax`, `missing_name`, `unknown_vocabulary`, `malformed_vocabulary`, `empty_entry`, `duplicate_entry`, `reserved_token`, `colliding_entry`, `unknown_entry`, `unknown_language`, `unknown_preset`, `invalid_preset_value`, `duplicate_overlay_name`, `conflicting_change`, `ineffective_removal`; For a `colliding_entry`, `message` names the other vocabulary that contains the entry. `entry` is in the form that the vocabulary compares (`Making Of` → `makingof`); `overlays` lists the conflicting overlays in declared order; `line` is 1-based in the overlay's document.
- `rank_candidates` (P1-P6): `kind` is `movie`, `series` or `episode` (required on a candidate, optional in the facts); `handle` is the caller's opaque name for a candidate, unique per call. `outcome`: `selected`, `ambiguous`, `no_match` or `invalid_input`; `selected` is the chosen handle, `null` otherwise; `tied` lists the handles that an ambiguous result could not choose between. `reason`: `provider_id`, `best_score`, `tied_scores`, `several_provider_id_matches`, `no_candidates`, `all_contradicted`, `provider_id_match_contradicted`, `insufficient_evidence`, `duplicate_handle`. `components[]` are `{ component, points, reason, provider, matched_title }`, with `component` `provider_id` (one per namespace both sides carry), `title` and `year` (always), `episode` (when the candidate has an episode and the facts have episodes) and `date` (when an episode candidate and the facts both have a date); `contradictions[]` are `{ component, reason, provider }`, `component` also `kind`. `score` is the sum of the components' points, not a probability.
