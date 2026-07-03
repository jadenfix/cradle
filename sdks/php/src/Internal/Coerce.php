<?php

declare(strict_types=1);

namespace Beatbox\Internal;

/**
 * Internal, defensive coercion helpers used by model deserialization.
 *
 * Deserialization must never crash on unexpected/extra fields, so every
 * helper degrades to a sensible default instead of throwing.
 *
 * @internal
 */
final class Coerce
{
    /** @param mixed $v */
    public static function intOrNull($v): ?int
    {
        if (is_int($v)) {
            return $v;
        }
        if (is_float($v)) {
            return (int) $v;
        }
        if (is_string($v) && is_numeric($v)) {
            return (int) $v;
        }
        return null;
    }

    /** @param mixed $v */
    public static function intOr($v, int $default): int
    {
        return self::intOrNull($v) ?? $default;
    }

    /** @param mixed $v */
    public static function stringOrNull($v): ?string
    {
        return is_string($v) ? $v : null;
    }

    /** @param mixed $v */
    public static function stringOr($v, string $default): string
    {
        return is_string($v) ? $v : $default;
    }

    /** @param mixed $v */
    public static function boolOr($v, bool $default): bool
    {
        return is_bool($v) ? $v : $default;
    }

    /**
     * @param mixed $v
     * @return array<int|string,mixed>
     */
    public static function arrayOr($v, array $default = []): array
    {
        return is_array($v) ? $v : $default;
    }

    /**
     * @param mixed $v
     * @return list<string>
     */
    public static function stringList($v): array
    {
        if (!is_array($v)) {
            return [];
        }
        $out = [];
        foreach ($v as $item) {
            if (is_string($item)) {
                $out[] = $item;
            }
        }
        return $out;
    }
}
