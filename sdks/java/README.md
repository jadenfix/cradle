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
        .apiKey(System.getenv("BEATBOX_API_KEY")) // optional
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
| `apiKey`    | no       | none           | sent as `x-beatbox-api-key` on every request except `health()` and `openapi()` |
| `timeout`   | no       | 65 seconds     | per-request timeout |

Redirects are never followed, so the api-key header can't leak cross-origin.

## Methods

| Method                     | HTTP                     | Returns             |
| -------------------------- | ------------------------ | ------------------- |
| `health()`                 | `GET /v1/health`         | `JsonNode` (raw)    |
| `capabilities()`           | `GET /v1/capabilities`   | `JsonNode` (raw)    |
| `browserProfiles()`        | `GET /v1/browser/profiles` | `JsonNode` (raw)  |
| `browserAdmit(request)`    | `POST /v1/browser/admit` | `JsonNode` (raw)    |
| `execute(request)`         | `POST /v1/execute`       | `ExecutionResult`   |
| `createJob(request)`       | `POST /v1/jobs`          | `CreateJobResponse` |
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

Set `apiKey` on the builder. It is sent as `x-beatbox-api-key` on all authenticated endpoints
(everything except `health()` and `openapi()`). The key is never included in exception messages.

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
BEATBOX_API_KEY=... mvn -q exec:java -Dexec.mainClass=ai.beatbox.examples.AddOneExample
```

## Build & test

```bash
mvn -q -e -DskipTests=false test
```

Tests do not require a live daemon (job-URI encoding + JSON round-trip).

## License

Apache-2.0.
