<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * The shared error payload embedded in error responses
 * and in failed {@see ExecutionResult}/{@see JobRecord} objects.
 */
final class ErrorBody
{
    public function __construct(
        public string $code,
        public string $message,
        public int $status = 0,
        public string $requestId = '',
        public bool $retryable = false,
        /** @var array<int,array<string,mixed>> */
        public array $details = [],
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            code: Coerce::stringOr($data['code'] ?? null, ''),
            message: Coerce::stringOr($data['message'] ?? null, ''),
            status: Coerce::intOr($data['status'] ?? null, 0),
            requestId: Coerce::stringOr($data['request_id'] ?? null, ''),
            retryable: Coerce::boolOr($data['retryable'] ?? null, false),
            details: is_array($data['details'] ?? null) ? $data['details'] : [],
        );
    }
}
