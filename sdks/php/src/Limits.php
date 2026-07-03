<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Partial resource limits. Only the fields you set are serialized; the
 * daemon merges them onto its defaults. All values are in the units named
 * by the wire field (milliseconds, bytes, fuel units, process count).
 */
final class Limits
{
    public function __construct(
        public ?int $wallMs = null,
        public ?int $cpuMs = null,
        public ?int $memoryBytes = null,
        public ?int $fuel = null,
        public ?int $pids = null,
        public ?int $diskBytes = null,
        public ?int $outputBytes = null,
    ) {
    }

    /** @return array<string,int> */
    public function toArray(): array
    {
        $out = [];
        if ($this->wallMs !== null) {
            $out['wall_ms'] = $this->wallMs;
        }
        if ($this->cpuMs !== null) {
            $out['cpu_ms'] = $this->cpuMs;
        }
        if ($this->memoryBytes !== null) {
            $out['memory_bytes'] = $this->memoryBytes;
        }
        if ($this->fuel !== null) {
            $out['fuel'] = $this->fuel;
        }
        if ($this->pids !== null) {
            $out['pids'] = $this->pids;
        }
        if ($this->diskBytes !== null) {
            $out['disk_bytes'] = $this->diskBytes;
        }
        if ($this->outputBytes !== null) {
            $out['output_bytes'] = $this->outputBytes;
        }
        return $out;
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        return new self(
            wallMs: Coerce::intOrNull($data['wall_ms'] ?? null),
            cpuMs: Coerce::intOrNull($data['cpu_ms'] ?? null),
            memoryBytes: Coerce::intOrNull($data['memory_bytes'] ?? null),
            fuel: Coerce::intOrNull($data['fuel'] ?? null),
            pids: Coerce::intOrNull($data['pids'] ?? null),
            diskBytes: Coerce::intOrNull($data['disk_bytes'] ?? null),
            outputBytes: Coerce::intOrNull($data['output_bytes'] ?? null),
        );
    }
}
