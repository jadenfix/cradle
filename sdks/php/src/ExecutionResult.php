<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Result of a synchronous execution (the body of {@see Client::execute()}
 * and the {@code result} field of a finished {@see JobRecord}).
 *
 * Unknown/extra wire fields are ignored. Unknown enum values for
 * {@see $status}/{@see $lane} deserialize to null rather than crashing.
 */
final class ExecutionResult
{
    /**
     * @param mixed $value the program's return value (any JSON)
     * @param list<EgressRecord> $egress
     */
    public function __construct(
        public ?ExecutionStatus $status,
        public mixed $value,
        public string $stdout,
        public bool $stdoutTruncated,
        public string $stderr,
        public bool $stderrTruncated,
        public Metrics $metrics,
        public ?Lane $lane,
        public bool $deterministic,
        public string $inputsDigest,
        public string $engineVersion,
        public string $beatboxVersion,
        public ?EffectiveIsolation $effectiveIsolation,
        public array $egress,
        public ?ErrorBody $error = null,
        public ?int $exitCode = null,
    ) {
    }

    /** True when {@see $status} is {@see ExecutionStatus::Ok}. */
    public function isOk(): bool
    {
        return $this->status === ExecutionStatus::Ok;
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        $egress = [];
        foreach (Coerce::arrayOr($data['egress'] ?? null) as $item) {
            if (is_array($item)) {
                $egress[] = EgressRecord::fromArray($item);
            }
        }

        $error = null;
        if (isset($data['error']) && is_array($data['error'])) {
            $error = ErrorBody::fromArray($data['error']);
        }

        $isolation = null;
        if (isset($data['effective_isolation']) && is_array($data['effective_isolation'])) {
            $isolation = EffectiveIsolation::fromArray($data['effective_isolation']);
        }

        return new self(
            status: is_string($data['status'] ?? null) ? ExecutionStatus::tryFrom($data['status']) : null,
            value: $data['value'] ?? null,
            stdout: Coerce::stringOr($data['stdout'] ?? null, ''),
            stdoutTruncated: Coerce::boolOr($data['stdout_truncated'] ?? null, false),
            stderr: Coerce::stringOr($data['stderr'] ?? null, ''),
            stderrTruncated: Coerce::boolOr($data['stderr_truncated'] ?? null, false),
            metrics: Metrics::fromArray(Coerce::arrayOr($data['metrics'] ?? null)),
            lane: is_string($data['lane'] ?? null) ? Lane::tryFrom($data['lane']) : null,
            deterministic: Coerce::boolOr($data['deterministic'] ?? null, false),
            inputsDigest: Coerce::stringOr($data['inputs_digest'] ?? null, ''),
            engineVersion: Coerce::stringOr($data['engine_version'] ?? null, ''),
            beatboxVersion: Coerce::stringOr($data['beatbox_version'] ?? null, ''),
            effectiveIsolation: $isolation,
            egress: $egress,
            error: $error,
            exitCode: Coerce::intOrNull($data['exit_code'] ?? null),
        );
    }
}
