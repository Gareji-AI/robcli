# Changelog

## 0.8.1

Distribution-only patch. No CLI behavior, command surface, API contract, or
credential handling changed; the crate contents differ only in the version,
this changelog, and the release-archive docs.

- Document installing through Rokit, so a Roblox project can pin `rob` next to
  `rojo` and `stylua` instead of requiring Rust 1.99 and a native C toolchain.
- Add a `macos-x86_64` archive. Rokit validates the operating system of a
  downloaded binary but not its architecture, so an Intel Mac previously
  installed the aarch64 build, reported success, and then could not execute.
- Write the `.sha256` sidecar with an explicit newline. Text mode emitted CRLF
  on Windows, which broke `sha256sum -c`.
- Report the target name for an unapproved archive target, instead of a raw
  `KeyError` repr.
- Package only the assets a release does not already contain, and take the
  packager from the dispatch commit rather than the release tag. A tag that
  predates the packaging script otherwise fails before building anything.

Approved archive matrix: Windows x86_64 (`.zip`), Linux x86_64 (`.tar.gz`),
macOS aarch64 (`.tar.gz`), macOS x86_64 (`.tar.gz`).

## 0.8.0

- Send `games thumbnails --count` as the published `countPerUniverse` query, and
  require that key in the anonymous request allowlist.
- Report a conservative maximum of five direct attempts for `apikey list --all`,
  including with `--allow-csrf-handshake`. A one-page listing stays at two or three.
- Accept the documented `isContentRestricted` game rows that carry an empty name,
  preserving the flag instead of inventing a name or rejecting the response.
- Accept the documented nullable Creator Store `asset.name` and keep it null.
- Return exit code 3 with `httpStatus: 403` for a CSRF 403, including after the one
  permitted replay. Exit code 2 stays reserved for refusals before a request.
- Send `accessFilter` as the published `int32` enum value `2` (public) on both
  creation routes instead of the undocumented enum name `Public`.
- Accept the documented nullable votes array. A null vote wrapper keeps the game
  snapshot and leaves the vote totals null; a non-null array still needs exactly
  one matching row.

## 0.7.0

- Compare saved JSON snapshots offline, and render one success envelope as
  Markdown or CSV without fetching Roblox.
- Read one anonymous game icon, one page of thumbnails, and one page of public
  badges. Thumbnail image IDs stay distinct from the Universe ID.
- Read one avatar bundle's typed child IDs. Unknown item types stay unsupported.
- Add offline `doctor` and `plan`. The plan counts requests and does not send them.
- Build release archives and SHA-256 checksums for Windows x86_64, Linux x86_64,
  and macOS aarch64. A checksum is not a publisher signature.
- Keep Dashboard resource IDs inside the signed 64-bit range.
- Return exit code 3 for anonymous HTTP refusals, including 429 and 5xx, with no
  retry and no credential fallback.
- Send the fixed public query from `groups list`.
- Return exit code 1 when JSON stdout cannot be written, including argument errors.
- Prove the loopback driver owns its port before a session cookie is sent.
- Look up the interactive CSV session cookie from the cookie list. This does not
  claim that a live CSV export succeeded.

## 0.6.1

- Adopt the framed, transparent-background project icon and display it in the
  README; include the icon in the crate archive.
- No command, authentication, networking or traffic-limit behavior changes.

## 0.6.0

First public release. Package: `robcli`. Executable: `rob`.

- Anonymous public profile/community search, metadata, public roles/memberships
  and creations.
- Public game charts, Music Top 100, experience snapshots/votes, published Top
  Songs, passes and developer-product metadata.
- Anonymous Creator Store asset and avatar Marketplace searches, typed IDs and
  offline page links.
- Firefox-session Dashboard identity, groups, owned experiences/settings and
  key metadata; separately gated personal API-key issuance.
- JSON envelopes, offline command discovery and shared conservative traffic
  limits, without automatic retries or credential fallback.
- Dual license: MIT OR Apache-2.0.

Known limitations: browser CSV export remains experimental/blocked, key issuance
has not been verified end to end, and public metadata never grants asset rights,
ownership or management permissions. See [validation scope](docs/verification.md).
