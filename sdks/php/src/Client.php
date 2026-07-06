<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Client for the beatbox sandbox REST API.
 *
 * Zero third-party dependencies: uses the bundled curl and json extensions.
 *
 * ```php
 * $client = new Beatbox\Client('http://127.0.0.1:7300', getenv('BEATBOX_API_KEY') ?: null);
 * $result = $client->execute(Beatbox\ExecuteRequest::wasmWat($wat, input: ['n' => 41]));
 * echo $result->value; // 42
 * ```
 *
 * On a non-2xx response, methods throw {@see ApiError}. On a transport-level
 * failure (connection/timeout/malformed body) they throw {@see TransportError}.
 * The API key is never included in any exception message.
 */
final class Client
{
    private string $baseUrl;
    private ?string $apiKey;
    private float $timeout;

    /**
     * @param string      $baseUrl e.g. "http://127.0.0.1:7300"; trailing slashes are trimmed
     * @param string|null $apiKey  sent as the `x-beatbox-api-key` header on authenticated calls
     * @param float       $timeout total request timeout in seconds
     */
    public function __construct(string $baseUrl, ?string $apiKey = null, float $timeout = 65.0)
    {
        $this->baseUrl = rtrim($baseUrl, '/');
        $this->apiKey = $apiKey;
        $this->timeout = $timeout;
    }

    /**
     * GET /v1/health (unauthenticated). Returns the raw decoded JSON,
     * e.g. `{status, version, uptime_s}`.
     *
     * @return array<string,mixed>
     */
    public function health(): array
    {
        return $this->requestJson('GET', '/v1/health', null, false);
    }

    /**
     * GET /v1/capabilities. Returns raw decoded JSON (lane availability
     * and host limits).
     *
     * @return array<string,mixed>
     */
    public function capabilities(): array
    {
        return $this->requestJson('GET', '/v1/capabilities', null, true);
    }

    /**
     * GET /v1/browser/profiles. Returns browser sandbox discovery metadata.
     *
     * @return array<string,mixed>
     */
    public function browserProfiles(): array
    {
        return $this->requestJson('GET', '/v1/browser/profiles', null, true);
    }

    /**
     * POST /v1/browser/admit. Returns browser sandbox admission decision JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function browserAdmit(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/admit', $request, true);
    }

    /** POST /v1/execute — run a program synchronously. */
    public function execute(ExecuteRequest $request): ExecutionResult
    {
        return ExecutionResult::fromArray(
            $this->requestJson('POST', '/v1/execute', $request->toArray(), true)
        );
    }

    /** POST /v1/jobs — enqueue an asynchronous job (202). */
    public function createJob(ExecuteRequest $request): CreateJobResponse
    {
        return CreateJobResponse::fromArray(
            $this->requestJson('POST', '/v1/jobs', $request->toArray(), true)
        );
    }

    /** GET /v1/jobs/{id} — fetch job state. */
    public function getJob(string $jobId): JobRecord
    {
        return JobRecord::fromArray(
            $this->requestJson('GET', '/v1/jobs/' . self::encodeJobId($jobId), null, true)
        );
    }

    /** DELETE /v1/jobs/{id} — cancel a job (204, idempotent). */
    public function cancelJob(string $jobId): void
    {
        $this->send('DELETE', '/v1/jobs/' . self::encodeJobId($jobId), null, true);
    }

    /**
     * GET /openapi.json (unauthenticated). Returns the raw decoded spec.
     *
     * @return array<string,mixed>
     */
    public function openapi(): array
    {
        return $this->requestJson('GET', '/openapi.json', null, false);
    }

    /**
     * Percent-encode a job id as a single path segment.
     *
     * Rejects "", "." and ".." because they can retarget the request to a
     * different resource. Everything else is encoded with rawurlencode so
     * that slashes, query, and fragment characters stay inside the segment.
     */
    private static function encodeJobId(string $jobId): string
    {
        if ($jobId === '' || $jobId === '.' || $jobId === '..') {
            throw new \InvalidArgumentException('invalid job id: must not be empty, "." or ".."');
        }
        return rawurlencode($jobId);
    }

    /**
     * @param array<string,mixed>|null $body
     * @return array<string,mixed>
     */
    private function requestJson(string $method, string $path, ?array $body, bool $auth): array
    {
        $raw = $this->send($method, $path, $body, $auth);
        if ($raw === '') {
            return [];
        }
        $decoded = json_decode($raw, true);
        if (!is_array($decoded)) {
            throw new TransportError(sprintf('expected a JSON object in the response to %s', $path));
        }
        /** @var array<string,mixed> $decoded */
        return $decoded;
    }

    /**
     * Perform an HTTP request, returning the raw 2xx response body.
     *
     * @param array<string,mixed>|null $body JSON body for POST requests
     * @throws ApiError       on a non-2xx response
     * @throws TransportError on a transport/serialization failure
     */
    private function send(string $method, string $path, ?array $body, bool $auth): string
    {
        $ch = curl_init();
        if ($ch === false) {
            throw new TransportError('failed to initialize curl');
        }

        $headers = ['Accept: application/json'];
        $payload = null;
        if ($body !== null) {
            $payload = json_encode($body, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE);
            if ($payload === false) {
                throw new TransportError('failed to encode request body: ' . json_last_error_msg());
            }
            $headers[] = 'Content-Type: application/json';
        }
        if ($auth && $this->apiKey !== null && $this->apiKey !== '') {
            $headers[] = 'x-beatbox-api-key: ' . $this->apiKey;
        }

        $timeoutMs = (int) round($this->timeout * 1000);
        curl_setopt_array($ch, [
            CURLOPT_URL => $this->baseUrl . $path,
            CURLOPT_CUSTOMREQUEST => $method,
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_FOLLOWLOCATION => false,
            CURLOPT_HTTPHEADER => $headers,
            CURLOPT_TIMEOUT_MS => $timeoutMs,
            CURLOPT_CONNECTTIMEOUT_MS => $timeoutMs,
        ]);
        if ($payload !== null) {
            curl_setopt($ch, CURLOPT_POSTFIELDS, $payload);
        }

        $response = curl_exec($ch);
        if ($response === false) {
            $message = curl_error($ch);
            $errno = curl_errno($ch);
            curl_close($ch);
            // Message derives only from curl's own diagnostics — no api key.
            throw new TransportError(sprintf('request to %s failed: %s', $path, $message), $errno);
        }

        $status = (int) curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
        curl_close($ch);
        $responseBody = is_string($response) ? $response : '';

        if ($status >= 400) {
            throw self::makeApiError($status, $responseBody);
        }

        return $responseBody;
    }

    private static function makeApiError(int $status, string $body): ApiError
    {
        $code = '';
        $message = '';
        $decoded = json_decode($body, true);
        if (is_array($decoded) && isset($decoded['error']) && is_array($decoded['error'])) {
            $error = $decoded['error'];
            $code = is_string($error['code'] ?? null) ? $error['code'] : '';
            $message = is_string($error['message'] ?? null) ? $error['message'] : '';
        }
        if ($message === '') {
            $message = 'HTTP ' . $status;
        }
        return new ApiError($status, $code, $message);
    }
}
