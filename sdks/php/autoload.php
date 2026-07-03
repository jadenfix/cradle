<?php

declare(strict_types=1);

/**
 * Standalone PSR-4 autoloader for the Beatbox namespace.
 *
 * Use this when running the examples/tests without Composer. When the SDK
 * is installed via Composer, require `vendor/autoload.php` instead.
 */
spl_autoload_register(static function (string $class): void {
    $prefix = 'Beatbox\\';
    if (!str_starts_with($class, $prefix)) {
        return;
    }
    $relative = substr($class, strlen($prefix));
    $path = __DIR__ . '/src/' . str_replace('\\', '/', $relative) . '.php';
    if (is_file($path)) {
        require $path;
    }
});
