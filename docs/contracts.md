# Dashboard contract evidence

Implementation evidence, not a guarantee of live compatibility. Checked
2026-10-17 JST using public documentation/source and a user-supplied screenshot.
No real-account reads, key issuance, browser navigation or CSV export were run
while implementing version 0.2. Automated tests use synthetic fixtures.

## Published API reads

[Roblox's public OpenAPI](https://create.roblox.com/docs/cloud/openapi.json)
contains these session-authenticated read routes:

| Host | Method/path | Use |
|---|---|---|
| users.roblox.com | GET /v1/users/authenticated | Fresh account ID |
| groups.roblox.com | GET /v2/users/{userId}/groups/roles | Groups/roles |
| develop.roblox.com | GET /v1/user/universes | Personal collection |
| develop.roblox.com | GET /v1/groups/{groupId}/universes | Explicit group collection |
| develop.roblox.com | GET /v1/universes/{universeId} | Owner check |
| develop.roblox.com | GET /v1/universes/{universeId}/configuration | Read configuration |

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
and one bounded download. Missing/ambiguous selectors fail closed. The UI adapter
and Firefox private-mode attestation remain unverified against a real browser.
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
ZIP was checked against its official GitHub asset SHA-256; only `--version` and
`--help` were executed. No driver service or Firefox session was started.

Firefox's own page requests are outside the direct-client five-request quota.
The adapter requires separate browser-page consent and limits navigations/clicks;
it does not claim a five-HTTP-request limit for a modern web page. Only a newly
owned private process tree is controlled, with no real-profile cloning/attachment.
