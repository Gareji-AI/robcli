# Changelog

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
