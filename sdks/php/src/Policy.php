<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Execution policy. Partial: only fields you set are serialized.
 *
 * {@see Limits} is modelled directly. Less common policy sections
 * (determinism, fs, net, env, secrets, double_jail) are preserved
 * verbatim through {@see $extra} so the SDK stays forward-compatible
 * without a bespoke type for every knob.
 */
final class Policy
{
    /**
     * @param array<string,mixed> $extra additional policy fields passed through as-is
     */
    public function __construct(
        public ?Limits $limits = null,
        public array $extra = [],
    ) {
    }

    /** Convenience constructor for the common "just set some limits" case. */
    public static function withLimits(Limits $limits): self
    {
        return new self(limits: $limits);
    }

    /** @return array<string,mixed> */
    public function toArray(): array
    {
        $out = $this->extra;
        if ($this->limits !== null) {
            $out['limits'] = $this->limits->toArray();
        }
        return $out;
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        $limits = null;
        if (isset($data['limits']) && is_array($data['limits'])) {
            $limits = Limits::fromArray($data['limits']);
            unset($data['limits']);
        }
        return new self(limits: $limits, extra: $data);
    }
}
