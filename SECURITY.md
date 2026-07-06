# Security

`beatbox` treats generated code as hostile. A successful escape, undeclared host
capability access, or out-of-policy network egress is a critical vulnerability.

## defended classes

- Filesystem exfiltration: deny ambient host filesystem access; expose only
  policy-declared mounts.
- Network exfiltration: deny raw egress by default; future egress must go
  through a logging proxy.
- Resource exhaustion: enforce wall time, fuel or CPU budget, memory, output,
  process, and disk ceilings where the selected lane supports them.
- Persistence and lateral movement: deny writes outside the workspace and deny
  access to localhost, LAN, cloud metadata, launch agents, hooks, and host env.
- Control-plane access: `beatboxd` requires `BEATBOX_API_KEY` by default. The
  explicit `--allow-unauthenticated` override is for isolated local tests only;
  it is accepted only on loopback bind addresses, and loopback binding is not
  treated as authentication. MCP requests, including `GET /mcp`, `initialize`,
  and `ping`, are authenticated before request-body parsing or method dispatch
  when this key is required. Empty or whitespace-only configured API keys fail
  closed, including when the server or client APIs are used directly.
- Operational metadata such as `/openapi.json` and `/v1/capabilities` is
  authenticated with the same control-plane key; `/v1/health` is the static
  public liveness exception.
- Browser and local control-plane boundary: REST and MCP requests reject
  malformed or non-local `Host`, absolute request-target authority, and browser
  `Origin` values before request parsing or dispatch. This is a CSRF and
  rebinding boundary, not authentication. Duplicate `Host`, `Origin`,
  `Content-Type`, or credential headers are rejected instead of choosing one
  value and risking parser or proxy disagreement.
- API-key transport: clients do not attach `x-beatbox-api-key` to arbitrary
  `http://` base URLs. Key-bearing requests require HTTPS unless the caller
  uses the explicit loopback-HTTP opt-in with a literal loopback IP for local
  daemon testing, and clients disable ambient proxy configuration for
  control-plane requests. Client endpoint URLs are built from parsed base URLs,
  reject query or fragment components, and percent-encode dynamic path segments
  before attaching an API key. Successful client responses must advertise a JSON
  media type, empty-success client operations require the documented no-content
  status, and response bodies are capped while reading so a malicious or
  compromised peer cannot force unbounded buffering.

## current grades

| lane | Linux | macOS | status |
| --- | --- | --- | --- |
| `wasm` | prod-grade substrate | prod-grade substrate | implemented as an empty-linker Wasmtime lane with pre-compile import scanning, fuel, epoch interruption, a fixed pre-compile `max_wasm_module_bytes` cap for source/module bytes, and one aggregate store limit covering linear memory plus table elements. |
| `python-wasi`, `js-wasm` | planned prod-grade substrate | planned prod-grade substrate | not implemented yet. |
| `python-native` | planned OS jail | experimental dev-grade Seatbelt lane | unavailable by default; requires `lane-python`, `sandbox-exec`, and `python3`. It executes only regular-file Python binaries that resolve to known Homebrew or Command Line Tools Python runtime paths, refuses arbitrary executables merely under `/usr/local` or `/opt/homebrew`, allows only Python runtime reads plus a randomly named, exclusively created private workspace, enforces wall time, output, `max_python_source_bytes`, and a polling private-workspace disk quota, ties source delivery to the wall/cancel watchdog, rejects non-default CPU-time, process-memory, pid, and fuel limits, denies network, broad Mach service lookup, broad sysctl reads, host env, process fork, secrets, and policy mounts, and reports downgrades for default CPU and memory ceilings that are not enforced. |
| `js-native`, `exec` | planned OS jail | planned dev-grade OS jail | not implemented yet. |

Remote control-plane submission fails closed against `/v1/capabilities`: lanes
that are not reported available are rejected before synchronous worker execution
or asynchronous job queueing.

Wasm source bytes are bounded before expensive parsing or compilation. Local
file sources, including the CLI compile helper, are opened and read through a
capped reader so a mutable or misreported file cannot be fully materialized
before the source limit fires.

Lane request ABIs fail closed. A lane must reject request fields it does not
currently expose, such as `stdin`, structured `input`, or `entrypoint`, rather
than accepting and silently ignoring caller-provided data. Remote policy
admission rejects unenforceable fields before worker permits, job queueing, or
job request persistence. Unknown REST wire fields are rejected during JSON
parsing and the OpenAPI request schemas advertise `additionalProperties=false`,
so misspelled or future-looking policy fields cannot be silently ignored.
Omitted policy and nested limit fields are filled from the runtime defaults
reported by `/v1/capabilities`, preserving compact clients without weakening
the unknown-field gate.

Stateful async job history is bounded separately from per-request body and
worker concurrency limits. `/v1/jobs` rejects new records after
`max_stored_jobs` is reached while allowing idempotent retries that reuse an
existing record. `beatboxd` creates the SQLite job store as a private file,
hardens SQLite sidecar permissions, and rejects symlink, non-regular, or SQLite
URI DB paths before creating or opening the store so persisted requests and
results are not exposed through permissive defaults. The shared `JobStore::open`
path also rejects SQLite URI filenames, creates or
hardens the primary DB file and SQLite sidecars as private local state, rejects
sidecar symlinks, and opens literal DB files with SQLite no-follow semantics.

Synchronous worker limits survive client disconnects. REST and MCP sync
executions hold their semaphore permit inside the blocking worker and signal the
lane cancellation token when the request future is dropped, so abandoned
requests cannot free the control-plane permit while CPU or subprocess work keeps
running.

## out of scope for v1

Hardware side channels, malicious host operating systems, and kernel zero-days
are not eliminated. The roadmap includes a microVM backend for stronger process
and kernel separation where hardware support is available.
