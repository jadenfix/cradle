<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Body for {@see Client::execute()} and {@see Client::createJob()}.
 *
 * Optional fields are omitted from the wire form unless explicitly set.
 * {@code input} may be any JSON value (including {@code null}), so a
 * private sentinel distinguishes "no input" from "input is null".
 */
final class ExecuteRequest
{
    /** Sentinel meaning "input was not provided". */
    public const UNSET = "\0beatbox:unset\0";

    /**
     * @param mixed $input any JSON value, or {@see self::UNSET} to omit
     */
    public function __construct(
        public Lane $lane,
        public Source $source,
        public ?string $entrypoint = null,
        public mixed $input = self::UNSET,
        public ?string $stdin = null,
        public ?Policy $policy = null,
        public ?string $idempotencyKey = null,
    ) {
    }

    /**
     * Ergonomic constructor for the common wasm-WAT case.
     *
     * @param mixed $input any JSON value, or {@see self::UNSET} to omit
     */
    public static function wasmWat(
        string $text,
        mixed $input = self::UNSET,
        ?string $entrypoint = null,
        ?Policy $policy = null,
    ): self {
        return new self(
            lane: Lane::Wasm,
            source: Source::wasmWat($text),
            entrypoint: $entrypoint,
            input: $input,
            policy: $policy,
        );
    }

    /** @return array<string,mixed> */
    public function toArray(): array
    {
        $out = [
            'lane' => $this->lane->value,
            'source' => $this->source->toArray(),
        ];
        if ($this->entrypoint !== null) {
            $out['entrypoint'] = $this->entrypoint;
        }
        if ($this->input !== self::UNSET) {
            $out['input'] = $this->input;
        }
        if ($this->stdin !== null) {
            $out['stdin'] = $this->stdin;
        }
        if ($this->policy !== null) {
            $out['policy'] = $this->policy->toArray();
        }
        if ($this->idempotencyKey !== null) {
            $out['idempotency_key'] = $this->idempotencyKey;
        }
        return $out;
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        $lane = Lane::tryFrom(Coerce::stringOr($data['lane'] ?? null, '')) ?? Lane::Wasm;
        $source = Source::fromArray(Coerce::arrayOr($data['source'] ?? null));
        $policy = isset($data['policy']) && is_array($data['policy'])
            ? Policy::fromArray($data['policy'])
            : null;

        return new self(
            lane: $lane,
            source: $source,
            entrypoint: Coerce::stringOrNull($data['entrypoint'] ?? null),
            input: array_key_exists('input', $data) ? $data['input'] : self::UNSET,
            stdin: Coerce::stringOrNull($data['stdin'] ?? null),
            policy: $policy,
            idempotencyKey: Coerce::stringOrNull($data['idempotency_key'] ?? null),
        );
    }
}
