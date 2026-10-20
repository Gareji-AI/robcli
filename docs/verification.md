# Validation scope

The first public release is 0.6.0. Earlier version numbers refer to local
development milestones, not previously published crates.

Automated tests use fake accounts/profiles, mock HTTP/UI, fake clocks and
loopback servers. They never read real credentials, call Roblox, issue keys or
change an account. CI adds build/test checks on Windows, Linux and macOS;
that is not end-to-end Firefox verification on every platform.

## Separately authorized live observations

| Area | Observed | Not established |
| --- | --- | --- |
| Dashboard | Identity, groups, selected experience lists; owner/URL diagnostics | Completed CSV or key issuance |
| Creator Store | Four anonymous GETs: Music 3, SFX 3, zero-price Model 2, one music detail | Every category, usage rights or asset safety |
| Avatar Marketplace | One anonymous Jacket search, 10 cards | Studio application, account save, ownership or every category |
| Charts/music/commerce | Five anonymous GETs: 97 chart rows (10 projected), 10 Top 100 songs, 4 experience songs, 10 products, 2 passes | All filters/pages, individual game details/votes or Place resolution |
| Profiles/communities | Five anonymous GETs: public profile, 10 community results, one community, 5 public roles, empty public game page | User search/memberships/creations, non-empty game rows or pagination |

Counts are point-in-time observations, not fixed expectations or permission
grants. Most category/edge cases are specification/mock-tested only. The five
saved public charts/music/commerce responses were replayed offline; capture and
replay hooks were removed before release. No raw credential or private screenshot
was committed. Public profile smoke evidence retained counts/IDs/status only.

An experimental anonymous avatar-detail POST returned 403. Its command and
constructor were removed before release; no `avatar get` is shipped. No credential
fallback, CSRF exchange or alternate-endpoint probe was used to overcome it.

## Known Dashboard blocker

A manually authenticated CSV attempt stopped at
`read_session_cookie / driver HTTP 404 / no such cookie`, before same-account
and owner re-verification. That named WebDriver path is a hypothesis for the
404: the cookie name starts with a dot, and the named lookup can miss it.
This is not proof of a CAPTCHA or 2FA challenge, and no CSV success is claimed.

The interactive lookup now switches to the content context, requires the current
document URL to equal the approved analytics HTTPS URL, then uses W3C Get All
Cookies. It accepts exactly one `.ROBLOSECURITY` cookie whose domain is
`.roblox.com` or `roblox.com`. Same-account confirmation and experience-owner
verification still run before any CSV click. A missing context stops as
`session_cookie_context_unverified`. A missing or duplicate cookie stops as
`manual_login_session_missing`. The change is covered by offline WebDriver
tests only. Live CSV export is still not claimed.

## Publication verification

Release preparation uses format, clippy, offline Rust/JavaScript tests, a release
build, archive inspection, package rebuild and a publish dry run. Git history is
secret-scanned before its first push. Local captures, OS traffic state, browser
profiles, Cargo credentials and private keys are outside the package allowlist.
Registry acceptance and GitHub CI are verified separately; a dry run is not a
successful publication. See [releasing](releasing.md).
