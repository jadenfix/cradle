<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

final class Operation
{
    /**
     * @param array<string,mixed>|null $response
     */
    public function __construct(
        public string $name,
        public bool $done,
        public ?OperationMetadata $metadata = null,
        public ?array $response = null,
        public ?ErrorBody $error = null,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        $metadata = $data['metadata'] ?? null;
        $error = $data['error'] ?? null;
        return new self(
            name: Coerce::stringOr($data['name'] ?? null, ''),
            done: Coerce::boolOr($data['done'] ?? null, false),
            metadata: is_array($metadata) ? OperationMetadata::fromArray($metadata) : null,
            response: is_array($data['response'] ?? null) ? $data['response'] : null,
            error: is_array($error) ? ErrorBody::fromArray($error) : null,
        );
    }
}
