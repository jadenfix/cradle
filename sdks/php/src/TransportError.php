<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Raised when a request cannot be completed at the transport layer
 * (connection failure, timeout, TLS error, malformed response body).
 *
 * The API key is never included in the message.
 */
final class TransportError extends \RuntimeException
{
    public function __construct(string $message, int $code = 0)
    {
        parent::__construct($message, $code);
    }
}
