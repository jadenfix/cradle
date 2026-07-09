# beatbox — Java SDK

Official Java SDK for the [beatbox](https://github.com/jadenfix/beatbox) sandbox REST API.
Run untrusted code (WebAssembly, Python, JS, ...) in an isolated daemon over HTTP.

- Java 17+
- HTTP via the JDK's `java.net.http.HttpClient` — no third-party HTTP client
- JSON via Jackson (`jackson-databind`), the SDK's only runtime dependency

## Install

Maven coordinate:

```xml
<dependency>
  <groupId>ai.beatbox</groupId>
  <artifactId>beatbox</artifactId>
  <version>0.1.0</version>
</dependency>
```

## Quickstart

Runs a one-instruction "add one" wasm module and prints the value (`42`):

```java
import ai.beatbox.BeatboxClient;
import ai.beatbox.model.ExecuteRequest;
import ai.beatbox.model.ExecutionResult;
import java.util.Map;

BeatboxClient client = BeatboxClient.builder()
        .baseUrl("http://127.0.0.1:7300")
        .token(System.getenv("CRADLE_TOKEN")) // optional
        .build();

String wat = "(module (func (export \"run\") (param i64) (result i64) "
        + "local.get 0 i64.const 1 i64.add))";

ExecutionResult result = client.execute(ExecuteRequest.wasmWat(wat, Map.of("n", 41)));

long value = ((Number) result.value()).longValue();
assert value == 42;                 // 41 + 1
System.out.println(result.value()); // 42
```

`value` is arbitrary JSON, exposed as `Object` — cast it as your program expects.

## Configuration

Build a client with the builder:

| Option      | Required | Default        | Notes |
| ----------- | -------- | -------------- | ----- |
| `baseUrl`   | yes      | —              | e.g. `http://127.0.0.1:7300`; trailing slashes trimmed |
| `token`     | no       | none           | sent as `Authorization: Bearer <token>` on every request except `health()` and `openapi()` |
| `apiKey`    | no       | none           | legacy compatibility alias sent as `x-beatbox-api-key` only when `token` is not set |
| `timeout`   | no       | 65 seconds     | per-request timeout |

`baseUrl` must be an absolute `https://` URL, or `http://127.0.0.1...` /
`http://[::1]...` for local development. Credentials, query strings,
fragments, and path prefixes with dot segments or encoded separators are
rejected before any request is built. Custom `HttpClient` instances must also
have redirects disabled, no proxy selector configured, and are accepted only
with `https://` base URLs.

Redirects are never followed, so the token header can't leak cross-origin.

## Methods

| Method                     | HTTP                     | Returns             |
| -------------------------- | ------------------------ | ------------------- |
| `health()`                 | `GET /v1/health`         | `JsonNode` (raw)    |
| `capabilities()`           | `GET /v1/capabilities`   | `JsonNode` (raw)    |
| `integration()`            | `GET /v1/integration`    | `JsonNode` (raw)    |
| `browserProfiles()`        | `GET /v1/browser/profiles` | `JsonNode` (raw)  |
| `browserAdmit(request)`    | `POST /v1/browser/admit` | `JsonNode` (raw)    |
| `browserAdapterContract()` | `GET /v1/browser/adapter/contract` | `JsonNode` (raw) |
| `issueBrowserAdapterCapability(request)` | `POST /v1/browser/adapter/capability` | `JsonNode` (raw) |
| `registerBrowserAdapter(request)` | `POST /v1/browser/adapter/register` | `JsonNode` (raw) |
| `planBrowserAdapterLaunch(request)` | `POST /v1/browser/adapter/launch/plan` | `JsonNode` (raw) |
| `claimBrowserAdapterLaunch(request)` | `POST /v1/browser/adapter/launch/claim` | `JsonNode` (raw) |
| `validateBrowserAdapter(request)` | `POST /v1/browser/adapter/validate` | `JsonNode` (raw) |
| `validateBrowserAdapterCompletion(request)` | `POST /v1/browser/adapter/completion/validate` | `JsonNode` (raw) |
| `execute(request)`         | `POST /v1/execute`       | `ExecutionResult`   |
| `createJob(request)`       | `POST /v1/jobs`          | `Operation`         |
| `getJob(id)`               | `GET /v1/jobs/{id}`      | `JobRecord`         |
| `cancelJob(id)`            | `DELETE /v1/jobs/{id}`   | `void`              |
| `openapi()`                | `GET /openapi.json`      | `JsonNode` (raw)    |

The job id is percent-encoded into a single path segment; `""`, `"."` and `".."` are rejected.

### Building requests

`Source` is a tagged union — use its factories:

```java
Source.wasmWat("(module ...)");
Source.wasmBytesBase64("AGFzbQ...");
Source.inline("print('hi')");
Source.wasmFile("/path/on/host.wasm");
Source.moduleRef("sha256:...");
```

Beyond the `ExecuteRequest.wasmWat(...)` shortcut, use the builder for full control:

```java
ExecuteRequest req = ExecuteRequest.builder(Lane.PYTHON_WASI, Source.inline("print(41 + 1)"))
        .entrypoint("run")
        .input(Map.of("n", 41))
        .policy(Policy.withLimits(Limits.wallMs(5000)))
        .idempotencyKey("step-1")
        .build();
```

Partial `Policy`/`Limits` fields you leave unset are omitted and merged onto the daemon defaults.

## Auth

Set `token` on the builder. It is sent as `Authorization: Bearer <token>` on
all authenticated endpoints (everything except `health()` and `openapi()`).
The token is never included in exception messages. `apiKey` remains a legacy
compatibility alias and is used only when `token` is not set.

## Error handling

Non-2xx responses raise `BeatboxApiException` (unchecked) carrying `status()`, `code()` (from the
`{error:{code,message}}` body, may be `null`) and `getMessage()`. Transport failures raise
`BeatboxTransportException`. Both extend `BeatboxException`.

```java
try {
    ExecutionResult result = client.execute(req);
} catch (BeatboxApiException e) {
    System.err.println("API " + e.status() + " / " + e.code() + ": " + e.getMessage());
} catch (BeatboxTransportException e) {
    System.err.println("transport: " + e.getMessage());
}
```

## Example

See [`AddOneExample`](src/main/java/ai/beatbox/examples/AddOneExample.java). With a daemon running:

```bash
CRADLE_TOKEN=... mvn -q exec:java -Dexec.mainClass=ai.beatbox.examples.AddOneExample
```

## Build & test

```bash
mvn -q -e -DskipTests=false test
```

Tests do not require a live daemon (job-URI encoding + JSON round-trip).

## License

Apache-2.0.
