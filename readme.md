# robcli

**Roblox Creator Dashboard from the terminal.** Project: `robcli`. Executable:
`rob`, following [gogcli / gog](https://github.com/openclaw/gogcli)'s naming and
resource-oriented command style. Implemented in Rust; no Node.js runtime required.

Version 0.2 adds explicit group selection, experience settings, one-chart CSV
export, and gated API-key issuance. It uses **your own Firefox login session**,
not API-key/OAuth authentication. Using generated keys belongs in
[rbxoc](https://github.com/eiei114/rbxoc). No authentication fallback joins the two.
This is an unofficial, early local tool, not full Dashboard coverage.

**Verification limit:** automated tests use fake accounts, mock HTTP/UI, fake
clocks and loopback only. The Rust implementation, private-browser CSV flow and
key issuance have **not been exercised against a real Roblox account**. UI and
private API contracts can change. See [contract evidence](docs/contracts.md).

## Build and discover commands offline

Requires Rust 1.99 or later. CSV export additionally requires Windows, Firefox and
Mozilla geckodriver; direct API commands do not require the driver.

```sh
cargo build --release --locked
cargo install --path . --locked
rob --version
rob --help
rob schema --json
```

Results are JSON envelopes: `ok`, `command`, `summary`, `data` or `error`, and
`hints`. Help/version are text. `--json` is accepted; JSON is already the default.
Without `--live`, account operations stop **before credential lookup**. Key
creation is an offline plan unless `--apply` is also supplied. The CLI never
starts a browser merely to print a link.

## Select a group and experience explicitly

| Command | Scope |
|---|---|
| `rob auth check --live` / `rob whoami --live` | Fresh identity check |
| `rob groups list --live` | Signed-in user's groups and roles; membership is not an admin grant |
| `rob experiences list --live` | Personal experiences, one page |
| `rob experiences list --group 42 --live` | Group-owned experiences, one page |
| `rob experiences settings --group 42 --universe 77 --live` | Verify the selected owner, then read configuration |
| `rob dashboard links --group 42 --universe 77` | Offline overview/settings/retention/credentials links |

IDs above are synthetic. These are **alternatives, not a batch script**; choose
one authorized operation at a time. `--group` is per-command selection, not a
persistent browser dropdown setting. Omit it for personal scope. Settings reads
stop if universe metadata does not match that owner; no automatic owner fallback.

Listing accepts `--archived` (archived instead of non-archived), `--limit` with
10/25/50/100 and `--all`. Without `--all`, `hasMore: true` means incomplete.
With `--all`, an exhausted budget or repeated cursor is an error, not a completed
listing. Every live command first consumes one request for identity verification.

`--profile <name>` chooses an existing Firefox profile by configured name. The
default comes from Firefox settings; ambiguous defaults stop, rather than trying
every profile. Raw Cookie, API-key, OAuth and arbitrary endpoint flags are refused.

## Download one chart through its existing CSV action

The implementation opens an **owned, new private Firefox**, injects only the
selected Roblox session in memory, visits one exact analytics URL and uses the
chart menu's **Download CSV** action. It does not copy the user's Firefox profile
or guess an internal analytics-query payload. Private mode is checked before
cookie injection; inability to attest it stops the command.

Example shape, for a separately authorized future export:

```sh
rob analytics export \
  --url 'https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42' \
  --chart 'Day 1 retention' --output retention-new.csv \
  --live --allow-browser-page-load
```

Replace synthetic IDs and use the exact visible chart title. Accepted tabs:
retention, engagement, acquisition, demographics, economy, funnels, explore.
Only `https://create.roblox.com` with this experience analytics path and an optional
numeric `groupId` is allowed. Existing output files are never overwritten.

**Browser traffic exception:** normal page assets/XHR are outside the direct-API
five-request cap. This is why a separate `--allow-browser-page-load` is mandatory.
The flow allows one cookie-origin seed navigation, one analytics page navigation,
one chart-menu click and one CSV click. No reload, repeated export click, polling
of a metrics endpoint, or fallback API query. The browser's internal requests and
retries are not controlled by the direct HTTP client. Do not run this in a loop.

Charts use **fresh page defaults**. Date ranges, segments and filters from an
already-open Firefox tab are not copied; no custom filter-setting commands yet.
“No data for selected period”, disabled Download CSV, missing/ambiguous controls,
login or verification challenges stop the flow. No empty/fabricated CSV success.
Download wait: at most 20 seconds; one UTF-8 CSV, at most 20 MiB. CSV contents are
saved locally, not printed or uploaded. Treat analytics data as private and inspect
untrusted CSV values before opening them in spreadsheet software.

Driver lookup: `--driver <path>`, then `%LOCALAPPDATA%/robcli/tools/geckodriver.exe`,
then PATH. No automatic download/updater runs during commands. This Windows setup
has Mozilla geckodriver **0.37.1**, downloaded from its official GitHub release;
win64 ZIP SHA-256:

```text
dfed9315abe8d2fbc1b6161a2ee8002452e79cf05ee92fdc653a4e26bc35edd8
```

The driver needs `--allow-system-access` for one fixed private-mode attestation,
not arbitrary user scripts. Driver/browser logs are suppressed. A Windows
kill-on-close Job Object owns only this new process tree; existing Firefox tabs
are never attached to or terminated. The adapter's real UI behavior is unverified.
Geckodriver is a separate Mozilla tool under MPL-2.0; see its
[source and license](https://github.com/mozilla/geckodriver).

## Review a key plan before issuing anything

`rob apikey list [--group 42] --live` returns metadata only: ID, owner, name, enable
state and expiry. Secret values/previews are never printed. Its Dashboard endpoint
uses **POST for a logical read**. CSRF token validation may require the explicit
`--allow-csrf-handshake` flag: at most one 403+token replay per command. A 2FA,
CAPTCHA or other verification challenge is never replayed or bypassed.

Create a **metadata-only** `key-plan.json`. This template deliberately requires
editing: replace the expiry with a future RFC3339 instant within 30 days, the
documentation IP with your explicit allowed egress CIDR, and the synthetic target
with your permitted resource. Verify scope names against Roblox documentation.

```json
{
  "name": "analytics-read",
  "description": "Narrow read access",
  "isEnabled": false,
  "expirationTime": "REPLACE_WITH_FUTURE_RFC3339_WITHIN_30_DAYS",
  "allowedCidrs": ["203.0.113.7/32"],
  "scopes": [{
    "scopeType": "universe-analytics",
    "targetParts": ["777"],
    "operations": ["universe.analytics:read"]
  }]
}
```

Unknown fields/raw-credential fields, wildcard scopes/targets, empty CIDR lists,
`/0` and missing/distant expiry are rejected. Omitted `isEnabled` defaults to
false. Offline preview reads only the plan, not browser credentials:

```sh
rob apikey create --request key-plan.json --secret-name analytics-read
```

**Issuance changes the account.** Only after reviewing the plan and intended
account, use `--apply --live --confirm-user <YOUR_USER_ID>`. Add
`--allow-csrf-handshake` only if you consent to the single token-validation
exchange. The fresh identity must match. This issues one personal-account-owned
key; group-owned key creation is not inferred. Explicit scopes can target group
resources when permitted. Enable, update, revoke and expiry repair are not added.

The secret is written once to `%LOCALAPPDATA%/robcli/keys/<secret-name>.key`
(equivalent OS-local location elsewhere), raw key plus newline, suitable for an
API-key-file consumer such as rbxoc. It is never stdout/JSON/log output, nor saved
in Vault/Git. Existing files, Windows device names and any Git ancestor are
refused. Unix uses directory 0700/file 0600; Windows uses the per-user local-data
directory's inherited ACL. **This is a plaintext file, not Credential Manager or
encryption**; keep its directory private and out of sync/backup sharing.

The receipt contains only metadata/path, server-reported enable/expiry state and
whether expiry matches the request. Missing server expiry is not silently repaired.
Verify owner, scopes, CIDRs, enable state and expiry in Dashboard before use.

**Unknown outcome:** network errors, redirects, 408/429/5xx or malformed success
can mean creation succeeded. No retry. An empty reservation remains; a failed
local secret write may leave a partial file. Stop and reconcile in Dashboard.
Do not delete the marker, choose another filename and resubmit to force progress.
No secret is printed as a fallback and no automatic revoke is attempted.

## Keep development offline and traffic bounded

- One cross-process live lock; residual locks are not automatically taken over.
- Direct API: at most five attempts per run and five per rolling 60 seconds across
  invocations, with at least two seconds between reservations. Failed attempts and
  the optional CSRF replay count. This is a conservative local limit, not Roblox's
  published quota or a guarantee that an operation is permitted.
- Fixed HTTPS API routes only on users/develop/groups/apis.roblox.com. No redirect
  following, environment proxy, automatic HTTP retry, credential/account rotation
  or refusal bypass. Browser page traffic is the explicitly documented exception.
- Cookie DB is queried read-only for `.ROBLOSECURITY` only. No DB/profile copies,
  cookie export, console secret, or stored session cookie. A session has broad
  authority; use only accounts/resources you are authorized to administer.
- `%LOCALAPPDATA%/robcli/traffic` stores only non-secret lock/timestamp state.
  Key files are separate. Corrupt state, backward clock, full budget or refusal
  stops the run. Do not erase shared state to work around a refusal.
- Development tests must not discover real browser profiles or hit real accounts.
  No load tests, continuous polling or bulk admin operations against Roblox.

Settings modification, uploads, publishing, purchases and general group/permission
administration are not implemented. No public repository/remote, public license,
registry publication or release workflow has been selected; `publish = false`.

## Run local verification

```sh
cargo fmt --check
cargo clippy --offline --locked --all-targets -- -D warnings
cargo test --offline --locked
```

With cached dependencies these run offline. Windows is the verified build/test
platform; macOS/Linux execution is unverified, and browser export rejects those
platforms. Contract drift or unsupported UI should produce a stop, not a live
selector/API probing loop. Record a sanitized fixture before changing adapters.
