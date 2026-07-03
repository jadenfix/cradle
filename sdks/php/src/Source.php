<?php

declare(strict_types=1);

namespace Beatbox;

use Beatbox\Internal\Coerce;

/**
 * Program source for an execution. A tagged union on {@code kind}.
 *
 * Construct one via the static factory for the desired variant, e.g.
 * {@code Source::wasmWat('(module ...)')}. The remote wasm lane only
 * accepts {@code wasm_wat} and {@code wasm_bytes_base64}.
 */
final class Source
{
    /**
     * @param array<string,mixed> $fields variant payload (excluding "kind")
     */
    private function __construct(
        public readonly string $kind,
        public readonly array $fields,
    ) {
    }

    /** Inline source code for interpreted lanes. */
    public static function inline(string $code): self
    {
        return new self('inline', ['code' => $code]);
    }

    /** Path to a wasm module readable by the daemon. */
    public static function wasmFile(string $path): self
    {
        return new self('wasm_file', ['path' => $path]);
    }

    /** WebAssembly text format (WAT). */
    public static function wasmWat(string $text): self
    {
        return new self('wasm_wat', ['text' => $text]);
    }

    /** Base64-encoded wasm module bytes. */
    public static function wasmBytesBase64(string $bytes): self
    {
        return new self('wasm_bytes_base64', ['bytes' => $bytes]);
    }

    /** Reference to a previously registered module by sha256. */
    public static function moduleRef(string $sha256): self
    {
        return new self('module_ref', ['sha256' => $sha256]);
    }

    /** @return array<string,mixed> */
    public function toArray(): array
    {
        return array_merge(['kind' => $this->kind], $this->fields);
    }

    /** @param array<string,mixed> $data */
    public static function fromArray(array $data): self
    {
        $kind = Coerce::stringOr($data['kind'] ?? null, '');
        unset($data['kind']);
        return new self($kind, $data);
    }
}
