# nomenclator

nomenclator implements reusable media naming conventions for movie and TV libraries. It reads file and folder names and returns titles, years, provider IDs, seasons, episodes, extras, sidecars and ignore decisions. If the names do not determine a fact, the result marks the fact as ambiguous or unknown.

The project is a work in progress. It contains a pure Rust library (`crates/nomenclator`), a gRPC server that exposes the same implementation to other languages (`crates/nomenclator-grpc`), and a diagnostics CLI (`crates/nomenclator-cli`).

```sh
just check
```

## gRPC server

The contract is `crates/nomenclator-grpc/proto/nomenclator/v1/nomenclator.proto` (package `nomenclator.v1`). It defines the batched operations `ParseNames`, `GroupParts`, `EvaluateIgnore`, `Interpret`, `RankCandidates`, `RenderNames`, `PlanRenames` and `Validate`, plus `GetVersion`. Each query carries a caller handle, and its result returns the same handle. Each result contains the library's facts, outcomes and evidence with one wire field per library field. The server is stateless. It calls the library directly and does not reimplement any rule.

```sh
cargo run -p nomenclator-grpc --bin nomenclator-server -- --tcp 127.0.0.1:50051
cargo run -p nomenclator-grpc --bin nomenclator-server -- --unix /tmp/nomenclator.sock
```

The server prints one `listening on tcp:...` or `listening on unix:...` line. With `--tcp 127.0.0.1:0`, the operating system chooses the port. The server stops on Ctrl-C and removes its socket file. The socket file must not exist when the server starts. You can generate a client from the `.proto` file in any language. The build does not require a system `protoc`.

- Paths are root-relative and separated by `/`. A folder path ends in `/`.
- Protobuf strings are UTF-8. If a name is not valid UTF-8, the caller reports it as `unsupported_encoding` itself and does not send it (X4).
- The project keeps three separate versions: the wire contract (`nomenclator.v1`), the naming rules (`rules_version`, the library's version) and the build (`implementation_version`). A compatible wire change can come with a change in naming behaviour. Every response states the rules it was answered under in `rules`.

`cargo test` also runs every non-pending corpus case through an in-process server. This test checks that direct calls and wire calls return the same facts.

## Diagnostics CLI

`nomenclator` reads names and real folders the way a caller does (entries and `.ignore`/`.nomedia` files). It prints the library's results as a table, or as JSON with `--format json`. It does not rename any file.

```sh
cargo run -p nomenclator-cli -- parse "Northern Passage (2021) {edition-Extended}.mkv" --library movies
cargo run -p nomenclator-cli -- parse "Show/Season 02/Show S02E03E04.mkv" --library tv
cargo run -p nomenclator-cli -- release "Northern.Passage.2021.2160p.UHD.BluRay.REMUX.HDR10.HEVC.TrueHD.Atmos.7.1-NPX.mkv"
cargo run -p nomenclator-cli -- interpret ~/Media/Movies --library movies
cargo run -p nomenclator-cli -- plan ~/Media/TV --library tv --format json
cargo run -p nomenclator-cli -- validate ~/Media/Movies --library movies --fail-on warning
cargo run -p nomenclator-cli -- diff ~/Media/Movies --library movies --rules builtin --rules mine.yaml
```

`--language <tag>` is repeatable, such as `--language en --language de`. It limits the vocabulary to the words of those languages plus the shared base (decision X11). `--inputs inputs.yaml` passes facts that you already know: hints per path, overrides and series numbering (decisions T20, X16, X17). `validate --reference reference.yaml` checks the library against catalogue years and episode lists (X18). `--rule-file .mediaignore` or `--rule-file .skip:glob` is repeatable and reads additional rule files, in gitignore syntax or with one glob per line (X14). `--overlay <file>` is repeatable and applies YAML or JSON overlays that add or remove vocabulary words. For example, an overlay can remove a release word that truncates your titles (decision X10).

With `movies`, `parse` reads a movie name. With `tv`, `parse` reads an episode file, or a series folder if the name has a trailing `/`. `plan` prints proposals and blocked renames under the default profile. `--profile profile.yaml` sets the output profile for `plan` and `validate`, such as `movie_layout: flat` or `provider_tag_style: brackets_id` (decision R15). `plan --from-profile old.yaml --profile new.yaml` converts a library from one layout to another (R18). `--target-filesystem posix` relaxes the portable character rules. `plan --script sh` or `--script ps1` prints a script that performs the proposals, and `--undo-script undo.sh` writes the script that reverses them (R16). `queries <dir>` prints the provider search queries of each item (P7). `import <download> <library>` proposes moves of a download folder's files into a library: season packs split into episodes, a known work goes into its library folder, samples and release junk stay, and collisions with library files are blocked (R17). nomenclator itself never renames a file. `validate` prints errors and warnings, and `--all` adds info diagnostics. `validate` exits 1 if a diagnostic reaches the `--fail-on` severity (`error` by default). Any command exits 2 if it cannot run.

## Versioning

The project has three versions, and each has its own compatibility guarantee:

- **Wire contract** (`nomenclator.v1`). It changes only compatibly. An incompatible change creates a new package (`v2`).
- **Naming rules** (`rules_version`, the library's version). While the version is 0.x, any minor version bump may change naming results. A patch version bump only fixes bugs, and a corpus case pins each fix.
- **Implementation** (`implementation_version`). It identifies the build that produced a result.

Every crate, and every package for another platform, has the same version. `CHANGELOG.md` is the one list of changes for all of them. It lists naming-behaviour changes first, then changes to the library API, the wire contract, the CLI and the packages. Each release on GitHub has that version's section as its notes.

## Contributing

`CONTRIBUTING.md` describes how to send a change, and `AGENTS.md` and `CODING_STANDARDS.md` hold the rules for people and coding agents. Report a vulnerability as `SECURITY.md` describes.

## License

MIT, see `LICENSE`.
