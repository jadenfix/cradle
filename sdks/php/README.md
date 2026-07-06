# beatbox PHP SDK

Zero-dependency PHP client for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox REST API. Uses only the bundled `curl` and `json` extensions. PHP 8.1+.

One of a fleet of 7 language SDKs that share the same method names, configuration,
and error model — learn it once, use it anywhere.

## Install

```bash
composer require beatbox/beatbox
```

Then autoload via Composer:

```php
require 'vendor/autoload.php';
```

Not using Composer? The SDK has no dependencies — just require the bundled
autoloader:

```php
require '/path/to/sdks/php/autoload.php';
```

## Quickstart

Run a WebAssembly module that adds one to its input and read back the value:

```php
<?php
require 'vendor/autoload.php';

use Beatbox\Client;
use Beatbox\ExecuteRequest;

$client = new Client('http://127.0.0.1:7300', getenv('BEATBOX_API_KEY') ?: null);

$wat = '(module (func (export "run") (param i64) (result i64) '
     . 'local.get 0 i64.const 1 i64.add))';

$result = $client->execute(ExecuteRequest::wasmWat($wat, input: ['n' => 41]));

echo $result->value, "\n";                 // 42
echo $result->status->value, "\n";         // "ok"
echo $result->metrics->wallTimeMs, "\n";   // e.g. 12
```

`ExecuteRequest::wasmWat(...)` is the one-line ergonomic path. For other lanes or
sources, build the request explicitly:

```php
use Beatbox\ExecuteRequest;
use Beatbox\Lane;
use Beatbox\Source;
use Beatbox\Policy;
use Beatbox\Limits;

$req = new ExecuteRequest(
    lane: Lane::Wasm,
    source: Source::wasmWat($wat),
    entrypoint: 'run',
    input: ['n' => 41],
    policy: Policy::withLimits(new Limits(wallMs: 5000, fuel: 10_000_000)),
);
```

`Source` constructors: `inline`, `wasmFile`, `wasmWat`, `wasmBytesBase64`,
`moduleRef`. The remote wasm lane accepts `wasmWat` and `wasmBytesBase64`.

## Configuration

```php
new Beatbox\Client(
    string $baseUrl,          // e.g. "http://127.0.0.1:7300" (trailing slashes trimmed)
    ?string $apiKey = null,   // sent as x-beatbox-api-key on authenticated calls
    float $timeout = 65.0,    // seconds
);
```

When `apiKey` is set it is sent on every request **except** `health()` and
`openapi()`, which are unauthenticated. Redirects are never followed, so the
key can't leak cross-origin.

## Methods

| Method | HTTP | Returns |
| --- | --- | --- |
| `health()` | `GET /v1/health` | `array` (raw JSON) |
| `capabilities()` | `GET /v1/capabilities` | `array` (raw JSON) |
| `browserProfiles()` | `GET /v1/browser/profiles` | `array` (raw JSON) |
| `browserAdmit($request)` | `POST /v1/browser/admit` | `array` (raw JSON) |
| `browserAdapterContract()` | `GET /v1/browser/adapter/contract` | `array` (raw JSON) |
| `issueBrowserAdapterCapability($request)` | `POST /v1/browser/adapter/capability` | `array` (raw JSON) |
| `registerBrowserAdapter($request)` | `POST /v1/browser/adapter/register` | `array` (raw JSON) |
| `validateBrowserAdapter($request)` | `POST /v1/browser/adapter/validate` | `array` (raw JSON) |
| `validateBrowserAdapterCompletion($request)` | `POST /v1/browser/adapter/completion/validate` | `array` (raw JSON) |
| `execute($request)` | `POST /v1/execute` | `ExecutionResult` |
| `createJob($request)` | `POST /v1/jobs` | `CreateJobResponse` |
| `getJob($jobId)` | `GET /v1/jobs/{id}` | `JobRecord` |
| `cancelJob($jobId)` | `DELETE /v1/jobs/{id}` | `void` |
| `openapi()` | `GET /openapi.json` | `array` (raw JSON) |

Job ids are percent-encoded as a single path segment; `""`, `"."` and `".."`
are rejected with `InvalidArgumentException`.

### Asynchronous jobs

```php
$job = $client->createJob(ExecuteRequest::wasmWat($wat, input: ['n' => 41]));
$record = $client->getJob($job->jobId);

if ($record->status === Beatbox\JobStatus::Succeeded) {
    echo $record->result->value, "\n";
}

$client->cancelJob($job->jobId); // idempotent
```

## Error handling

Every call raises a typed exception on failure; the API key is never included in
any message.

```php
use Beatbox\ApiError;
use Beatbox\TransportError;

try {
    $result = $client->execute($req);
} catch (ApiError $e) {
    // Non-2xx response from the daemon.
    $e->getStatus();     // int  — HTTP status
    $e->getErrorCode();  // string — error code from the {error:{code,message}} body
    $e->getMessage();    // string — human-readable message
    // Note: getCode() is final on PHP's Exception and returns the HTTP status;
    // use getErrorCode() for the API's string error code.
} catch (TransportError $e) {
    // Connection failure, timeout, or malformed response body.
    $e->getMessage();
}
```

## Models

Typed models mirror the OpenAPI components and serialize to the exact snake_case
wire names (`wall_ms`, `cpu_time_ms`, `idempotency_key`, ...): `ExecuteRequest`,
`Source`, `Policy`, `Limits`, `ExecutionResult`, `Metrics`, `EffectiveIsolation`,
`EgressRecord`, `JobRecord`, `CreateJobResponse`, `ErrorBody`, and the enums
`Lane`, `ExecutionStatus`, `JobStatus`. Unknown/extra fields are ignored and
unknown enum values deserialize to `null`, so new server fields never break an
older SDK.

## Example & tests

```bash
php examples/add_one.php               # needs a running daemon; asserts value === 42
php test/run.php                       # offline unit tests, prints "OK"
```

## License

Apache-2.0
