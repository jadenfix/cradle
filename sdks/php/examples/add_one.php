<?php

declare(strict_types=1);

/**
 * Runs an add-one wasm module and asserts the returned value is 42.
 *
 * Usage:
 *   CRADLE_TOKEN=... php examples/add_one.php [base_url]
 *
 * Requires a running beatbox daemon (default http://127.0.0.1:7300).
 */

require __DIR__ . '/../autoload.php';

use Beatbox\ApiError;
use Beatbox\Client;
use Beatbox\ExecuteRequest;
use Beatbox\TransportError;

$baseUrl = $argv[1] ?? 'http://127.0.0.1:7300';
$token = getenv('CRADLE_TOKEN') ?: null;

$client = new Client($baseUrl, token: $token);

// A wasm module exporting `run(i64) -> i64` that adds 1 to its argument.
$wat = '(module (func (export "run") (param i64) (result i64) '
    . 'local.get 0 i64.const 1 i64.add))';

try {
    $result = $client->execute(ExecuteRequest::wasmWat($wat, input: ['n' => 41]));
} catch (ApiError $e) {
    fwrite(STDERR, sprintf("API error %d (%s): %s\n", $e->getStatus(), $e->getErrorCode(), $e->getMessage()));
    exit(1);
} catch (TransportError $e) {
    fwrite(STDERR, 'transport error: ' . $e->getMessage() . "\n");
    exit(1);
}

printf("status=%s value=%s\n", $result->status?->value ?? 'null', var_export($result->value, true));

if ($result->value !== 42) {
    fwrite(STDERR, "expected value 42\n");
    exit(1);
}

echo "OK: add-one returned 42\n";
