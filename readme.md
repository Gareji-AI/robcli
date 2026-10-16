# robcli

**Roblox Creator Dashboard from the terminal.** The project is `robcli`; its
executable is `rob`, following the naming and resource-oriented command style of
[gogcli / gog](https://github.com/openclaw/gogcli).

This is an early **read-only Rust MVP**, not an official Roblox product. It uses
your own existing Firefox session. It is a separate product from
[rbxoc](https://github.com/eiei114/rbxoc): API keys and OAuth belong there, not here.
There is no fallback from a rejected API key to a full browser session.

## Build and use

Requires Rust 1.99 or later. No Node.js or browser extension is required.

```sh
cargo build --release --locked
cargo install --path . --locked

rob --help
rob schema --json
```

The commands above are offline. Account commands also refuse by default, before
reading any browser credentials:

```sh
rob auth check
# JSON error: live_not_enabled
```

Only for one deliberately approved, bounded read-only smoke test, choose a command
and explicitly enable live access:

```sh
rob auth check --live
# or: rob whoami --live
# or: rob experiences list --all --live --json
```

These are alternatives, not a script to run repeatedly. `--profile <name>` selects
one existing Firefox profile by its configured name. The default comes from
Firefox's install/profile settings, never from scanning every profile for cookies.

All command results use a JSON envelope: `ok`, `command`, `summary`, `data` or
`error`, and `hints`. Help and version are normal text. `--json` is accepted for
explicit script intent; JSON is already the default.

## What works

| Command | Purpose |
|---|---|
| `rob schema` | Offline discovery of commands, authentication, and safety limits |
| `rob auth check --live` | Verify the signed-in Firefox Roblox account |
| `rob whoami --live` | Alias of the identity check |
| `rob experiences list --live` | One page of non-archived personal experiences |
| `rob experiences list --all --live` | Follow pages, within the same hard request budget |

Listing accepts `--archived` (archived **instead of** non-archived) and `--limit`
with `10`, `25`, `50`, or `100` results per page. Without `--all`, `hasMore: true`
means the listing is incomplete. With `--all`, a repeated cursor or exhausted
budget is an error, never a claim that all resources were fetched.

Group-owned experiences, settings, uploads, publishing, analytics, purchases,
permissions, and all other writes are not implemented. Broader Dashboard coverage
is a future goal, not a promise that every UI action has an accessible API.

## Safety is part of the MVP

- **Offline by default.** No credential lookup or network request without `--live`.
- **Read-only destinations.** Only GET requests to `users.roblox.com` and
  `develop.roblox.com`, through fixed command routes. No arbitrary URL or endpoint
  overrides. HTTPS only; redirects and environment proxies are disabled.
- **Single live probe.** A cross-process lock is held for the whole command.
  Existing or residual locks are not automatically taken over.
- **Bounded traffic.** At most five attempts per command and five per rolling
  60 seconds across invocations; at least two seconds between request reservations.
  Identity verification consumes one request. Failed attempts consume budget too.
- **No automatic retries.** HTTP failures, challenges, rate limits, redirects,
  malformed responses, or transport failures stop the run. There is no account,
  credential, or proxy rotation to bypass a refusal.
- **Fail-closed state.** Invalid state, backward clock movement, or state-write
  failure stops the probe. Do not delete locks/state or change directories to
  bypass a stop. No user-facing option increases these limits.
- **No secret export.** The original Firefox SQLite database is opened read-only.
  Only the unexpired, non-container `.ROBLOSECURITY` cookie for `roblox.com` is
  queried. The database is not copied. The cookie is kept in process memory,
  redacted in debug headers, and never stored in output or traffic state.

Traffic state contains only timestamps and a local lock PID. On Windows it lives
under `%LOCALAPPDATA%/robcli/traffic`, on macOS under
`~/Library/Application Support/robcli/traffic`, and on Linux under
`$XDG_STATE_HOME/robcli/traffic` (or `~/.local/state/robcli/traffic`).

These conservative development limits are **not** Roblox's published rate limits
or a guarantee of compliance. Stricter endpoint limits and server refusals always
take precedence. Cookie-based legacy APIs can change without notice; Roblox
recommends API keys or OAuth when possible. Authentication method alone does not
determine legal or terms-of-service compliance. Use only your own authorized
account; never bypass 2FA, challenges, permissions, or rate limits.

## Test without contacting Roblox

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Tests use synthetic cookie databases, fake profile directories, mock transports,
and a fake clock. One HTTP integration test uses loopback only to prove that a
redirect does not forward the synthetic cookie. Tests never use the developer's
real Roblox session. Do not add live integration tests, scraping loops, load tests,
or CI browser-credential access.

Windows is the initial development host. macOS/Linux profile and state paths are
implemented but have not been verified on those operating systems.

## Source contracts

- [Roblox OpenAPI and authentication definitions](https://create.roblox.com/docs/cloud/openapi.json)
- [Users API: authenticated account](https://users.roblox.com/docs/json/v1)
- [Develop API: current user's universe collection](https://develop.roblox.com/docs/json/v1)

No public repository, crate publication, live writes, or automatic updater is
created by building this project. `publish = false` prevents accidental crates.io
publication. The public license and distribution workflow are not selected yet.
