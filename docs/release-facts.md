# Release facts: comparison with public test sets

`parse_release` reads the release facts of a name (decision X19). This page reports how often it matches two public release-name test sets. The sets are listed in `references/SOURCES.md`. They are not committed, and no case is copied into the corpus.

Run the comparison with `just release-benchmark` after downloading the sets.

## Method

- The comparison uses 1,011 names from the larger set. Entries with parser options are left out.
- A name without a known media extension is passed as a folder name, as a caller passes it.
- The set's values are mapped onto this crate's values: `H.264` is `h264`, `Dolby Digital` is `dolby_digital`, and so on. The tables are in `crates/nomenclator/tests/release_benchmark.rs`.
- **stated** counts the names for which the set states the fact. **matched** counts those for which `parse_release` returns the same value. **wrong** counts a different value. **extra** counts a value for a fact that the set does not state.
- For lists (HDR formats, audio codecs), a match means that every value the set states is in the result.

## Results

Measured on 2026-11-07.

| Fact | Stated | Matched | Rate | Wrong | Extra |
| --- | ---: | ---: | ---: | ---: | ---: |
| resolution | 483 | 458 | 94.8% | 4 | 77 |
| source | 571 | 507 | 88.8% | 10 | 35 |
| video_codec | 498 | 466 | 93.6% | 1 | 56 |
| bit_depth | 26 | 21 | 80.8% | 0 | 3 |
| hdr | 9 | 4 | 44.4% | 4 | 2 |
| audio_codecs | 226 | 200 | 88.5% | 0 | 16 |
| audio_channels | 126 | 105 | 83.3% | 2 | 12 |
| release_group | 601 | 446 | 74.2% | 38 | 57 |
| checksum | 26 | 26 | 100.0% | 0 | 6 |
| revision | 32 | 31 | 96.9% | 0 | 4 |
| streaming_service | 33 | 26 | 78.8% | 0 | 3 |
| container | 185 | 153 | 82.7% | 0 | 357 |

## Known differences

- **HDR:** the set reports a plain `HDR` as HDR10. `parse_release` reports it as `hdr`, because the name does not state which format it is.
- **Release groups:** the set reads some words that end a name after a space as the group (`x264 CCATS`). `parse_release` reads only a group after a `-` or in leading brackets, because a word after a space can be a title word. The set keeps an index site's tag on the group (`LOL[ettv]`); `parse_release` drops it, and the comparison removes it from the set's value.
- **Container:** the set counts `.torrent` and `.nzb` files and a container named in the name (`MP4-GROUP`). `parse_release` reports the file's own extension.
- **Names without facts:** some names in the set are hashes or abbreviations (`dmd-aw.avi`). The set's values for them come from knowledge that is not in the name.
- **extra** is not a false-positive count. The set does not state every fact that a name contains, so most extra values are correct.
