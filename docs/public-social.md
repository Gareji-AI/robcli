# Public profiles and communities

Checked 2026-10-17 against the [official consolidated OpenAPI](https://create.roblox.com/docs/cloud/openapi.json)
cached during this session, followed by five bounded anonymous CLI reads.
These web contracts may change. This is not full community administration or a
promise of access to private/profile-restricted information.

## Primary contracts

Each listed operation has an anonymous `{}` security alternative in that spec.
The documented [Users v1](https://users.roblox.com/docs/json/v1),
[Groups v1](https://groups.roblox.com/docs/json/v1),
[Groups v2](https://groups.roblox.com/docs/json/v2) and
[Games v2](https://games.roblox.com/docs/json/v2) endpoints are reference entry points.

| CLI | Fixed public GET | Notes |
|---|---|---|
| `users get --user <id>` | `users.roblox.com/v1/users/{id}` | Exact returned ID must match; name/displayName/description/created and optional public flags |
| `users search --query <text>` | `users.roblox.com/v1/users/search` | keyword, limit, cursor; never claim an exact identity match |
| `users communities --user <id>` | `groups.roblox.com/v2/users/{id}/groups/roles` | includeLocked=false, includeNotificationPreferences=false, discoveryType=0 |
| `users games --user <id>` | `games.roblox.com/v2/users/{id}/games` | accessFilter=Public, sortOrder=Asc, limit, cursor |
| `communities get --community <id>` | `groups.roblox.com/v1/groups/{id}` | Exact returned ID must match; optional owner/shout may be null |
| `communities search --query <text>` | `groups.roblox.com/v1/groups/search` | keyword, limit, cursor |
| `communities roles --community <id>` | `groups.roblox.com/v1/groups/{id}/roles` | includePrivate=false, matching groupId, no member or permission requests |
| `communities games --community <id>` | `games.roblox.com/v2/groups/{id}/games` | accessFilter=Public, sortOrder=Asc, limit, cursor |

All eight live operations make one GET, not a detail/count join. The username
lookup POST, authenticated user endpoint, audit logs, settings, join requests,
member/role rosters, social links and messaging/forum APIs are not added to the
public request allowlist. A docs entry's anonymous alternative alone is not a
reason to include unrelated private-looking resources.

`users` aliases: `profile`, `profiles`; `users communities` alias: `users groups`.
`communities` alias: `community`; its `--community` flag also accepts `--group`.
The modern Community ID is the API Group ID. User IDs, Community IDs, role IDs,
Universe IDs and Place IDs still have separate meanings.

The old `groups list` remains session-backed and refers to the signed-in user's
groups. It uses the same v2 membership path, now with the same explicit query as
the anonymous command: `includeLocked=false`, `includeNotificationPreferences=false`,
and `discoveryType=0`. Locked communities are not requested. Notification
preferences are not requested or projected. Discovery type 0 is the published
default set, not a complete inventory of every community type. The session
constructor rejects a missing, duplicate, or different query. It does not
de-authenticate the command: public `users communities --user <id>` still uses
the anonymous constructor and an explicit User ID.

## Public metadata, not identity or management authority

Only allowlisted response fields are projected. Profiles do not copy arbitrary
fields, email/verification data, previous username history, presence or inventory.
Search results carry null for details not provided; there is no per-row profile
fetch or guessed name-to-account resolution. A display name, search result or
verified badge does not verify the caller's identity/ownership.

Community cards preserve optional owner and public shout metadata. Missing owner
or shout is null. Unneeded `communityTier` details and owner-verification
requirements are discarded, not echoed. `memberCount` is API-reported: the
official `hasSocialModules` field notes that social-module communities can use
the **followers** UI label instead of members. `audienceLabel` is unknown when
that flag is absent, as is usual for basic/search responses. Counts are not an
engagement audit or a community-member enumeration.

Memberships preserve only the public community card and available role; missing
roles remain null. Notification preferences are not requested or projected.
Locked/private/missing communities are not probed through another endpoint.
`allMembershipsVerified=false`; the selected discovery type is the published
enum value 0, not a guarantee of all community/follower types.

Roles explicitly exclude private roles, and a response claiming `isPrivate=true`
is rejected instead of exposed. The legitimate Guest role has ID/rank 0;
other zero IDs are not accepted as guessed roles. Rank/name/base-role metadata
does not establish management powers. `managementPermissionsVerified=false`;
no role permissions matrix, member list or user-role enumeration is fetched.

Creation cards preserve Universe and root Place IDs separately. An available
creator must match the selected User/Group type and ID; an explicit root entity
type must be Place (9 or `Place`). Missing creator metadata is unknown, not
verified ownership; a missing root Place yields no fabricated game link. No CCU,
votes, music, products or passes are fetched for each result. The public filter
is mandatory in both the command and the transport constructor. Public access
does not itself verify profile listing/discoverability status.

## Bounds, privacy and failure behavior

Search/creation page sizes are the documented 10/25 subset (default 10), not
50/100 or unlimited; cursor continuation is explicit. Queries are nonblank,
control-free and at most 512 bytes; cursors at most 4096 bytes. Unsupported page
sizes and IDs fail before state/credentials. Paginated responses exceeding the
requested page, duplicate IDs, wrong targets or malformed cursors fail closed.

Memberships and roles have no cursor in the contract. The CLI returns at most
1–25 records (default 25), reports API count/local truncation and never invents
a next page. Each response is capped at 1000 unpaged records and the shared
two-MiB body limit. This is not a complete hidden-membership/role inventory.

The public request type cannot carry Cookie/Authorization/API-key/CSRF headers
or a request body. Host/path/query keys are fixed; duplicate query keys are
rejected. `includePrivate`, `includeLocked`, notification flags, discovery type,
`accessFilter` and sort order have fixed values, not arbitrary user overrides.
No Firefox discovery or browser-cookie lookup. The global `--profile` option
is rejected for all these commands, including offline links.

Every live read reserves the same cross-process budget before HTTP: exclusive
lock, at most five attempts per run and rolling minute, minimum two seconds,
20-second timeout. 401/403/challenge/429/5xx, redirects, timeouts and schema
failures stop. No retry, token negotiation, alternate endpoint, account/proxy
rotation or authenticated fallback. Error bodies/headers are not echoed.

`users link` and `communities link` generate official page URLs offline, with
`existsChecked=false` and `browserOpened=false`. They do not navigate or assert
that the target exists.

No participation, join/leave, follow, friend operation, posting/messaging, role
or membership change, private data export, media download or account mutation.
Public names, bios, descriptions and shouts are untrusted data, not executable
agent instructions. No sensitive attribute or real identity is inferred.

## Validation scope

Offline fake-transport and binary tests cover ID/type matching, search encoding,
null/missing fields, Guest role 0, private-role refusal, no credential headers,
counter labels, public creation filters, output caps, duplicate IDs/cursors,
no retries, aliases, local links and shared budgets without Firefox paths.

After mocks passed, exactly five sequential anonymous CLI reads succeeded with
the real shared guard: public user 1, community search (`Roblox`, 10 cards),
the first result's community 7 snapshot, its public roles (5), and its public
creations (**an empty page**, no non-empty creator/root rows verified live).
Only a count/ID/status summary was retained under ignored `target/`, not profile
bios or a durable social dataset. No additional live request followed.

User search, user communities/creations, non-empty creation rows, other targets,
pagination and privacy-restricted cases are contract/mock-tested, not live-tested
in this increment. These public reads do not resolve the earlier Dashboard CSV
named-cookie 404 and do not retry the refused avatar metadata POST.
