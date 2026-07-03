<?php

declare(strict_types=1);

namespace Beatbox;

/**
 * Execution lane. Wire values are snake_case.
 */
enum Lane: string
{
    case Wasm = 'wasm';
    case PythonWasi = 'python_wasi';
    case PythonNative = 'python_native';
    case JsWasm = 'js_wasm';
    case JsNative = 'js_native';
    case Exec = 'exec';
}
