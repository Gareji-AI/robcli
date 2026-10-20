# Avatar Marketplace search and experience-local appearance

Checked 2026-10-17. Scope: anonymous clothing/accessory/bundle search, typed IDs,
public page links and appearance-binding hints. No purchase, account inventory,
avatar/outfit save, actual try-on rendering, asset/source download or Studio IPC.

## Primary sources

- [Catalog v2 Swagger](https://catalog.roblox.com/docs/json/v2) defines public
  `GET /v2/search/items/details`, multi-valued `AssetTypeIds`/`BundleTypeIds`,
  Robux price filters, creator filters, sorting and cursor pagination.
- [Catalog v1 Swagger](https://catalog.roblox.com/docs/json/v1) defines item kinds
  (Asset=1, Bundle=2) and the experimental single/batch metadata POST.
- [Consolidated Roblox OpenAPI](https://create.roblox.com/docs/cloud/openapi.json)
  explicitly lists anonymous `{}` authentication alternatives for catalog reads.
- [Character appearance](https://create.roblox.com/docs/characters/appearance)
  documents `HumanoidDescription` changes to player and NPC characters, classic
  clothes and layered/rigid accessories.
- [HumanoidDescription source reference](https://raw.githubusercontent.com/Roblox/creator-docs/main/content/en-us/reference/engine/classes/HumanoidDescription.yaml)
  states that Shirt/Pants/GraphicTShirt expect the matching clothing asset type,
  **not a Decal/Image**, and documents GetAccessories/SetAccessories semantics.
- [AssetType enum](https://raw.githubusercontent.com/Roblox/creator-docs/main/content/en-us/reference/engine/enums/AssetType.yaml)
  and [AccessoryType enum](https://raw.githubusercontent.com/Roblox/creator-docs/main/content/en-us/reference/engine/enums/AccessoryType.yaml)
  establish the numeric catalog types and appearance accessory names.
- [Avatar Editor Service](https://create.roblox.com/docs/players/avatar-editor)
  explicitly says users do not receive catalog items they do not own when saving
  an avatar/outfit; inventory access, save prompts and purchases are separate flows.

## Supported public routes

Search: `GET https://catalog.roblox.com/v2/search/items/details`

Bundle details: `GET https://catalog.roblox.com/v1/bundles/{bundleId}/details`

The bundle route has no query. Item `type` stays `Asset`, `Bundle`, or
`UserOutfit`. Any other type is `unsupported` and is not coerced into an Asset
or used as a HumanoidDescription asset ID. Outfits are not expanded. The bundle
ID is not treated as an asset ID.

The constructor accepts only this host/path and an allowlist of published query
keys. Cookie, Authorization, API-key and CSRF headers are absent. Session-backed
request constructors cannot address the catalog path. Refusals/challenges/429/5xx
and redirects stop without token exchange, login, credential/endpoint fallback
or retry. The same single-process lock, five attempts/run and rolling minute,
two-second reservation spacing, timeouts and two-MiB response bound apply.

| CLI | API |
|---|---|
| `--type` | Repeated `AssetTypeIds` or `BundleTypeIds` query values, Swagger `collectionFormat=multi` |
| `--query` | `Keyword`, encoded as one value |
| `--creator-user` / `--creator-group` | `CreatorType=1/2` and `CreatorTargetId` |
| `--min-price` / `--max-price` / `--free` | `MinPrice` / `MaxPrice` / `MaxPrice=0`, in Robux |
| `--sort` | `SortType`: relevance 0, favorites 1, sales 2, updated 3, price-asc 4, price-desc 5 |
| `--period` | `SortAggregation`: day 1, week 3, month 4, all 5 |
| `--include-offsale` | `IncludeNotForSale` |
| `--limit` / `--cursor` | `limit` / `cursor`; CLI permits only 10, 28, 30, default 10 |

The API also documents 60/120 page sizes; this client deliberately caps at 30.
No automatic pagination, per-item fan-out or profile/identity query occurs.
Impossible price ranges, unsupported sizes and conflicting creators fail offline.

### Typed category mappings

- Classic T-shirt/Shirt/Pants: AssetType 2/11/12.
- Layered T-shirt/Shirt/Pants/Jacket/Sweater/Shorts: 64/65/66/67/68/69.
- LeftShoe/RightShoe/DressSkirt: 70/71/72.
- Hat/Hair/Face/Neck/Shoulder/Front/Back/Waist accessories: 8/41/42/43/44/45/46/47.
- `clothing` combines the classic and layered clothing types. `accessories`
  combines the listed rigid accessory types; layered clothing is separate.
- `bundles` requests the documented bundle type set 1/2/3/4. It does not turn a
  bundle ID into an Asset ID. `all` leaves the type filters absent.

Response cards normalize named/numeric Asset and Bundle tags, build only the
appropriate official `/catalog/{id}` or `/bundles/{id}` URL, and preserve the
reported Robux price/status and creator metadata. Unknown fields are discarded.
Missing prices are null, not zero/free. Ownership-related response fields are
not used to claim the anonymous caller owns anything. Bundle children retain
their declared types, including UserOutfit, and are not fetched or applied.
An invalid ID/kind or a result outside the requested asset filter stops rather
than fabricating an appearance. Offline links do not verify existence.

## What applying an ID does and does not mean

Within your own experience, supported catalog clothing/accessories can be used
with the normal appearance APIs for a character or preview rig. The character's
appearance and the Roblox account's ownership/saved avatar are different states.
This is not a universal promise that every ID loads on every rig or a license to
copy/redistribute files. Rig compatibility, availability, moderation, relevant
permissions and Roblox terms still apply.

Applying a description does not itself update the account avatar. Saving outfit
configuration or reapplying IDs in your own game also does not grant inventory
ownership. Official Avatar Editor documentation states that saving an
avatar/outfit does not grant unowned items. Purchase/inventory/save prompts are
not implemented by robcli.

Cards always report `ownershipVerified=false`, `previewEligibilityVerified=false`,
`appearanceApplied=false`, `accountAvatarChanged=false`. Appearance hints are
informational and explicitly `validatedInStudio=false`.

### Studio-side examples, not executed by this CLI

For a **Classic Shirt** catalog Asset ID, not an Image/Decal ID:

```lua
local function previewClassicShirt(humanoid, shirtAssetId)
    local description = humanoid:GetAppliedDescription()
    description.Shirt = shirtAssetId
    humanoid:ApplyDescriptionAsync(description)
end
```

For a **Layered Jacket**, merge with the current accessory list instead of
overwriting it. Choose a compatible avatar and handle load errors in your game:

```lua
local function previewJacket(humanoid, jacketAssetId)
    local description = humanoid:GetAppliedDescription()
    local accessories = description:GetAccessories(true)
    local order = 1
    for _, item in ipairs(accessories) do
        if item.AssetId == jacketAssetId then return end
        if item.Order then order = math.max(order, item.Order + 1) end
    end
    table.insert(accessories, {
        AssetId = jacketAssetId,
        AccessoryType = Enum.AccessoryType.Jacket,
        IsLayered = true,
        Order = order,
    })
    description:SetAccessories(accessories, true)
    humanoid:ApplyDescriptionAsync(description)
end
```

Rigid accessories must not be assigned an Order; `includeRigidAccessories=true`
allows them in GetAccessories/SetAccessories. These examples are documentation,
not Studio-tested runtime integration, and do not purchase or save an account look.

## Observed validation and excluded endpoint

An anonymous v2 Jacket/black search returned 10 items with AssetType 67, public
IDs, names and Robux prices. Other category mappings, classic-property hints,
bundle namespaces, unknown fields, pricing, no retry/CSRF fallback, shared state
without Firefox, profile rejection and offline behavior are tested synthetically.
No actual Studio preview or save/purchase was executed.

An experimental `POST /v1/catalog/items/details` with one typed asset returned
HTTP 403. Its cause was not inferred from hidden error content. No CSRF exchange,
Cookie/auth fallback, alternate endpoint probe or further live query followed.
That command/constructor was removed before release; **no `avatar get` is shipped**.
Detailed search responses satisfy the current discovery scope. The 403 does not
establish that experience-local appearance preview itself requires ownership.
The two live requests (one successful search, one refused experimental read) both
used no browser credential; failed attempts still count in the shared budget.
