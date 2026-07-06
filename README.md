# beatbox

`beatbox` is a standalone sandbox service for running untrusted,
agent-generated code behind explicit capabilities. It is designed to run on its
own through a CLI, daemon, REST API, and MCP endpoint, then plug into sibling
Beater projects over protocol boundaries.

## Quickstart

Run the local CLI against a hermetic Wasmtime lane:

```sh
cargo run -p beatbox -- run examples/fib.wasm --input '{"n":10}'
```

Start the daemon:

```sh
cargo run -p beatboxd -- --addr 127.0.0.1:7300
```

Call the REST API with the example request body (computes `fib(10) = 55`):

```sh
curl -sS http://127.0.0.1:7300/v1/execute \
  -H 'content-type: application/json' \
  -d @examples/req-fib.json
```

Remote requests upload the module as inline WAT or base64 Wasm bytes;
daemon-local `wasm_file` paths are rejected by design. Limits may be given
partially (unspecified fields fall back to defaults), and unknown policy keys
are rejected rather than silently ignored.

Run tests:

```sh
cargo test
```

The current implementation covers the first standalone path: core wire types, a
Wasmtime-backed `wasm` lane, a local CLI, a daemon router, a typed HTTP client,
OpenAPI JSON, MCP tools, and rusqlite-backed async jobs. Native Python, JS,
exec jails, stateful sessions, and the `beater.js` integration remain later
milestones.

Browser automation is exposed only as an explicit discovery contract today, not
as a runnable browser lane. Authenticated callers can read
`GET /v1/browser/profiles`, call the MCP `get_browser_profiles` tool, or inspect
the `browser_sandbox` section of `/v1/capabilities` to see the profile levels
Beatbox intends to support for Tempo-style integrations: external
instrumentation, ephemeral profiles, network-suppressed browsing, sealed
persisted state, OS-isolated browsing, and remote isolated workers. The response
deliberately reports
`runnable_browser_sessions: false`, serializes `default_level` as `null`, and
marks every profile as `planned` or `unavailable` until a real browser launcher,
egress boundary, storage policy, and teardown path enforce the claim. Call
`POST /v1/browser/admit` or the MCP `admit_browser_session` tool before starting
browser work; callers can include `required_controls` such as `fresh_profile`,
`egress_policy`, `local_network_block`, `sealed_artifacts`, or OS/remote
isolation controls. They can also declare `target_origins`,
`credential_mode`, and `artifact_mode` so Tempo can bind a user or agent's
intent to an origin allowlist, credential posture, and persistence posture
before any browser starts. Target origins must be public HTTP(S) origins only:
paths, credentials, localhost, private/LAN addresses, and link-local metadata
targets are rejected at preflight. Admission responses include a `guard_plan`
that spells out the network, credential, storage, DNS/redirect revalidation,
and runtime guards a future browser adapter must enforce before the request can
become runnable. The current implementation always rejects admission and
explains which production pieces or requested controls are still missing.

## Ecosystem

beatbox is part of the [ecosystem](https://github.com/jadenfix/ecosystem) — a family of Rust-first, local-first agent-infrastructure projects. It is fully standalone by design: the CLI, daemon, REST API, and MCP endpoint run on their own, and sibling integrations should plug in only over those protocol boundaries. Planned connection points include:

- the sandboxed-execution lane for [beater.js](https://github.com/jadenfix/beater.js) untrusted code, [tempo](https://github.com/jadenfix/tempo) tool execution, and [beaterOS](https://github.com/jadenfix/beaterOS) auditable side effects
