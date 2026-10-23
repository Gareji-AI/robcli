# Public charts, experience snapshots, music and commerce

Checked 2026-10-17. Version 0.5 adds anonymous GET-only discovery, separate from
Dashboard session operations and rbxoc API-key/OAuth workflows. These web
contracts can change; undocumented web endpoints are not a stable Open Cloud API.

## Sources and confidence

- [Official Top Playing Now announcement](https://devforum.roblox.com/t/introducing-top-playing-now-on-charts/3529809)
  describes the chart as based on concurrent players. The
  [public chart page](https://www.roblox.com/charts/top-playing-now) links the sort.
- [First-party GameCarousel JavaScript inspected from that page](https://js.rbxcdn.com/77327ea6bae354296d06154f2f8fe93148ee98a60c3ab8ae64aabf7d46551003-GameCarousel.js)
  defines `explore-api/v1/get-sorts` and `get-sort-content`, passing `sessionId`,
  `sortsPageToken`/`pageToken`, sort ID and device/country filters. It reads
  `sorts`, `header.sorts`, `games`, `nextSortsPageToken` and `nextPageToken`.
  Browser `withCredentials` settings are not proof that cookies are required:
  our chart read succeeded anonymously. We do not call its telemetry POSTs,
  recommendation APIs or newer SDUI/protobuf chart endpoints as fallbacks.
- [Official consolidated OpenAPI](https://create.roblox.com/docs/cloud/openapi.json)
  documents anonymous alternatives for `games.roblox.com/v1/games` and
  `/v1/games/votes`. Details use Universe IDs, `playing`, `visits`,
  `favoritedCount`, root Place ID, creator, genre and timestamps. Votes use
  `id`, `upVotes`, `downVotes`.
- [Official Music Top Songs announcement](https://devforum.roblox.com/t/top-songs-available-on-your-experience-detail-page/3862496/1)
  says the experience section includes licensed/ISRC tracks, ranked by playtime
  over the last 7 days. [Roblox staff's endpoint reply](https://devforum.roblox.com/t/top-songs-available-on-your-experience-detail-page/3862496/35)
  gives `/music-discovery/v1/experience-songs?universeId=...&limit=...`.
- [Music Top 100](https://music.roblox.com/charts) is the official music chart,
  distinct from Creator Store relevance/trending. The page's DNS lookup failed
  during research; we did not claim to render it or switch hosts to work around
  that. The first-party `music-discovery/v1/top-songs` API itself returned 10
  tracks anonymously. Metadata schema/order is observed, not an Open Cloud SLA.
- [Official product endpoint migration, post 101](https://devforum.roblox.com/t/official-list-of-deprecated-web-endpoints/62889/101)
  replaces the old universe/store list with
  `/developer-products/v2/universes/{id}/developerproducts`. It retires the
  internal `DeveloperProductId` in favor of `ProductId`/`productId`.
- [Official pass endpoint migration, post 103](https://devforum.roblox.com/t/official-list-of-deprecated-web-endpoints/62889/103)
  replaces `games.roblox.com/v1/games/{id}/game-passes` with
  `/game-passes/v1/universes/{id}/game-passes`, `passView=Full`, `pageSize`,
  `pageToken`, `gamePasses` and `nextPageToken`.
- Public Place-to-Universe resolution uses the narrowly allowlisted
  `apis.roblox.com/universes/v1/places/{placeId}/universe` response `universeId`.
  This web contract is **not claimed as a documented stable Open Cloud resource**
  or live-verified in this increment. Its ID handling and failure behavior are
  covered by mocks. The user may supply a chart's Universe ID directly instead.

The creator-management `/creator` APIs are different authenticated resources;
they are not alternate paths for these public lists. No private product/pass
configuration or sales/revenue is requested.

## Command and request boundaries

| Command | Fixed GET | Requests with Universe ID |
|---|---|---:|
| `charts sorts` | `apis.roblox.com/explore-api/v1/get-sorts` | 1 |
| `charts games` | `apis.roblox.com/explore-api/v1/get-sort-content` | 1 |
| `charts music` | `apis.roblox.com/music-discovery/v1/top-songs` | 1 |
| `games get` | `games.roblox.com/v1/games`, `/v1/games/votes` | 2 |
| `games music` | `apis.roblox.com/music-discovery/v1/experience-songs` | 1 |
| `games passes` | `apis.roblox.com/game-passes/v1/universes/{id}/game-passes` | 1 |
| `games products` | `apis.roblox.com/developer-products/v2/universes/{id}/developerproducts` | 1 |

`games` also accepts alias `places`. Exactly one positive signed-64-bit-range
`--universe` or `--place` is required. A Place adds one resolution request, then
the returned Universe is used; it need not equal that Universe's root Place.
Returned detail/vote IDs and product Universe IDs, when present, must match.

All requests reserve the existing shared budget **before sending**, using the
same OS-local state and exclusive cross-process lock. Five attempts per run and
rolling 60 seconds, minimum two-second reservations, 20-second timeout and
two-MiB response cap. Failures count. No redirects, environment proxy, retry,
CSRF exchange, authentication/endpoint fallback or challenge solving.

Inputs are validated before state/credential lookup. `--profile` is rejected,
not ignored. The dedicated request constructor holds no Cookie, Authorization,
API-key or CSRF header/body and cannot address private/user-specific/purchase
routes. Conversely, session-backed constructors cannot address these routes.
Chart `sessionId` is a new public correlation identifier, not a Roblox login
session or copied browser token; no browser state is consulted.

## Output preserves scope and unknowns

Charts retain server order and `positionInPage`, not a guessed global rank.
Device/country filters are explicit (default computer/all). The server controls
chart page size, so output caps at 1–30 with `returnedByApi`, `truncatedLocally`,
`omittedFromPage`, `nextPageToken` and `hasMorePages`. A missing server token is
not fabricated, even if local truncation omitted records. Sort catalog output
is capped at 30. No chart row triggers a detail/music/commerce request.

Current player counts are **Universe/experience snapshots**, not one subplace,
server roster, player identities or historical series. The completion timestamp
is local observation time, not a claim about backend refresh latency. Like rate
is rounded to two decimals from `100 * up / (up + down)`; unknown or zero totals
yield null. Checked integer addition rejects overflow. No polling or joins over
every chart result.

Music returns public asset IDs, titles, artist/album, duration and cover asset ID
when supplied. Source/stream URLs are not followed or exposed. Store page links
are generated from validated IDs. `usagePermissionVerified=false`,
`allBgmEnumerated=false`, `liveNowPlaying=false`, `playbackPerformed=false`.
An empty Top Songs list does not prove there is no BGM. No game place file,
script, model, private audio inventory or runtime is inspected.

Public product responses currently contain `developerProducts` with PascalCase
fields such as `ProductId`, `Name`, `PriceInRobux`, `UniverseId`; they also
contain a different legacy `DeveloperProductId`/`TargetId`. Output uses
`productId` from **ProductId only**, never the retired ID. Pass responses use
`gamePasses`, primary `id` and separate `productId`; output `passId` uses the
**pass id only**. Retired/internal IDs and `isOwned` are discarded. Unknown
fields are not copied wholesale.

Cards include public/display names, descriptions, icon IDs, reported
`priceInRobux`, `userBasePriceInRobux`, sale status and timestamps when supplied.
Missing prices stay null, not zero/free. Public anonymous prices are not verified
regional/personalized checkout quotes. Ownership, purchase, private configuration,
sales/revenue and rights are not verified or changed. No purchase/import/save,
asset/media download, game launch, asset code execution or account mutation.

## Verification performed without repeated live tests

Mock tests preceded live access. One manually authorized, sequential session
captured exactly five anonymous GET responses with the real shared traffic guard:

1. Top Playing Now: 97 public games returned; 10 projected locally, including
   Universe/root Place IDs, player counts and up/down votes.
2. Music Top 100: 10 songs, next token `1`.
3. The first chart experience's published Top Songs: 4 songs.
4. That experience's developer-product list: 10, explicit next cursor.
5. That experience's passes: 2, no next page token.

All returned HTTP 200 with no challenge. These are point-in-time counts, not
fixed expected catalog sizes. The five saved public responses were replayed
offline through the final projections (10/10/4/10/2 cards). Temporary capture
and replay test hooks were removed before release; normal tests cannot call
real Roblox. No additional live request, alternate endpoint probe, prior avatar
detail POST retry, Dashboard CSV attempt, login or key issuance followed.

Individual detail/vote endpoints, Place resolution, sort listing, other chart
filters/pages, unavailable/restricted targets and empty music cases have mocks
and/or published contracts, **not live verification in this increment**.
The separate Dashboard CSV export is still not a claimed live success. See
[verification](verification.md).

## Icons, thumbnails, and badges

These are separate anonymous commands. They are not folded into `games get`.

| CLI | Route | Bound |
|---|---|---|
| `games icon` | `GET thumbnails.roblox.com/v1/games/icons` | one Universe, sizes 50x50, 128x128, 150x150, 256x256, 512x512 |
| `games thumbnails` | `GET thumbnails.roblox.com/v1/games/multiget/thumbnails` | one Universe, `countPerUniverse` 1..=10, sizes 768x432, 576x324, 480x270, 384x216, 256x144 |
| `games badges` | `GET badges.roblox.com/v1/universes/{id}/badges` | one page, limit 10/25/50/100, sort Asc/Desc, optional opaque cursor |

Icon `targetId` is the Universe ID. `games thumbnails` reads the parent
`universeId`, then each nested `thumbnails[].targetId` as `thumbnailId`. That
nested ID is the thumbnail image, not the Universe, and it is not copied into
`targetId`. `format` is `Png` or `Jpeg`. `isCircular=false` is fixed. `--count`
is sent as the published `countPerUniverse` query (integer, provider default 1),
not as a local `count` key. Only `Completed`
exposes an `https://*.rbxcdn.com` image URL. Pending, Blocked, Error, InReview,
and TemporarilyUnavailable set `imageReady` false and `imageUrl` null even if a
URL was returned. Image bytes are not fetched. An empty badge page is not proof
that the experience has no badges. Win rate is not completion or churn.
