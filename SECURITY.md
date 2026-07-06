# Security

`beatbox` treats generated code as hostile. A successful escape, undeclared host
capability access, or out-of-policy network egress is a critical vulnerability.

## defended classes

- Filesystem exfiltration: deny ambient host filesystem access; expose only
  policy-declared mounts.
- Network exfiltration: deny raw egress by default; future egress must go
  through a logging proxy.
- Resource exhaustion: enforce wall time, fuel or CPU budget, memory, output,
  process, and disk ceilings where the selected lane supports them. The W0
  `wasm` lane bounds compute via `wall_ms` + `fuel` and host memory via
  `memory_bytes` (linear memory and tables share that budget); it cannot honor
  an independent `cpu_ms`, `pids`, or `disk_bytes` ceiling and so rejects a
  request that sets any of them to a non-default value rather than silently
  ignoring it.
- Persistence and lateral movement: deny writes outside the workspace and deny
  access to localhost, LAN, cloud metadata, launch agents, hooks, and host env.
- Browser automation: Beatbox does not currently claim a runnable browser
  sandbox. `/v1/browser/profiles` is authenticated control-plane metadata for
  integration planning; MCP exposes the same contract through
  `get_browser_profiles` with structured content. It reports no default browser
  profile and no available browser session level until a real implementation
  enforces fresh profiles, network suppression or allowlisting, credential
  isolation, teardown, and any stated encryption behavior in the production call
  path.

## current grades

| lane | Linux | macOS | status |
| --- | --- | --- | --- |
| `wasm` | prod-grade substrate | prod-grade substrate | implemented as an empty-linker Wasmtime lane with fuel, epoch interruption, and store limits. |
| `python-wasi`, `js-wasm` | planned prod-grade substrate | planned prod-grade substrate | not implemented yet. |
| `python-native`, `js-native`, `exec` | planned OS jail | planned dev-grade OS jail | not implemented yet. |

Browser sandbox profiles are not execution lanes yet. The cataloged levels
range from an explicitly non-sandboxed external-browser instrumentation mode to
planned ephemeral, network-suppressed, sealed-state, OS-isolated, and
remote-isolated profiles. Encryption is claimed only as future behavior unless
the profile response names the algorithm, key source, and plaintext lifetime.

## out of scope for v1

Hardware side channels, malicious host operating systems, and kernel zero-days
are not eliminated. The roadmap includes a microVM backend for stronger process
and kernel separation where hardware support is available.
