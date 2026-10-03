# Releasing Ironhold

All crates in the workspace share one version and are released together.

## 1. Prepare

1. Set the new version in `Cargo.toml`: `version` under `[workspace.package]`, and the `version` of each `ironhold-*` entry under `[workspace.dependencies]`.
2. Add a section for the release at the top of `CHANGELOG.md`.
3. Check that every crate packages and builds on its own, the same way crates.io will:

   ```bash
   cargo publish --workspace --dry-run
   ```

4. Open a pull request with these changes and merge it once CI passes.

## 2. Publish

From an up-to-date `main`:

```bash
cargo publish --workspace
```

This publishes every crate in dependency order. It can't be undone: a published version can be yanked, but never deleted or replaced.

## 3. Tag and announce

```bash
git tag -a vX.Y.Z -m "Ironhold X.Y.Z"
```

```bash
git push origin vX.Y.Z
```

Then create a GitHub release for the tag, with the release's changelog section as its notes.

## 4. Check the published release

In an empty folder, outside this repository:

```bash
cargo install ironhold-cli --version X.Y.Z
```

```bash
ironhold new release-check && cd release-check && cargo test
```

## Credentials

Publishing needs a crates.io API token. Use a token limited to the `ironhold*` crates with only the publish scopes, give it a short expiry, and revoke it after the release. Moving releases to crates.io Trusted Publishing, so GitHub Actions publishes with short-lived credentials and no long-lived token exists, is planned.
