<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * The {@code {code, message}} error payload embedded in error responses
 * and in failed {@see ExecutionResult}/{@see JobRecord} objects.
 */
final class ErrorBody
{
    public function __construct(
        public string $code,
        public string $message,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            code: Coerce::stringOr($data['code'] ?? null, ''),
            message: Coerce::stringOr($data['message'] ?? null, ''),
        );
    }
}
