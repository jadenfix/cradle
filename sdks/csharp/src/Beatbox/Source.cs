using System.Text.Json.Serialization;

namespace Beatbox;

/// <summary>
/// Program source for an execution. A tagged union discriminated by
/// <see cref="Kind"/>; construct one with the factory methods rather than setting
/// fields directly. Only the field relevant to the chosen kind is serialized.
/// </summary>
public sealed record Source
{
    /// <summary>
    /// Discriminator: one of <c>inline</c>, <c>wasm_file</c>, <c>wasm_wat</c>,
    /// <c>wasm_bytes_base64</c>, or <c>module_ref</c>.
    /// </summary>
    [JsonPropertyName("kind")]
    public string Kind { get; init; } = "";

    /// <summary>Inline source code (kind <c>inline</c>).</summary>
    [JsonPropertyName("code")]
    public string? Code { get; init; }

    /// <summary>Path to a wasm module on the host (kind <c>wasm_file</c>).</summary>
    [JsonPropertyName("path")]
    public string? Path { get; init; }

    /// <summary>WebAssembly text (kind <c>wasm_wat</c>).</summary>
    [JsonPropertyName("text")]
    public string? Text { get; init; }

    /// <summary>Base64-encoded wasm module bytes (kind <c>wasm_bytes_base64</c>).</summary>
    [JsonPropertyName("bytes")]
    public string? Bytes { get; init; }

    /// <summary>Content-addressed module reference (kind <c>module_ref</c>).</summary>
    [JsonPropertyName("sha256")]
    public string? Sha256 { get; init; }

    /// <summary>Inline source code, interpreted by the target lane.</summary>
    public static Source Inline(string code) => new() { Kind = "inline", Code = code };

    /// <summary>A wasm module already present on the host filesystem.</summary>
    public static Source WasmFile(string path) => new() { Kind = "wasm_file", Path = path };

    /// <summary>WebAssembly text (WAT) compiled by the daemon.</summary>
    public static Source WasmWat(string text) => new() { Kind = "wasm_wat", Text = text };

    /// <summary>A base64-encoded wasm binary.</summary>
    public static Source WasmBytesBase64(string bytes) => new() { Kind = "wasm_bytes_base64", Bytes = bytes };

    /// <summary>A previously uploaded module referenced by its sha256 digest.</summary>
    public static Source ModuleRef(string sha256) => new() { Kind = "module_ref", Sha256 = sha256 };
}
