<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

final class OperationMetadata
{
    public function __construct(
        public string $targetResource = '',
        public string $createTime = '',
        public string $currentStage = '',
        public float $progressRatio = 0.0,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            targetResource: Coerce::stringOr($data['target_resource'] ?? null, ''),
            createTime: Coerce::stringOr($data['create_time'] ?? null, ''),
            currentStage: Coerce::stringOr($data['current_stage'] ?? null, ''),
            progressRatio: is_numeric($data['progress_ratio'] ?? null) ? (float) $data['progress_ratio'] : 0.0,
        );
    }
}
