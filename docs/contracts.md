# Dashboard contract evidence

Implementation evidence, not a guarantee of live compatibility. Checked
2026-10-17 JST using public documentation/source and a user-supplied screenshot.
The initial 0.2 implementation was tested only with synthetic fixtures. Subsequent
separately authorized identity/group/experience reads succeeded. A single bounded
0.2.1 diagnostic export identified a Roblox-web `login_route` during `probe`;
no export click/file or key issuance occurred. This does not prove a 2FA/CAPTCHA
challenge. Automated regression tests remain offline.

## Published API reads

[Roblox's public OpenAPI](https://create.roblox.com/docs/cloud/openapi.json)
contains these session-authenticated read routes:

| Host | Method/path | Use |
|---|---|---|
| users.roblox.com | GET /v1/users/authenticated | Fresh account ID |
| groups.roblox.com | GET /v2/users/{userId}/groups/roles | Groups/roles. Required query is fixed: `includeLocked=false`, `includeNotificationPreferences=false`, `discoveryType=0` |
| develop.roblox.com | GET /v1/user/universes | Personal collection |
| develop.roblox.com | GET /v1/groups/{groupId}/universes | Explicit group collection |
| develop.roblox.com | GET /v1/universes/{universeId} | Owner check |
| develop.roblox.com | GET /v1/universes/{universeId}/configuration | Read configuration |
| thumbnails.roblox.com | GET /v1/games/icons and /v1/games/multiget/thumbnails | Anonymous image metadata. No image bytes |
| badges.roblox.com | GET /v1/universes/{universeId}/badges | One anonymous public badge page |
| catalog.roblox.com | GET /v1/bundles/{bundleId}/details | Anonymous bundle composition |

Collection query: `isArchived`, `limit` (10/25/50/100), `sortOrder`, optional
`cursor`. Universe metadata's `creatorType` and `creatorTargetId` must match the
explicit selection before configuration is read. The configuration **GET is v1**;
do not substitute a v2 write route. Output positively projects documented fields.

The public `/analytics-query-api/v1/universes/{universeId}/metrics` route requires
API-key/OAuth authentication, not the Firefox cookie. That data-plane API belongs
in rbxoc. Discovery of an undocumented Dashboard analytics gateway is not proof
of its body contract; robcli does not construct or probe that request.

## Existing chart UI for CSV

The user's screenshot shows a Retention page and a “Day 1 retention” chart with
an ellipsis menu: Download CSV, View source query, Add to dashboard. The chart
reports “No data for selected period” and Download CSV is disabled. This is
evidence of the UI action and a required no-data stop, **not evidence of a
successful export or a verified DOM selector**. Private titles/IDs and the
screenshot itself are not copied into this repository.

The browser adapter uses an exact chart title, unique menu, enabled Download CSV
and one bounded download. Missing/ambiguous selectors fail closed. The UI selectors
and completed CSV export remain unverified. Private-mode attestation, cookie
injection and bounded navigation reached the login-route diagnostic in a real
owned browser; this is not successful Dashboard login.
Fresh-page filter defaults are intentionally disclosed; no claim of exporting
the date range or filter state of an existing user's tab.

Dashboard links use `/dashboard/creations/experiences/{id}/overview`, `/configure`
and `/analytics/retention`, with an optional numeric `groupId`; key administration
uses `/dashboard/credentials`. Links are navigation hints, not permission grants.

## Session-authenticated key administration

The Dashboard control plane is not API-key authentication. Contract references:

- [rbx-forge/rbx-cli key API adapter](https://github.com/rbx-forge/rbx-cli/blob/main/crates/rbx-apikey/src/api/api_keys.rs)
- [Shared adapter and CSRF handling](https://github.com/rbx-forge/rbx-cli/blob/main/crates/rbx-apikey/src/api/mod.rs)
- [Scope builder](https://github.com/rbx-forge/rbx-cli/blob/main/crates/rbx-apikey/src/scope_builder.rs)

These are third-party implementation references, not a stable official contract.
No source code is vendored. The fixed host is `apis.roblox.com`:

| Method/path | Body/response |
|---|---|
| POST /cloud-authentication/v1/apiKeys | List body: cursor, limit, reverse, optional numeric groupId. Response: cloudAuthInfo array, optional nextCursor |
| POST /cloud-authentication/v1/apiKey | Create body: cloudAuthUserConfiguredProperties. Response: cloudAuthInfo and apikeySecret |

Configured properties: `name`, `description`, `isEnabled`, `expirationTime`,
`allowedCidrs`, `scopes`. Each scope has `scopeType`, `targetParts`, `operations`.
Group-owned key creation is not inferred from list support or resource scopes.

Server expiry behavior may differ from the requested creation metadata. Report
what was returned; do not add an unapproved follow-up PATCH/enable/revoke. Unknown
creation outcomes require manual reconciliation. Raw key/previews are never
included in metadata listing or the creation receipt.

An explicit `--allow-csrf-handshake` permits one 403+`x-csrf-token` validation
replay per command; requests with Roblox challenge headers are never replayed.
Both attempts consume the shared quota. Ordinary errors, 429/5xx and transport
failures have no automatic retry.

## Browser tooling boundary

Mozilla [geckodriver releases](https://github.com/mozilla/geckodriver/releases)
and [Firefox capability documentation](https://firefox-source-docs.mozilla.org/testing/geckodriver/Capabilities.html)
describe the separate WebDriver tool and preference setup. The local 0.37.1 win64
ZIP was checked against its official GitHub asset SHA-256. Initial installation
ran version/help only; subsequent authorized CSV attempts started new owned
private sessions and ended them on the authentication/navigation stop.

Firefox's own page requests are outside the direct-client five-request quota.
The adapter requires separate browser-page consent and limits navigations/clicks;
it does not claim a five-HTTP-request limit for a modern web page. Only a newly
owned private process tree is controlled, with no real-profile cloning/attachment.

## Safe diagnostics and human authentication

The old adapter collapsed navigation refusal, login routing, challenge-like
iframes and verification-like dialogs into one error. Version 0.2.1 retains the
stop conditions but positively projects fixed signal/origin/route/phase enums.
Real observation is a Roblox-web login route, not evidence identifying the
reason for that redirect (missing browser state, policy, or something else).
Synthetic tests exercise the exact production script, including no raw URL/DOM
echo and rejection of another universe/group after human login.

`--interactive-login` is an explicit, bounded human-authentication mode. The
private browser is visible; the user authenticates on Roblox itself and confirms
in stdin within five minutes. There is no WebDriver polling while the user types,
credential autofill, challenge solution, stealth flag or browser-profile copy.
Then the exact target is rechecked and the owned browser's named cookie is used
only in memory to reverify the same account against the published whoami route.
Additional identity verification counts within the same hard request budget.
Different accounts/targets stop before CSV. Browser-managed user login traffic
is not represented as five total HTTP requests. Complete manual authentication
and CSV download have not yet been confirmed end to end.

## Canonical URL and owner verification correction

A user-confirmed Retention URL has the same universe/tab path, no `groupId`, and
`annotation=Benchmark`. Treating query-group presence as mandatory produced a
false target mismatch after the user completed login. A synthetic reproduction
failed with the production 0.2.1 script and passes with 0.2.2.

Version 0.2.2 accepts only the fixed Benchmark annotation and an optional numeric
group hint. `--group` explicitly selects the owner if the URL omits it. Before
any chart/menu/download action, the fixed universe metadata API must confirm its
ID, creatorType and creatorTargetId. Contradictory/duplicate group hints and
different universe/tab paths still stop. The shared ownership check also backs
settings reads; URL parameters are not authorization or ownership evidence.

In interactive mode the original Firefox session establishes the expected user
but is no longer injected into the new private browser, and the seed navigation
is omitted. Reservations are initial identity, analytics navigation, manually
authenticated identity, universe-owner read, one CSV click: five total. The
non-interactive mode instead uses identity, seed, analytics navigation, owner
read, CSV click: also five. No quota increase, automatic login, retries or
credential persistence is introduced.

A user-confirmed manual run subsequently stopped with the old generic
`browser_control_failed`, without a completed CSV. The exact control action was
not captured, so neither a missing cookie nor a JavaScript error is established.
Version 0.2.3 adds bounded decoding of fixed W3C error names and action/phase
labels, never message/stacktrace/URL/credential output. Synthetic protocol tests
distinguish cookie and script errors and reject unrecognized private strings.
Further real-account diagnosis still requires a separately confirmed human run;
it is not an automatic retry or a claim that authentication/CSV is fixed.

The final separately approved 0.2.3 human run reached the exact target probe,
then reported `read_session_cookie`, local driver HTTP 404, W3C `no such cookie`.
Thus named-cookie lookup in the current controlled document is the observed
blocker, before browser-account/owner API reverification. This is not evidence of
a 2FA/CAPTCHA refusal, nor proof about cookies in other origins, windows or
partitions. Do not infer the cookie's storage scope or copy other credentials
without further evidence. No CSV completed. The user authorized one final run;
no additional live attempt followed this result.
