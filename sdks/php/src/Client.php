<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Client for the beatbox sandbox REST API.
 *
 * Zero third-party dependencies: uses the bundled curl and json extensions.
 *
 * ```php
 * $client = new Beatbox\Client('http://127.0.0.1:7300', token: getenv('CRADLE_TOKEN') ?: null);
 * $result = $client->execute(Beatbox\ExecuteRequest::wasmWat($wat, input: ['n' => 41]));
 * echo $result->value; // 42
 * ```
 *
 * On a non-2xx response, methods throw {@see ApiError}. On a transport-level
 * failure (connection/timeout/malformed body) they throw {@see TransportError}.
 * The token is never included in any exception message.
 */
final class Client
{
    private string $baseUrl;
    private ?string $token;
    private ?string $apiKey;
    private float $timeout;

    /**
     * @param string      $baseUrl e.g. "http://127.0.0.1:7300"; HTTPS, or HTTP only for exact loopback literals
     * @param string|null $apiKey  legacy `x-beatbox-api-key` alias, used only when token is not set
     * @param float       $timeout total request timeout in seconds
     * @param string|null $token   sent as `Authorization: Bearer <token>` on authenticated calls
     */
    public function __construct(string $baseUrl, ?string $apiKey = null, float $timeout = 65.0, ?string $token = null)
    {
        $this->baseUrl = self::validateBaseUrl($baseUrl);
        $this->token = $token;
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
     * GET /v1/integration. Returns raw ecosystem integration contract JSON.
     *
     * @return array<string,mixed>
     */
    public function integration(): array
    {
        return $this->requestJson('GET', '/v1/integration', null, true);
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

    /**
     * GET /v1/browser/adapter/contract. Returns browser adapter contract JSON.
     *
     * @return array<string,mixed>
     */
    public function browserAdapterContract(): array
    {
        return $this->requestJson('GET', '/v1/browser/adapter/contract', null, true);
    }

    /**
     * POST /v1/browser/adapter/capability. Returns browser adapter capability JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function issueBrowserAdapterCapability(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/adapter/capability', $request, true);
    }

    /**
     * POST /v1/browser/adapter/register. Returns browser adapter registration JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function registerBrowserAdapter(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/adapter/register', $request, true);
    }

    /**
     * POST /v1/browser/adapter/launch/plan. Returns launch plan JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function planBrowserAdapterLaunch(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/adapter/launch/plan', $request, true);
    }

    /**
     * POST /v1/browser/adapter/launch/claim. Returns launch claim JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function claimBrowserAdapterLaunch(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/adapter/launch/claim', $request, true);
    }

    /**
     * POST /v1/browser/adapter/validate. Returns browser adapter validation JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function validateBrowserAdapter(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/adapter/validate', $request, true);
    }

    /**
     * POST /v1/browser/adapter/completion/validate. Returns completion validation JSON.
     *
     * @param array<string,mixed> $request
     * @return array<string,mixed>
     */
    public function validateBrowserAdapterCompletion(array $request): array
    {
        return $this->requestJson('POST', '/v1/browser/adapter/completion/validate', $request, true);
    }

    /** POST /v1/execute — run a program synchronously. */
    public function execute(ExecuteRequest $request): ExecutionResult
    {
        return ExecutionResult::fromArray(
            $this->requestJson('POST', '/v1/execute', $request->toArray(), true)
        );
    }

    /** POST /v1/jobs — enqueue an asynchronous job (202). */
    public function createJob(ExecuteRequest $request): Operation
    {
        return Operation::fromArray(
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
     * Validate daemon base URLs before token-bearing requests can be built.
     *
     * HTTPS is accepted for production. Plain HTTP is accepted only for the
     * exact loopback literals used by local daemons, which avoids parser
     * aliases like localhost, short IPv4 forms, DNS names, and IPv6 zone IDs.
     */
    private static function validateBaseUrl(string $baseUrl): string
    {
        if ($baseUrl === '') {
            throw new \InvalidArgumentException('baseUrl is required');
        }
        if (preg_match('/[\x00-\x20\x7f]/', $baseUrl) === 1) {
            throw new \InvalidArgumentException('baseUrl must not contain whitespace or control characters');
        }
        if (str_contains($baseUrl, '\\')) {
            throw new \InvalidArgumentException('baseUrl must not contain backslashes');
        }
        if (str_contains($baseUrl, '?')) {
            throw new \InvalidArgumentException('baseUrl must not contain a query string');
        }
        if (str_contains($baseUrl, '#')) {
            throw new \InvalidArgumentException('baseUrl must not contain a fragment');
        }

        $parts = parse_url($baseUrl);
        if (!is_array($parts)) {
            throw new \InvalidArgumentException('invalid baseUrl');
        }
        $scheme = strtolower((string) ($parts['scheme'] ?? ''));
        if ($scheme !== 'http' && $scheme !== 'https') {
            throw new \InvalidArgumentException('baseUrl must use http or https');
        }

        $authority = self::rawAuthority($baseUrl);
        if ($authority === '') {
            throw new \InvalidArgumentException('baseUrl must include a host');
        }
        if (isset($parts['user']) || isset($parts['pass']) || str_contains($authority, '@')) {
            throw new \InvalidArgumentException('baseUrl must not include credentials');
        }

        $rawHost = self::rawHostFromAuthority($authority);
        if ($rawHost === '' || !isset($parts['host']) || (string) $parts['host'] === '') {
            throw new \InvalidArgumentException('baseUrl must include a host');
        }
        if ($scheme === 'http' && $rawHost !== '127.0.0.1' && strtolower($rawHost) !== '[::1]') {
            throw new \InvalidArgumentException('http baseUrl is allowed only for 127.0.0.1 or [::1]');
        }

        self::validateBasePath(self::rawPath($baseUrl));

        return rtrim($baseUrl, '/');
    }

    private static function rawAuthority(string $baseUrl): string
    {
        $schemeEnd = strpos($baseUrl, '://');
        if ($schemeEnd === false) {
            throw new \InvalidArgumentException('baseUrl must be an absolute URL');
        }

        $authorityStart = $schemeEnd + 3;
        $pathStart = strpos($baseUrl, '/', $authorityStart);
        return $pathStart === false
            ? substr($baseUrl, $authorityStart)
            : substr($baseUrl, $authorityStart, $pathStart - $authorityStart);
    }

    private static function rawHostFromAuthority(string $authority): string
    {
        if ($authority === '') {
            return '';
        }
        if (str_contains($authority, '@')) {
            throw new \InvalidArgumentException('baseUrl must not include credentials');
        }
        if (str_contains($authority, '%')) {
            throw new \InvalidArgumentException('invalid baseUrl');
        }

        if ($authority[0] === '[') {
            $bracketEnd = strpos($authority, ']');
            if ($bracketEnd === false) {
                throw new \InvalidArgumentException('invalid baseUrl');
            }
            $rawHost = substr($authority, 0, $bracketEnd + 1);
            $port = substr($authority, $bracketEnd + 1);
            if ($port !== '') {
                if ($port[0] !== ':' || substr($port, 1) === '' || preg_match('/\A:[0-9]+\z/', $port) !== 1) {
                    throw new \InvalidArgumentException('invalid baseUrl');
                }
            }

            $inside = substr($rawHost, 1, -1);
            if ($inside === '' || str_contains($inside, '%') || filter_var($inside, FILTER_VALIDATE_IP, FILTER_FLAG_IPV6) === false) {
                throw new \InvalidArgumentException('invalid baseUrl');
            }
            return strtolower($rawHost);
        }

        if (str_contains($authority, '[') || str_contains($authority, ']')) {
            throw new \InvalidArgumentException('invalid baseUrl');
        }

        $colon = strpos($authority, ':');
        if ($colon === false) {
            return strtolower($authority);
        }

        if (strpos($authority, ':', $colon + 1) !== false) {
            throw new \InvalidArgumentException('invalid baseUrl');
        }
        $port = substr($authority, $colon + 1);
        if ($port === '' || preg_match('/\A[0-9]+\z/', $port) !== 1) {
            throw new \InvalidArgumentException('invalid baseUrl');
        }

        return strtolower(substr($authority, 0, $colon));
    }

    private static function rawPath(string $baseUrl): string
    {
        $schemeEnd = strpos($baseUrl, '://');
        if ($schemeEnd === false) {
            return '';
        }

        $pathStart = strpos($baseUrl, '/', $schemeEnd + 3);
        return $pathStart === false ? '' : substr($baseUrl, $pathStart);
    }

    private static function validateBasePath(string $rawPath): void
    {
        if ($rawPath === '') {
            return;
        }

        foreach (explode('/', $rawPath) as $segment) {
            if ($segment === '' || $segment === '.') {
                if ($segment === '.') {
                    throw new \InvalidArgumentException('baseUrl path must not contain dot segments');
                }
                continue;
            }
            if ($segment === '..') {
                throw new \InvalidArgumentException('baseUrl path must not contain dot segments');
            }
            if (preg_match('/%(?![0-9A-Fa-f]{2})/', $segment) === 1) {
                throw new \InvalidArgumentException('baseUrl path contains invalid percent encoding');
            }

            $decoded = rawurldecode($segment);
            if ($decoded === '.' || $decoded === '..') {
                throw new \InvalidArgumentException('baseUrl path must not contain encoded dot segments');
            }
            if (str_contains($decoded, '/') || str_contains($decoded, '\\')) {
                throw new \InvalidArgumentException('baseUrl path segments must not encode separators');
            }
        }
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
        if ($auth && $this->token !== null && $this->token !== '') {
            $headers[] = 'Authorization: Bearer ' . $this->token;
        } elseif ($auth && $this->apiKey !== null && $this->apiKey !== '') {
            $headers[] = 'x-beatbox-api-key: ' . $this->apiKey;
        }

        $timeoutMs = (int) round($this->timeout * 1000);
        curl_setopt_array($ch, [
            CURLOPT_URL => $this->baseUrl . $path,
            CURLOPT_CUSTOMREQUEST => $method,
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_FOLLOWLOCATION => false,
            CURLOPT_PROXY => '',
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
            // Message derives only from curl's own diagnostics — no token.
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
