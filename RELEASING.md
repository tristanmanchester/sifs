# Releasing `sifs`

SIFS uses a small manual release flow for now:

- publish the Rust crate to crates.io
- create a GitHub release tag
- publish the Homebrew formula to `tristanmanchester/homebrew-tap`

Homebrew releases are published from the tap repository at
`tristanmanchester/homebrew-tap`.

## Files involved

- [`Cargo.toml`](Cargo.toml) - crate version and release metadata
- [`Cargo.lock`](Cargo.lock) - locked dependency graph used by Homebrew
- [`README.md`](README.md) - user-facing install and usage documentation
- [`packaging/homebrew/sifs.rb`](packaging/homebrew/sifs.rb) - formula used for release preparation
- `tristanmanchester/homebrew-tap:Formula/sifs.rb` - published Homebrew formula

## Normal release flow

1. Make the changes you want to ship.
2. Bump `version` in [`Cargo.toml`](Cargo.toml).
3. Run local checks:

   ```bash
   cargo fmt --all --check
   cargo test --locked
   cargo build --locked
   cargo build --locked --features diagnostics --bins
   cargo publish --dry-run --locked
   cargo package --list
   ruby -c packaging/homebrew/sifs.rb
   ```

4. Merge the release commit to `main`.
5. Publish the crate:

   ```bash
   cargo publish --locked
   ```

6. Tag the same `main` commit and create the GitHub release:

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   gh release create vX.Y.Z --repo tristanmanchester/sifs --title "sifs vX.Y.Z" --generate-notes
   ```

7. Compute the GitHub source tarball checksum:

   ```bash
   curl -L https://github.com/tristanmanchester/sifs/archive/refs/tags/vX.Y.Z.tar.gz | shasum -a 256
   ```

8. Update `tristanmanchester/homebrew-tap`:

   ```bash
   git clone https://github.com/tristanmanchester/homebrew-tap.git /tmp/homebrew-tap
   cp packaging/homebrew/sifs.rb /tmp/homebrew-tap/Formula/sifs.rb
   ```

   Update the copied formula so `url` points at the new tag and `sha256` is the
   checksum from the previous step.

9. Validate the tap formula:

   ```bash
   brew uninstall --ignore-dependencies sifs || true
   brew install --build-from-source /tmp/homebrew-tap/Formula/sifs.rb
   sifs --help
   sifs search authenticate_token /tmp/tiny-repo --mode bm25 --offline --no-cache
   sifs mcp install --dry-run --client codex --source /tmp/tiny-repo --offline
   sifs mcp install --dry-run --client claude --scope local --source /tmp/tiny-repo --offline
   ```

10. Commit and push the tap update.

## Current install surfaces

- `cargo install sifs` installs the public `sifs` binary from crates.io.
- `brew install tristanmanchester/tap/sifs` installs the public `sifs` binary
  from the tap formula.
- `sifs-benchmark` and `sifs-embed` are supported diagnostics, but they are not
  part of the default package-manager install surface. Build them explicitly
  with:

  ```bash
  cargo build --release --features diagnostics --bins
  ```

## Notes

- Release tags should be `vX.Y.Z` and should match the `Cargo.toml` version.
- Publish the Homebrew formula to the dedicated tap, not `homebrew/core`.
- The Homebrew test must stay model-free and network-free after installation;
  use BM25 with `--offline --no-cache`.
