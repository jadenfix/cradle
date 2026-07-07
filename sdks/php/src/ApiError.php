<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Raised when the beatbox API returns a non-2xx response.
 *
 * Carries the HTTP status and the machine-readable error code from the
 * {@code {"error": {"code", "message"}}} body. Auth material is never
 * included in the message.
 *
 * Note: PHP's {@see \Throwable::getCode()} is `final`, so it cannot return
 * the (string) API error code. It returns the HTTP status instead (the
 * conventional HTTP-exception idiom); use {@see getStatus()} for the same
 * value typed as int, and {@see getErrorCode()} for the string error code.
 */
final class ApiError extends \RuntimeException
{
    private int $status;
    private string $errorCode;

    public function __construct(int $status, string $code, string $message)
    {
        parent::__construct($message, $status);
        $this->status = $status;
        $this->errorCode = $code;
    }

    /** HTTP status code of the failed response. */
    public function getStatus(): int
    {
        return $this->status;
    }

    /** Machine-readable error code from the response body (may be empty). */
    public function getErrorCode(): string
    {
        return $this->errorCode;
    }
}
