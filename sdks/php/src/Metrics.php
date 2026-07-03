<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Resource-usage metrics for an execution.
 *
 * {@see $cpuTimeMs} is null on lanes that do not measure CPU time
 * separately from wall time (e.g. the W0 wasm lane) — use
 * {@see $fuelUsed} as the deterministic compute signal there.
 */
final class Metrics
{
    public function __construct(
        public int $wallTimeMs,
        public ?int $cpuTimeMs = null,
        public ?int $fuelUsed = null,
        public ?int $peakMemoryBytes = null,
    ) {
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            wallTimeMs: Coerce::intOr($data['wall_time_ms'] ?? null, 0),
            cpuTimeMs: Coerce::intOrNull($data['cpu_time_ms'] ?? null),
            fuelUsed: Coerce::intOrNull($data['fuel_used'] ?? null),
            peakMemoryBytes: Coerce::intOrNull($data['peak_memory_bytes'] ?? null),
        );
    }

    /** @return array<string,int|null> */
    public function toArray(): array
    {
        return [
            'wall_time_ms' => $this->wallTimeMs,
            'cpu_time_ms' => $this->cpuTimeMs,
            'fuel_used' => $this->fuelUsed,
            'peak_memory_bytes' => $this->peakMemoryBytes,
        ];
    }
}
