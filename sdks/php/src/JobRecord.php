<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * State of an asynchronous job (the body of {@see Client::getJob()}).
 *
 * {@see $result} is set once the job finishes; {@see $error} is set when
 * it failed. Unknown enum values for {@see $status} deserialize to null.
 */
final class JobRecord
{
    public function __construct(
        public string $jobId,
        public ?JobStatus $status,
        public ExecuteRequest $request,
        public string $createdAt,
        public string $updatedAt,
        public ?ExecutionResult $result = null,
        public ?ErrorBody $error = null,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        $result = null;
        if (isset($data['result']) && is_array($data['result'])) {
            $result = ExecutionResult::fromArray($data['result']);
        }

        $error = null;
        if (isset($data['error']) && is_array($data['error'])) {
            $error = ErrorBody::fromArray($data['error']);
        }

        return new self(
            jobId: Coerce::stringOr($data['job_id'] ?? null, ''),
            status: is_string($data['status'] ?? null) ? JobStatus::tryFrom($data['status']) : null,
            request: ExecuteRequest::fromArray(Coerce::arrayOr($data['request'] ?? null)),
            createdAt: Coerce::stringOr($data['created_at'] ?? null, ''),
            updatedAt: Coerce::stringOr($data['updated_at'] ?? null, ''),
            result: $result,
            error: $error,
        );
    }
}
