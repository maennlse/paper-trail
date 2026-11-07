# Release automation

`.github/workflows/release.yml` runs only when a `v*` tag is pushed. The tag must
exactly match `v` plus `[workspace.package].version` in `Cargo.toml`; a mismatched
tag fails before any build or publication. Prerelease versions such as
`v0.3.0-rc.1` are published as GitHub prereleases.

The release workflow reuses the existing Rust/GTK and standalone TUI workflows.
All tests, six native TUI target builds/linkage checks, the native GTK build,
and the GTK Flatpak build must succeed before the publish job starts. Normal
branch pushes and PRs still upload CI artifacts, but do not create releases.

## Release assets

- Six TUI archives: Linux musl, Windows MSVC with static CRT, and macOS, each for
  x86_64 and ARM64. Each archive includes the executable and TUI documentation.
- Native GTK Linux x86_64 `.tar.gz`: executable, `README.md`, and build/dependency info.
- GTK Linux x86_64 `.flatpak` bundle, with runtime-repository information.
- Per-package `.sha256` files and a combined `SHA256SUMS` (eight packages total).

The publish job downloads artifacts from the same workflow run, rejects missing
or duplicate packages, verifies every checksum, and stages only the expected
files. It creates a **draft**, uploads all assets, then publishes it with generated
release notes and the download guidance in `docs/release-notes.md`. A failed
upload leaves an unpublished draft, not a partially published release. Only
the publish job has `contents: write`; build jobs use read-only permissions.

## Creating a release (explicit maintainer action)

1. Review and merge the changes. Confirm the version in Cargo, Meson, and release
   metadata; update the lockfile and Flatpak sources when needed.
2. Wait for the branch checks to pass.
3. Create and push the matching version tag. For the current Cargo version:

   ```sh
   git tag -a v0.3.0 -m "Release 0.3.0"
   git push origin v0.3.0
   ```

4. Watch the **Release** workflow. Inspect the resulting downloads and release notes.

Do not run the example tag commands until publication is intended. The workflow
does not create tags or publish merely because a workflow file is added or a PR
is merged. No manual dispatch is exposed, so a version tag is the publication
boundary. GitHub repository Actions policies must permit the job's write token.

If publication fails after draft creation, inspect that draft and its uploaded
assets. To rerun the publishing workflow from scratch, delete only the failed
draft release (not the tag) through GitHub, then rerun the workflow. Existing
releases are not silently overwritten.

## Local validation without publishing

```sh
python3 -m unittest discover -s build-aux -p test_release.py
python3 build-aux/prepare_release.py validate-tag v0.3.0
python3 build-aux/package_desktop.py
```

The last command needs an existing native GTK release binary plus its runtime
libraries and `pkg-config`. `prepare_release.py collect ARTIFACTS DESTINATION`
can validate locally downloaded CI artifacts without contacting GitHub. Neither
Python helper creates tags or releases.
