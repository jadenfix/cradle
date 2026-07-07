<?php

declare(strict_types=1);

/**
 * Plain-PHP unit tests for the Beatbox SDK. No live daemon, no PHPUnit.
 *
 *   php test/run.php
 *
 * assert() is compiled out unless zend.assertions=1, so if assertions are
 * not active we re-exec ourselves once with them enabled. This keeps the
 * `php test/run.php` invocation working while still using real assert()s.
 */

if ((int) ini_get('zend.assertions') !== 1) {
    $cmd = escapeshellarg(PHP_BINARY)
        . ' -d zend.assertions=1 -d assert.exception=1 '
        . escapeshellarg(__FILE__);
    $args = array_slice($_SERVER['argv'] ?? [], 1);
    foreach ($args as $arg) {
        $cmd .= ' ' . escapeshellarg((string) $arg);
    }
    passthru($cmd, $code);
    exit($code);
}

assert_options(ASSERT_ACTIVE, 1);
assert_options(ASSERT_EXCEPTION, 1);

require __DIR__ . '/../autoload.php';

use Beatbox\Client;
use Beatbox\ExecuteRequest;
use Beatbox\ExecutionResult;
use Beatbox\ExecutionStatus;
use Beatbox\JobRecord;
use Beatbox\JobStatus;
use Beatbox\Lane;
use Beatbox\Limits;
use Beatbox\Policy;
use Beatbox\Source;

/**
 * Invoke the private Client::encodeJobId for encoding assertions.
 */
function encodeJobId(string $id): string
{
    $m = new ReflectionMethod(Client::class, 'encodeJobId');
    $m->setAccessible(true);
    return (string) $m->invoke(null, $id);
}

/** Read the private normalized base URL for construction assertions. */
function clientBaseUrl(Client $client): string
{
    $prop = new ReflectionProperty(Client::class, 'baseUrl');
    $prop->setAccessible(true);
    return (string) $prop->getValue($client);
}

/** Assert that a callable throws the given exception class. */
function assertThrows(string $class, callable $fn, string $label): void
{
    $threw = false;
    try {
        $fn();
    } catch (\Throwable $e) {
        $threw = $e instanceof $class;
        assert($threw, "$label: expected $class, got " . get_class($e));
    }
    assert($threw, "$label: expected $class to be thrown");
}

/** Run a callback with temporary environment overrides. */
function withEnv(array $overrides, callable $fn): void
{
    $oldValues = [];
    foreach ($overrides as $key => $_value) {
        $oldValues[$key] = getenv((string) $key);
    }

    try {
        foreach ($overrides as $key => $value) {
            if ($value === null) {
                putenv((string) $key);
                unset($_ENV[(string) $key], $_SERVER[(string) $key]);
            } else {
                putenv((string) $key . '=' . (string) $value);
                $_ENV[(string) $key] = (string) $value;
                $_SERVER[(string) $key] = (string) $value;
            }
        }
        $fn();
    } finally {
        foreach ($oldValues as $key => $value) {
            if ($value === false) {
                putenv((string) $key);
                unset($_ENV[(string) $key], $_SERVER[(string) $key]);
            } else {
                putenv((string) $key . '=' . (string) $value);
                $_ENV[(string) $key] = (string) $value;
                $_SERVER[(string) $key] = (string) $value;
            }
        }
    }
}

/** Reserve and release a local port, returning the now-unused port number. */
function unusedLocalPort(): int
{
    $server = stream_socket_server('tcp://127.0.0.1:0', $errno, $errstr);
    if ($server === false) {
        throw new RuntimeException("failed to reserve local port: $errstr", $errno);
    }
    $name = stream_socket_get_name($server, false);
    fclose($server);
    if (!is_string($name)) {
        throw new RuntimeException('failed to read reserved local port');
    }

    $pos = strrpos($name, ':');
    if ($pos === false) {
        throw new RuntimeException("unexpected local socket name: $name");
    }
    return (int) substr($name, $pos + 1);
}

$tests = [];

// ---- Job-id encoding ------------------------------------------------------

$tests['job id: ../execute is encoded to a single segment'] = static function (): void {
    assert(encodeJobId('../execute') === '..%2Fexecute', 'slash must be percent-encoded');
};

$tests['job id: query characters are encoded'] = static function (): void {
    assert(encodeJobId('x?k=v') === 'x%3Fk%3Dv', '? and = must be percent-encoded');
};

$tests['job id: normal uuid passes through unchanged'] = static function (): void {
    $uuid = '3f8b2c1a-0000-4a2b-8c3d-1234567890ab';
    assert(encodeJobId($uuid) === $uuid, 'uuid should be unchanged');
};

$tests['job id: empty/./.. are rejected'] = static function (): void {
    foreach (['', '.', '..'] as $bad) {
        assertThrows(
            \InvalidArgumentException::class,
            static fn () => encodeJobId($bad),
            "encode '$bad'"
        );
    }
};

$tests['job id: getJob/cancelJob reject bad ids before any network call'] = static function (): void {
    $client = new Client('http://127.0.0.1:7300', 'secret-key');
    foreach (['', '.', '..'] as $bad) {
        assertThrows(\InvalidArgumentException::class, static fn () => $client->getJob($bad), "getJob '$bad'");
        assertThrows(\InvalidArgumentException::class, static fn () => $client->cancelJob($bad), "cancelJob '$bad'");
    }
};

// ---- Request JSON round-trip ---------------------------------------------

$tests['request: round-trips through JSON with exact wire names'] = static function (): void {
    $req = new ExecuteRequest(
        lane: Lane::Wasm,
        source: Source::wasmWat('(module)'),
        entrypoint: 'run',
        input: ['n' => 41],
        stdin: 'hi',
        policy: Policy::withLimits(new Limits(wallMs: 5000, fuel: 10_000_000)),
        idempotencyKey: 'step-1',
    );

    $wire = $req->toArray();
    assert($wire['lane'] === 'wasm', 'lane wire value');
    assert($wire['source']['kind'] === 'wasm_wat', 'source kind wire value');
    assert($wire['source']['text'] === '(module)', 'source text');
    assert($wire['input'] === ['n' => 41], 'input preserved');
    assert($wire['idempotency_key'] === 'step-1', 'snake_case idempotency_key');
    assert($wire['policy']['limits']['wall_ms'] === 5000, 'snake_case wall_ms');
    assert($wire['policy']['limits']['fuel'] === 10_000_000, 'fuel');

    $json = json_encode($wire, JSON_UNESCAPED_SLASHES);
    assert(is_string($json), 'encodable');
    $decoded = json_decode($json, true);
    assert(is_array($decoded), 'decodable');

    $round = ExecuteRequest::fromArray($decoded)->toArray();
    assert($round === $wire, 'request must survive encode/decode/rebuild unchanged');
};

$tests['request: unset input is omitted, explicit null is kept'] = static function (): void {
    $omitted = ExecuteRequest::wasmWat('(module)')->toArray();
    assert(!array_key_exists('input', $omitted), 'unset input must not be serialized');

    $withNull = (new ExecuteRequest(Lane::Wasm, Source::wasmWat('(module)'), input: null))->toArray();
    assert(array_key_exists('input', $withNull), 'explicit null input must be serialized');
    assert($withNull['input'] === null, 'null input value');
};

$tests['source: every variant serializes its tagged shape'] = static function (): void {
    assert(Source::inline('x')->toArray() === ['kind' => 'inline', 'code' => 'x'], 'inline');
    assert(Source::wasmFile('/a')->toArray() === ['kind' => 'wasm_file', 'path' => '/a'], 'wasm_file');
    assert(Source::wasmBytesBase64('AAA')->toArray() === ['kind' => 'wasm_bytes_base64', 'bytes' => 'AAA'], 'bytes');
    assert(Source::moduleRef('deadbeef')->toArray() === ['kind' => 'module_ref', 'sha256' => 'deadbeef'], 'module_ref');
};

// ---- Result JSON round-trip ----------------------------------------------

$tests['result: deserializes wire JSON, nullable metrics, unknown fields tolerated'] = static function (): void {
    $wire = [
        'status' => 'ok',
        'value' => 42,
        'stdout' => 'out',
        'stdout_truncated' => false,
        'stderr' => '',
        'stderr_truncated' => false,
        'metrics' => [
            'wall_time_ms' => 12,
            'cpu_time_ms' => null,
            'fuel_used' => 9000,
            'peak_memory_bytes' => null,
        ],
        'lane' => 'wasm',
        'deterministic' => true,
        'inputs_digest' => 'sha256:abc',
        'engine_version' => 'w0',
        'beatbox_version' => '0.1.0',
        'effective_isolation' => [
            'os' => 'linux',
            'mechanisms' => ['seccomp', 'landlock'],
            'downgrades' => [],
            'landlock_abi' => 4,
        ],
        'egress' => [
            ['domain' => 'example.com', 'port' => 443, 'bytes' => 10],
        ],
        'exit_code' => 0,
        // Forward-compat: an unknown field the SDK has never seen.
        'future_field' => ['anything' => true],
    ];

    $json = json_encode($wire);
    assert(is_string($json), 'encodable');
    $result = ExecutionResult::fromArray(json_decode($json, true));

    assert($result instanceof ExecutionResult, 'type');
    assert($result->status === ExecutionStatus::Ok, 'status enum');
    assert($result->isOk(), 'isOk');
    assert($result->value === 42, 'value === 42');
    assert($result->lane === Lane::Wasm, 'lane enum');
    assert($result->metrics->wallTimeMs === 12, 'wall_time_ms');
    assert($result->metrics->cpuTimeMs === null, 'cpu_time_ms nullable -> null');
    assert($result->metrics->fuelUsed === 9000, 'fuel_used');
    assert($result->metrics->peakMemoryBytes === null, 'peak_memory_bytes nullable -> null');
    assert($result->effectiveIsolation !== null, 'isolation present');
    assert($result->effectiveIsolation->mechanisms === ['seccomp', 'landlock'], 'mechanisms');
    assert(count($result->egress) === 1, 'one egress record');
    assert($result->egress[0]->port === 443, 'egress port');
    assert($result->exitCode === 0, 'exit_code');
    assert($result->error === null, 'no error');
};

$tests['result: unknown enum values deserialize to null, not a crash'] = static function (): void {
    $result = ExecutionResult::fromArray([
        'status' => 'brand_new_status',
        'value' => null,
        'metrics' => ['wall_time_ms' => 1],
        'lane' => 'quantum',
    ]);
    assert($result->status === null, 'unknown status -> null');
    assert($result->lane === null, 'unknown lane -> null');
};

$tests['job: JobRecord round-trips including nested request and result'] = static function (): void {
    $wire = [
        'job_id' => 'job-123',
        'status' => 'succeeded',
        'request' => [
            'lane' => 'wasm',
            'source' => ['kind' => 'wasm_wat', 'text' => '(module)'],
            'input' => ['n' => 41],
        ],
        'result' => [
            'status' => 'ok',
            'value' => 42,
            'stdout' => '',
            'stdout_truncated' => false,
            'stderr' => '',
            'stderr_truncated' => false,
            'metrics' => ['wall_time_ms' => 3],
            'lane' => 'wasm',
            'deterministic' => true,
            'inputs_digest' => 'd',
            'engine_version' => 'w0',
            'beatbox_version' => '0.1.0',
            'effective_isolation' => ['os' => 'linux', 'mechanisms' => [], 'downgrades' => []],
            'egress' => [],
        ],
        'created_at' => '2026-07-03T00:00:00Z',
        'updated_at' => '2026-07-03T00:00:01Z',
    ];

    $job = JobRecord::fromArray(json_decode((string) json_encode($wire), true));
    assert($job->jobId === 'job-123', 'job_id');
    assert($job->status === JobStatus::Succeeded, 'status enum');
    assert($job->request->lane === Lane::Wasm, 'nested request lane');
    assert($job->request->input === ['n' => 41], 'nested request input');
    assert($job->result !== null && $job->result->value === 42, 'nested result value');
    assert($job->createdAt === '2026-07-03T00:00:00Z', 'created_at');
    assert($job->error === null, 'no error');
};

// ---- Client construction --------------------------------------------------

$tests['client: trims trailing slashes from base url'] = static function (): void {
    $client = new Client('http://127.0.0.1:7300///');
    assert(clientBaseUrl($client) === 'http://127.0.0.1:7300', 'trailing slashes trimmed');
};

$tests['client: accepts secure and exact loopback base urls'] = static function (): void {
    $cases = [
        'https://host:7300/api/' => 'https://host:7300/api',
        'https://daemon.example/root%7E' => 'https://daemon.example/root%7E',
        'http://127.0.0.1:7300/' => 'http://127.0.0.1:7300',
        'http://[::1]:7300/' => 'http://[::1]:7300',
    ];

    foreach ($cases as $baseUrl => $expected) {
        assert(clientBaseUrl(new Client($baseUrl)) === $expected, "$baseUrl normalized");
    }
};

$tests['client: rejects base urls that could leak api keys'] = static function (): void {
    $rejected = [
        ' http://127.0.0.1:7300',
        'http://127.0.0.1:7300 ',
        "https://api.example.com\t.evil.test:7300",
        "https://api.example.com\n.evil.test:7300",
        "http://127.0.0.1\t:7300",
        "http://127.0.0.1\r:7300",
        'http://host:7300',
        'http://localhost:7300',
        'http://LOCALHOST:7300',
        'http://localhost.evil.example:7300',
        'http://127.1:7300',
        'http://127.000.000.001:7300',
        'http://0177.0.0.1:7300',
        'http://2130706433:7300',
        'http://127.0.0.1.:7300',
        'http://192.168.1.10:7300',
        'http://169.254.169.254',
        'http://[0:0:0:0:0:0:0:1]:7300',
        'http://[::1]extra:7300',
        'http://[::1].evil.test:7300',
        'http://[::1%25lo]:7300',
        'https://user:pass@host:7300',
        'https://user@host:7300',
        'https://@host:7300',
        'https://host%zz',
        'https://host%',
        'https://host%41:7300',
        'https://host name:7300',
        'https://host:7300 ',
        'https://host:7300?api_key=hidden',
        'https://host:7300?',
        'https://host:7300/#fragment',
        'https://host:7300#',
        'https://host:7300/?',
        'https://host:7300/#',
        'http://127.0.0.1:7300/?',
        'https://host:badport',
        'https://host:',
        'https://:7300',
        'https://[]:7300',
        'https://host\\evil',
        'https://host:7300/api/../other',
        'https://host:7300/api/./other',
        'https://host:7300/api/%2e%2e/other',
        'https://host:7300/api/%2e/other',
        'https://host:7300/api%2fother',
        'https://host:7300/api%5Cother',
        'https://host:7300/api/%',
        'https://host:7300/api/%zz',
        'https://host:7300/api\\other',
        'file:///tmp/beatbox.sock',
        '/v1',
        '',
    ];

    foreach ($rejected as $baseUrl) {
        assertThrows(
            \InvalidArgumentException::class,
            static fn () => new Client($baseUrl),
            "baseUrl '$baseUrl'"
        );
    }
};

$tests['client: bypasses environment proxy for api-key-bearing requests'] = static function (): void {
    $proxy = stream_socket_server('tcp://127.0.0.1:0', $errno, $errstr);
    if ($proxy === false) {
        throw new RuntimeException("failed to start proxy listener: $errstr", $errno);
    }

    try {
        stream_set_blocking($proxy, false);
        $proxyName = stream_socket_get_name($proxy, false);
        assert(is_string($proxyName), 'proxy listener has a local address');
        $targetPort = unusedLocalPort();

        withEnv([
            'http_proxy' => 'http://' . $proxyName,
            'HTTP_PROXY' => 'http://' . $proxyName,
            'all_proxy' => 'http://' . $proxyName,
            'ALL_PROXY' => 'http://' . $proxyName,
            'no_proxy' => null,
            'NO_PROXY' => null,
        ], static function () use ($proxy, $targetPort): void {
            $client = new Client("http://127.0.0.1:$targetPort", 'secret-key', 0.2);
            assertThrows(
                \Beatbox\TransportError::class,
                static fn () => $client->capabilities(),
                'capabilities without daemon'
            );

            $read = [$proxy];
            $write = null;
            $except = null;
            $ready = stream_select($read, $write, $except, 0, 200000);
            assert($ready === 0, 'environment proxy must not receive the api-key-bearing request');
        });
    } finally {
        fclose($proxy);
    }
};

// ---- Runner ---------------------------------------------------------------

$failed = 0;
foreach ($tests as $name => $fn) {
    try {
        $fn();
        fwrite(STDOUT, "  ok   - $name\n");
    } catch (\Throwable $e) {
        $failed++;
        fwrite(STDOUT, "  FAIL - $name\n");
        fwrite(STDERR, '         ' . $e->getMessage() . "\n");
    }
}

$total = count($tests);
if ($failed > 0) {
    fwrite(STDERR, sprintf("\n%d of %d tests FAILED\n", $failed, $total));
    exit(1);
}

fwrite(STDOUT, sprintf("\nOK - %d tests passed\n", $total));
exit(0);
