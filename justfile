# Format, lint and test: the gate before review
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

# Check dependency licenses and security advisories (needs cargo-deny)
deps:
    cargo deny check

# Lint the wire contract and check it for breaking changes against main (needs buf)
proto:
    buf lint crates/nomenclator-grpc/proto
    buf breaking crates/nomenclator-grpc/proto --against '.git#branch=main,subdir=crates/nomenclator-grpc/proto'

# Check the core crate's public API for breaking changes against a release tag (needs cargo-semver-checks)
semver tag:
    cargo semver-checks --package nomenclator --baseline-rev {{tag}}

# Pending cases that pass per corpus file; `just progress movies/names` also lists that file's failures
progress filter="":
    PENDING="{{filter}}" cargo test --test corpus -- --nocapture --quiet

# Interpret and plan synthetic libraries of 1k, 10k and 100k entries in release mode, within
# absolute bounds, and write target/benchmarks.json
bench:
    cargo test --release --test scaling -- --include-ignored --nocapture

# Compare parse_release with the public release-name sets in references/release-names/
release-benchmark:
    cargo test --release --test release_benchmark -- --ignored --nocapture

# Remove the pending mark from cases that now pass
promote:
    PROMOTE=1 cargo test --test corpus -- --quiet
