<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * The isolation mechanisms actually applied to an execution, and any
 * downgrades the host had to make relative to the requested policy.
 */
final class EffectiveIsolation
{
    /**
     * @param list<string> $mechanisms
     * @param list<string> $downgrades
     */
    public function __construct(
        public string $os,
        public array $mechanisms,
        public array $downgrades,
        public ?int $landlockAbi = null,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            os: Coerce::stringOr($data['os'] ?? null, ''),
            mechanisms: Coerce::stringList($data['mechanisms'] ?? null),
            downgrades: Coerce::stringList($data['downgrades'] ?? null),
            landlockAbi: Coerce::intOrNull($data['landlock_abi'] ?? null),
        );
    }
}
