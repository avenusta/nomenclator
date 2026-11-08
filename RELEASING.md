# Releasing

The three crates, and later the packages for other platforms, share one version and one changelog, and are released together from `main`. A `v<version>` tag starts the release workflow, which runs `just check` and publishes the crates to crates.io with trusted publishing.

## Release a version

1. Check that CI on `main` is green and that the `[Unreleased]` section of `CHANGELOG.md` lists the changes.
2. Choose the level. While the version is 0.x, a change in naming behaviour or a breaking API change is `minor`, and a fix is `patch` (README, "Versioning").
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

A run that is not started by a tag runs every step with `cargo publish --dry-run` and publishes nothing.

## First release

crates.io accepts trusted publishing only for a crate that exists, so version 0.1.0 is published once from a maintainer's machine:

1. Create a crates.io API token with the `publish-new` scope, and run `cargo login`.
2. Run `cargo publish --workspace`.
3. For each crate on crates.io, open **Settings**, then **Trusted Publishing**, and add the repository `avenusta/nomenclator` with the workflow `release.yml`.
4. Revoke the token.
5. Run `cargo release 0.1.0 --execute --no-confirm` to date the changelog and push the `v0.1.0` tag. The workflow skips the crates that are already published.
