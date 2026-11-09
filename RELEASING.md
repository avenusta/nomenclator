# Releasing

The three crates, and later the packages for other platforms, share one version and one changelog, and are released together from `main`. A `v<version>` tag starts the release workflow, which runs `just check`, publishes the crates to crates.io with trusted publishing, builds the CLI and the server for every platform, and attaches the archives with their checksums to the GitHub release.

## Release a version

1. Check that CI on `main` is green and that the `[Unreleased]` section of `CHANGELOG.md` lists the changes.
2. Choose the level. While the version is 0.x, a change in naming behaviour or a breaking API change is `minor`, and a fix is `patch` (`CHANGELOG.md`, "Versioning").
3. Preview the release, then run it:

   ```sh
   cargo release minor
   cargo release minor --execute
   ```

cargo-release raises the one version in `[workspace.package]`, dates the `[Unreleased]` section of `CHANGELOG.md`, commits `Release <version>`, tags `v<version>` and pushes both. The release workflow publishes the crates from the tag and creates a GitHub release with that version's changelog section as its notes.

## Check the workflow without publishing

```sh
gh workflow run release.yml
```

A run that is not started by a tag runs every step with `cargo publish --dry-run`, keeps the platform archives as workflow artifacts, and publishes nothing.

## First release

crates.io accepts trusted publishing only for a crate that exists, so version 0.1.0 is published once from a maintainer's machine with an API token:

1. Create a token at <https://crates.io/settings/tokens/new> with the scopes `publish-new` and `trusted-publishing`, the crate scope `nomenclator*` and the shortest expiry.
2. Publish the crates:

   ```sh
   CARGO_REGISTRY_TOKEN=<token> cargo publish --workspace
   ```

3. Register the release workflow as the trusted publisher of each crate. This is the same as adding it under **Settings**, then **Trusted Publishing**, on the crate's page:

   ```sh
   for crate in nomenclator nomenclator-cli nomenclator-grpc; do
     curl -sS -f -X POST https://crates.io/api/v1/trusted_publishing/github_configs \
       -H "Authorization: <token>" -H "Content-Type: application/json" \
       -A "nomenclator-release-setup (github.com/avenusta/nomenclator)" \
       -d "{\"github_config\":{\"crate\":\"$crate\",\"repository_owner\":\"avenusta\",\"repository_name\":\"nomenclator\",\"workflow_filename\":\"release.yml\",\"environment\":null}}"
   done
   ```

4. Revoke the token at <https://crates.io/settings/tokens>.
5. Run `cargo release 0.1.0 --execute --no-confirm` to date the changelog and push the `v0.1.0` tag. The workflow skips the crates that are already published.
