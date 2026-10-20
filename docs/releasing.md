# Releasing robcli

For maintainers. Package: `robcli`; executable: `rob`; registry: crates.io.
The crate name, version and dual license must be intentional before upload.
An uploaded version cannot be replaced with different contents.

## Before publication

1. Review the diff and version/changelog. Keep experimental behavior and known
   limitations visible; publication is not permission for live Roblox operations.
2. Run formatting, clippy, all Rust tests, JavaScript regressions and a release
   build. GitHub CI must be green on the intended release commit.
3. Scan Git history for secrets before pushing newly public history. Never
   publish browser cookies, captured private responses or Cargo/GitHub tokens.
4. Inspect the actual package files and rebuild the archive:

```bash
cargo package --list --locked
cargo package --locked
cargo publish --dry-run --locked --registry crates-io
```

The manifest uses an explicit `include` allowlist. Review the generated
`Cargo.toml`, `Cargo.toml.orig`, `.cargo_vcs_info.json`, license files, source,
tests and docs. `target/`, local tools/captures, `.git/`, `.github/`, `.cargo/`,
`.env*` and OS-local credentials/traffic/key state are not package payloads.

## Publish once, then verify

Use Cargo's normal credential provider/login locally; do not pass tokens on the
command line, copy credentials into the repository or add a registry token to CI.

```bash
cargo publish --locked --registry crates-io
```

If upload fails or its outcome is unclear, inspect the registry version before
retrying. Never claim a dry run or local package as a successful release.
After acceptance, verify version, repository, license and archive checksum, then
test an install from crates.io into a separate temporary root and run only
offline help/schema/link commands.

Binary archives are separate from the crates.io package. The approved matrix is
Windows x86_64 (`.zip`), Linux x86_64 (`.tar.gz`), and macOS aarch64 (`.tar.gz`).
Each archive contains only `rob` or `rob.exe`, `license-mit`, and `license-apache`.
SHA-256 checksums identify the bytes. They do not prove publisher identity.

Download one archive and its `.sha256` file from the GitHub Release for the
same version. Verify the checksum before extracting. On Windows, `CertUtil
-hashfile robcli-<version>-windows-x86_64.zip SHA256` and compare it with the
digest in the `.sha256` file. On Linux or macOS, `sha256sum -c
robcli-<version>-<os>-<arch>.tar.gz.sha256` from the directory that contains
both files. Extract with `tar -xzf` or the platform zip tool. Put `rob` or
`rob.exe` on `PATH`. Run `rob --version`, `rob schema`, and `rob users link
--user 1`. Those checks stay offline. A matching checksum is not a publisher
signature, and the archives are not code-signed or notarized.
There is no code signing or notarization. CI on a pull request builds the
archives with `contents: read` and uploads them as Actions artifacts. Upload to
a GitHub Release happens only from `workflow_dispatch` with `publish=true` and
an existing tag. That job does not replace an existing asset. It does not mint
a new version.

Create the matching Git tag/GitHub Release after confirming registry acceptance.
Do not rewrite a published tag/version. Subsequent source fixes use a new patch
version. No credentials or real Roblox probes are required for CI or installation
verification.
