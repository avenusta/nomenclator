# nomenclator

A universal naming scheme for media libraries.

nomenclator's  goal is to bring and enforce a universal naming scheme for media libraries. It recognizes and reads a broad range of existing file and folder naming styles, gathered from existing naming conventions. It validates them against one clean, consistent scheme. It reports naming violations and proposes corrections, so libraries can adopt the same convention

For a library that contains `Blade.Runner.1982.1080p.BluRay.x264.mkv`:

```sh
nomenclator validate ./Movies --library movies
```

```text
SEVERITY  CODE  RULE               PATHS                                    DETAIL
warning   N403  profile_violation  Blade.Runner.1982.1080p.BluRay.x264.mkv  → Blade Runner (1982)/Blade Runner (1982).mkv
0 errors, 1 warnings, 1 infos, 0 suppressed
```

The tool reads the title and year, warns that the path does not follow the naming scheme, and proposes a corrected path. It does not rename the file.

The project is a work in progress.

## Build

Install Rust with rustup, then run from the repository root:

```sh
cargo build -p nomenclator-cli
```

`rust-toolchain.toml` pins the toolchain. Rustup installs it on the first build. The examples below use `cargo run` to build and run the CLI from this checkout.

## Quick start

Parse a movie name with provider IDs, an edition tag, a 3D layout and release details:

```sh
cargo run -p nomenclator-cli -- parse \
  "Avatar (2009) {tmdb-19995} {imdb-tt0499549} {edition-Extended} 3D HSBS 1080p BluRay x264 DTS 5.1.mkv" \
  --library movies
```

The result includes the title, year, both provider IDs, edition, 3D layout and container, with evidence spans for the parsed facts:

```text
FACT               VALUE              EVIDENCE
alternate_titles   -
container          mkv
edition            Extended           edition_tag [53,61]
edition_hint       unknown
format_3d          half_side_by_side  format3d_word [66,70]
provider_ids.imdb  tt0499549
provider_ids.tmdb  19995
title              Avatar             text_before_year [0,6]
trusted            true
year               2009               bracketed_year [8,12]
```

Read the release details from the same name:

```sh
cargo run -p nomenclator-cli -- release \
  "Avatar (2009) {tmdb-19995} {imdb-tt0499549} {edition-Extended} 3D HSBS 1080p BluRay x264 DTS 5.1.mkv"
```

This resolves the resolution to `1080p`, source to `bluray`, video codec to `h264`, audio codec to `dts`, and audio channels to `5.1`.

Parse a TV episode, inspect a library, or propose renames:

```sh
cargo run -p nomenclator-cli -- parse "Show/Season 02/Show S02E03E04.mkv" --library tv
cargo run -p nomenclator-cli -- interpret ~/Media/Movies --library movies
cargo run -p nomenclator-cli -- plan ~/Media/TV --library tv --format json
```

- `--library movies` and `--library tv` select separate naming pipelines. In TV mode, `parse` reads a file as an episode and a path with a trailing `/` as a series folder.
- The CLI prints tables by default. Use `--format json` for structured output.
- `plan` prints proposals and blocked renames. It does not rename files. No nomenclator command moves or renames media files.

## Commands

Examples use the binary built above:

```sh
# Read a title, year and provider IDs.
./target/debug/nomenclator parse "Blade Runner (1982) {tmdb-78}.mkv" --library movies

# Read resolution, source, codecs and audio.
./target/debug/nomenclator release "Blade.Runner.1982.1080p.BluRay.x264.DTS.5.1.mkv"

# Run any library operation with its input fields (corpus/README.md lists them).
echo '{ name_or_path: "Show (2019)/Season 01/" }' | ./target/debug/nomenclator run parse_series_name

# Suggest a library type from its names.
./target/debug/nomenclator suggest ./Media

# Group movies, episodes, extras, subtitles and artwork.
./target/debug/nomenclator interpret ./Movies --library movies

# Report naming violations; exit 1 on warnings or errors.
./target/debug/nomenclator validate ./Movies --library movies --fail-on warning

# Propose canonical paths without renaming files.
./target/debug/nomenclator plan ./Movies --library movies

# Propose moves from a download folder into a library.
./target/debug/nomenclator import ./Downloads ./Movies --library movies

# Produce provider search queries without calling provider APIs.
./target/debug/nomenclator queries ./Movies --library movies

# Compare built-in rules with a vocabulary overlay.
./target/debug/nomenclator diff ./Movies --library movies --rules builtin --rules mine.yaml
```

Use `--help` or `<command> --help` for details.

## Options

| Feature | Example | Effect |
|---|---|---|
| JSON output | `--format json` | Return structured results instead of tables. |
| Languages | `--language en --language de` | Read English, German and shared vocabulary. Default: all languages. |
| Vocabulary | `--overlay words.yaml` | Add or remove vocabulary words. Repeatable; accepts YAML or JSON. |
| Overrides | `--inputs inputs.yaml` | Supply known facts, hints and series numbering. |
| Ignore files | `--rule-file .skip:glob` | Read additional ignore rules. `.ignore` and `.nomedia` are read by default. |
| Output profile | `--profile profile.yaml` | Set naming layout and validation options. |
| Layout conversion | `plan --from-profile old.yaml --profile new.yaml` | Propose changes between layouts. |
| Rename script | `plan --script sh --undo-script undo.sh` | Print a shell script and write its reversal script. PowerShell: `--script ps1`. |
| Filesystem | `--target-filesystem posix` | Relax portable filename restrictions. |
| Reference checks | `validate --reference reference.yaml` | Check catalogue years and episode lists. |

## Library and gRPC

- **Rust:** use the pure library independently of the CLI. See the [API examples](crates/nomenclator/src/lib.rs), or run `cargo doc -p nomenclator --open`.
- **Other languages:** generate a client from the [gRPC contract](crates/nomenclator-grpc/proto/nomenclator/v1/nomenclator.proto). Start the stateless server with:

```sh
cargo run -p nomenclator-grpc --bin nomenclator-server -- --tcp 127.0.0.1:50051
```

## Reference

- [Naming conventions](docs/decisions.md)
- [Release facts](docs/release-facts.md)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)

## License

[MIT](LICENSE).
