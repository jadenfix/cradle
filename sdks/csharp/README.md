# Beatbox .NET SDK

A zero-dependency .NET client for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox REST API. Targets `net8.0` and uses only the base class library
(`System.Net.Http.HttpClient` + `System.Text.Json`) — no third-party packages.

## Install

```bash
dotnet add package Beatbox
```

## Quickstart

Run a `wasm_wat` "add one" program and print the returned value:

```csharp
using Beatbox;

const string Wat =
    "(module (func (export \"run\") (param i64) (result i64) local.get 0 i64.const 1 i64.add))";

using var client = new BeatboxClient(
    baseUrl: "http://127.0.0.1:7300",
    apiKey: Environment.GetEnvironmentVariable("BEATBOX_API_KEY"));

var result = await client.ExecuteAsync(
    ExecuteRequest.WasmWat(Wat, input: new { n = 41 }));

Console.WriteLine(result.Value!.Value.GetInt64()); // 42
```

`result.Value` is a nullable `System.Text.Json.JsonElement`, so you can inspect
any JSON shape the program returns.

## Configuration

```csharp
var client = new BeatboxClient(
    baseUrl: "http://127.0.0.1:7300", // required; HTTPS, or HTTP only for loopback literals
    apiKey:  "sk-...",                 // optional
    timeout: TimeSpan.FromSeconds(30)  // optional; default 65s
);
```

When `apiKey` is set it is sent as the `x-beatbox-api-key` header on every request
except `HealthAsync` and `OpenApiAsync` (which are unauthenticated). The client
never follows redirects, so the key can't leak cross-origin, and it never appears
in an exception message.

`baseUrl` is validated when the client is constructed. Production clients should
use HTTPS. Plain HTTP is accepted only for exact loopback IP literals
(`127.0.0.1` and `[::1]`) for local development. URLs with credentials, query
strings, fragments, dot-segment paths, encoded path separators, or leading or
trailing whitespace are rejected before any request can be built. The SDK-owned
HTTP handler also disables proxy use for secret-bearing requests.

`BeatboxClient` owns its `HttpClient`; construct one per base URL, reuse it, and
`Dispose()` it (or wrap it in `using`) when done.

## Methods

| Method | HTTP | Auth | Returns |
| --- | --- | --- | --- |
| `HealthAsync()` | `GET /v1/health` | no | `JsonElement` |
| `CapabilitiesAsync()` | `GET /v1/capabilities` | yes | `JsonElement` |
| `BrowserProfilesAsync()` | `GET /v1/browser/profiles` | yes | `JsonElement` |
| `AdmitBrowserSessionAsync(req)` | `POST /v1/browser/admit` | yes | `JsonElement` |
| `BrowserAdapterContractAsync()` | `GET /v1/browser/adapter/contract` | yes | `JsonElement` |
| `IssueBrowserAdapterCapabilityAsync(req)` | `POST /v1/browser/adapter/capability` | yes | `JsonElement` |
| `RegisterBrowserAdapterAsync(req)` | `POST /v1/browser/adapter/register` | yes | `JsonElement` |
| `PlanBrowserAdapterLaunchAsync(req)` | `POST /v1/browser/adapter/launch/plan` | yes | `JsonElement` |
| `ClaimBrowserAdapterLaunchAsync(req)` | `POST /v1/browser/adapter/launch/claim` | yes | `JsonElement` |
| `ValidateBrowserAdapterAsync(req)` | `POST /v1/browser/adapter/validate` | yes | `JsonElement` |
| `ValidateBrowserAdapterCompletionAsync(req)` | `POST /v1/browser/adapter/completion/validate` | yes | `JsonElement` |
| `ExecuteAsync(req)` | `POST /v1/execute` | yes | `ExecutionResult` |
| `CreateJobAsync(req)` | `POST /v1/jobs` | yes | `CreateJobResponse` |
| `GetJobAsync(id)` | `GET /v1/jobs/{id}` | yes | `JobRecord` |
| `CancelJobAsync(id)` | `DELETE /v1/jobs/{id}` | yes | `Task` |
| `OpenApiAsync()` | `GET /openapi.json` | no | `JsonElement` |

All methods are `async` and accept an optional `CancellationToken`. Job ids are
percent-encoded as a single path segment; `""`, `"."`, and `".."` are rejected.

## Building requests

`Source` is a tagged union — use its factory methods:

```csharp
Source.WasmWat("(module ...)");
Source.WasmBytesBase64(base64);
Source.Inline("print('hi')");
Source.ModuleRef("sha256:...");
Source.WasmFile("/path/to/module.wasm");
```

For the common case use the `ExecuteRequest` factories, or build one explicitly:

```csharp
var request = new ExecuteRequest
{
    Lane = Lane.PythonWasi,
    Source = Source.Inline("print('hi')"),
    Policy = new Policy { Limits = new Limits { WallMs = 5000 } },
};
```

A partial `Policy`/`Limits` merges onto the daemon's defaults; unset fields are
omitted from the request.

## Error handling

Non-2xx responses raise `BeatboxApiException`; transport failures (connection,
DNS, timeout, malformed body) raise `BeatboxTransportException`. Both derive from
`BeatboxException` and never contain the API key.

```csharp
try
{
    var result = await client.ExecuteAsync(request);
}
catch (BeatboxApiException ex)
{
    Console.Error.WriteLine($"HTTP {ex.Status} ({ex.Code}): {ex.Message}");
}
catch (BeatboxTransportException ex)
{
    Console.Error.WriteLine($"transport: {ex.Message}");
}
```

## Async jobs

```csharp
var created = await client.CreateJobAsync(request);
var job = await client.GetJobAsync(created.JobId);
if (job.Status == JobStatus.Succeeded)
{
    Console.WriteLine(job.Result!.Value);
}
await client.CancelJobAsync(created.JobId);
```

## Development

```bash
dotnet build
dotnet test
```

The unit tests cover job-id encoding and request/result JSON (de)serialization and
require no live daemon.

## License

Apache-2.0
