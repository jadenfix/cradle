<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Legacy pre-Operation create-job response shape.
 */
final class CreateJobResponse
{
    public function __construct(
        public string $jobId,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            jobId: Coerce::stringOr($data['job_id'] ?? null, ''),
        );
    }
}
