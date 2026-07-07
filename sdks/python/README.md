# beatbox — Python SDK

Zero-dependency Python client for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox REST API. Standard library only (`urllib` + `json`); works on Python 3.9+.

This is one of a 7-language SDK fleet (TypeScript, Python, Go, Java, Ruby, PHP,
C#) that all implement the same contract, so an agent that knows one knows them
all.

## Install

From the SDK directory:

```bash
pip install .
```

Or add the `sdks/python` directory to your `PYTHONPATH`. There are no runtime
dependencies.

## Quickstart

Run a `wasm_wat` "add one" module and read the result value:

```python
import os
from beatbox import Client, ExecuteRequest

client = Client(
    base_url="http://127.0.0.1:7300",
    api_key=os.environ.get("BEATBOX_API_KEY"),
)

result = client.execute(ExecuteRequest.wasm_wat(
    '(module (func (export "run") (param i64) (result i64)'
    ' local.get 0 i64.const 1 i64.add))',
    input={"n": 41},
))

print(result.status)          # ExecutionStatus.OK
print(result.value)           # 42
assert result.value == 42
```

A runnable version lives in [`examples/add_one.py`](./examples/add_one.py):

```bash
BEATBOX_API_KEY=... python examples/add_one.py
```

## Authentication

Pass `api_key` to the `Client`. When set, it is sent as the `x-beatbox-api-key`
header on every request **except** `health()` and `openapi()`, which are
unauthenticated. The key is never included in any error message. Redirects are
never followed, so the key cannot leak cross-origin.

```python
client = Client("http://127.0.0.1:7300", api_key="bbx-api-key-placeholder", timeout=65.0)
```

## API

| Method | HTTP | Auth | Returns |
| --- | --- | --- | --- |
| `client.health()` | `GET /v1/health` | no | `dict` |
| `client.capabilities()` | `GET /v1/capabilities` | yes | `dict` |
| `client.browser_profiles()` | `GET /v1/browser/profiles` | yes | `dict` |
| `client.browser_admit(request)` | `POST /v1/browser/admit` | yes | `dict` |
| `client.browser_adapter_contract()` | `GET /v1/browser/adapter/contract` | yes | `dict` |
| `client.browser_adapter_capability(request)` | `POST /v1/browser/adapter/capability` | yes | `dict` |
| `client.browser_adapter_register(request)` | `POST /v1/browser/adapter/register` | yes | `dict` |
| `client.browser_adapter_launch_plan(request)` | `POST /v1/browser/adapter/launch/plan` | yes | `dict` |
| `client.browser_adapter_validate(request)` | `POST /v1/browser/adapter/validate` | yes | `dict` |
| `client.browser_adapter_completion_validate(request)` | `POST /v1/browser/adapter/completion/validate` | yes | `dict` |
| `client.execute(request)` | `POST /v1/execute` | yes | `ExecutionResult` |
| `client.create_job(request)` | `POST /v1/jobs` | yes | `CreateJobResponse` |
| `client.get_job(job_id)` | `GET /v1/jobs/{id}` | yes | `JobRecord` |
| `client.cancel_job(job_id)` | `DELETE /v1/jobs/{id}` | yes | `None` |
| `client.openapi()` | `GET /openapi.json` | no | `dict` |

Job ids are percent-encoded as a single path segment; `""`, `"."` and `".."`
are rejected (they could retarget the request).

### Building requests

`ExecuteRequest` mirrors the wire schema. Ergonomic constructors cover the 90%
case in one line, and `Source` has a classmethod per variant:

```python
from beatbox import ExecuteRequest, Source, Policy, Limits, Lane

# One-liners:
ExecuteRequest.wasm_wat("(module ...)", input={"n": 10})
ExecuteRequest.wasm_bytes_base64("AGFzbQ...", entrypoint="run")

# Full control:
ExecuteRequest(
    lane=Lane.WASM,
    source=Source.wasm_wat("(module ...)"),
    entrypoint="run",
    input={"n": 10},
    policy=Policy(limits=Limits(wall_ms=5000, fuel=1_000_000)),
    idempotency_key="step-1",
)
```

`Policy` and `Limits` are partial: only the fields you set are sent, and the
daemon merges them onto its defaults.

### Async jobs

```python
created = client.create_job(ExecuteRequest.wasm_wat("(module ...)"))
job = client.get_job(created.job_id)
print(job.status)             # JobStatus.QUEUED / RUNNING / SUCCEEDED / ...
if job.result is not None:
    print(job.result.value)
client.cancel_job(created.job_id)
```

## Error handling

Every non-2xx response raises `BeatboxApiError` with `status`, `code`, and
`message`. Any transport failure (connection, DNS, timeout, malformed body)
raises `BeatboxTransportError`. Both derive from `BeatboxError`.

```python
from beatbox import BeatboxApiError, BeatboxTransportError

try:
    result = client.execute(ExecuteRequest.wasm_wat("(module ...)"))
except BeatboxApiError as exc:
    print(exc.status, exc.code, exc.message)   # e.g. 422 bad_source "..."
except BeatboxTransportError as exc:
    print("could not reach daemon:", exc.message)
```

## Forward compatibility

Unknown/extra JSON fields are ignored on deserialization, and unknown enum
values are preserved as their raw string, so newer daemons remain compatible
with older SDK builds.

## Development

Run the unit tests (no live daemon required) from this directory:

```bash
python3 -m unittest discover
```

## License

Apache-2.0.
