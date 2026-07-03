<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * A record of network egress observed during an execution.
 */
final class EgressRecord
{
    public function __construct(
        public string $domain,
        public int $port,
        public int $bytes,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            domain: Coerce::stringOr($data['domain'] ?? null, ''),
            port: Coerce::intOr($data['port'] ?? null, 0),
            bytes: Coerce::intOr($data['bytes'] ?? null, 0),
        );
    }
}
