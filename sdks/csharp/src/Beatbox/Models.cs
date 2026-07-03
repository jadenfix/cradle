using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Beatbox;

/// <summary>
/// Request for a synchronous <c>execute</c> or an asynchronous <c>create_job</c>.
/// Use the factory helpers (e.g. <see cref="WasmWat"/>) for the common case.
/// </summary>
public sealed record ExecuteRequest
{
    /// <summary>Execution lane (required).</summary>
    [JsonPropertyName("lane")]
    public Lane Lane { get; init; }

    /// <summary>Program source (required).</summary>
    [JsonPropertyName("source")]
    public Source Source { get; init; } = new();

    /// <summary>Optional entrypoint / exported function name.</summary>
    [JsonPropertyName("entrypoint")]
    public string? Entrypoint { get; init; }

    /// <summary>
    /// Optional input, any JSON value (object, array, number, string, boolean, or
    /// null). Serialized as-is; on read it deserializes to a <see cref="JsonElement"/>.
    /// </summary>
    [JsonPropertyName("input")]
    public object? Input { get; init; }

    /// <summary>Optional stdin passed to the program.</summary>
    [JsonPropertyName("stdin")]
    public string? Stdin { get; init; }

    /// <summary>Optional execution policy (partial; merges onto defaults).</summary>
    [JsonPropertyName("policy")]
    public Policy? Policy { get; init; }

    /// <summary>Optional idempotency key.</summary>
    [JsonPropertyName("idempotency_key")]
    public string? IdempotencyKey { get; init; }

    /// <summary>Build a wasm request from WebAssembly text (WAT).</summary>
    public static ExecuteRequest WasmWat(string text, object? input = null, string? entrypoint = null)
        => new()
        {
            Lane = Lane.Wasm,
            Source = Source.WasmWat(text),
            Input = input,
            Entrypoint = entrypoint,
        };

    /// <summary>Build a wasm request from a base64-encoded wasm binary.</summary>
    public static ExecuteRequest WasmBytesBase64(string bytes, object? input = null, string? entrypoint = null)
        => new()
        {
            Lane = Lane.Wasm,
            Source = Source.WasmBytesBase64(bytes),
            Input = input,
            Entrypoint = entrypoint,
        };
}

/// <summary>Resource usage measured for an execution.</summary>
public sealed record Metrics
{
    /// <summary>Wall-clock time in milliseconds (always present).</summary>
    [JsonPropertyName("wall_time_ms")]
    public long WallTimeMs { get; init; }

    /// <summary>
    /// CPU time in milliseconds, when the lane measures it separately from wall
    /// time. The W0 wasm lane does not, so this is <see langword="null"/> there —
    /// use <see cref="FuelUsed"/> as the deterministic compute signal.
    /// </summary>
    [JsonPropertyName("cpu_time_ms")]
    public long? CpuTimeMs { get; init; }

    /// <summary>Wasm fuel consumed, when the lane reports it.</summary>
    [JsonPropertyName("fuel_used")]
    public long? FuelUsed { get; init; }

    /// <summary>Peak memory in bytes, when the lane reports it.</summary>
    [JsonPropertyName("peak_memory_bytes")]
    public long? PeakMemoryBytes { get; init; }
}

/// <summary>Machine-readable error from the daemon.</summary>
public sealed record ErrorBody
{
    /// <summary>Stable error code.</summary>
    [JsonPropertyName("code")]
    public string Code { get; init; } = "";

    /// <summary>Human-readable message.</summary>
    [JsonPropertyName("message")]
    public string Message { get; init; } = "";
}

/// <summary>Envelope wrapping an <see cref="ErrorBody"/> in error responses.</summary>
public sealed record ErrorResponse
{
    /// <summary>The error payload.</summary>
    [JsonPropertyName("error")]
    public ErrorBody? Error { get; init; }
}

/// <summary>The sandbox isolation actually applied to a run.</summary>
public sealed record EffectiveIsolation
{
    /// <summary>Host operating system.</summary>
    [JsonPropertyName("os")]
    public string Os { get; init; } = "";

    /// <summary>Isolation mechanisms that were engaged.</summary>
    [JsonPropertyName("mechanisms")]
    public List<string> Mechanisms { get; init; } = new();

    /// <summary>Isolation features that were downgraded or unavailable.</summary>
    [JsonPropertyName("downgrades")]
    public List<string> Downgrades { get; init; } = new();

    /// <summary>Landlock ABI version, when applicable.</summary>
    [JsonPropertyName("landlock_abi")]
    public int? LandlockAbi { get; init; }
}

/// <summary>A record of network egress observed during a run.</summary>
public sealed record EgressRecord
{
    /// <summary>Destination domain.</summary>
    [JsonPropertyName("domain")]
    public string Domain { get; init; } = "";

    /// <summary>Destination port.</summary>
    [JsonPropertyName("port")]
    public int Port { get; init; }

    /// <summary>Bytes transferred.</summary>
    [JsonPropertyName("bytes")]
    public long Bytes { get; init; }
}

/// <summary>Result of a synchronous or completed execution.</summary>
public sealed record ExecutionResult
{
    /// <summary>Terminal status.</summary>
    [JsonPropertyName("status")]
    public ExecutionStatus Status { get; init; }

    /// <summary>
    /// The program's return value, any JSON value. Present as a
    /// <see cref="JsonElement"/>; inspect its <see cref="JsonElement.ValueKind"/>.
    /// </summary>
    [JsonPropertyName("value")]
    public JsonElement? Value { get; init; }

    /// <summary>Captured standard output.</summary>
    [JsonPropertyName("stdout")]
    public string Stdout { get; init; } = "";

    /// <summary>Whether <see cref="Stdout"/> was truncated.</summary>
    [JsonPropertyName("stdout_truncated")]
    public bool StdoutTruncated { get; init; }

    /// <summary>Captured standard error.</summary>
    [JsonPropertyName("stderr")]
    public string Stderr { get; init; } = "";

    /// <summary>Whether <see cref="Stderr"/> was truncated.</summary>
    [JsonPropertyName("stderr_truncated")]
    public bool StderrTruncated { get; init; }

    /// <summary>Error detail when <see cref="Status"/> is not <see cref="ExecutionStatus.Ok"/>.</summary>
    [JsonPropertyName("error")]
    public ErrorBody? Error { get; init; }

    /// <summary>Process exit code, when the lane reports one.</summary>
    [JsonPropertyName("exit_code")]
    public int? ExitCode { get; init; }

    /// <summary>Resource usage metrics.</summary>
    [JsonPropertyName("metrics")]
    public Metrics Metrics { get; init; } = new();

    /// <summary>Lane that ran the program.</summary>
    [JsonPropertyName("lane")]
    public Lane Lane { get; init; }

    /// <summary>Whether the run was deterministic.</summary>
    [JsonPropertyName("deterministic")]
    public bool Deterministic { get; init; }

    /// <summary>Digest of the resolved inputs.</summary>
    [JsonPropertyName("inputs_digest")]
    public string InputsDigest { get; init; } = "";

    /// <summary>Engine version string.</summary>
    [JsonPropertyName("engine_version")]
    public string EngineVersion { get; init; } = "";

    /// <summary>Beatbox daemon version string.</summary>
    [JsonPropertyName("beatbox_version")]
    public string BeatboxVersion { get; init; } = "";

    /// <summary>Isolation actually applied.</summary>
    [JsonPropertyName("effective_isolation")]
    public EffectiveIsolation EffectiveIsolation { get; init; } = new();

    /// <summary>Observed network egress.</summary>
    [JsonPropertyName("egress")]
    public List<EgressRecord> Egress { get; init; } = new();
}

/// <summary>Response from <c>create_job</c>.</summary>
public sealed record CreateJobResponse
{
    /// <summary>Identifier of the created job.</summary>
    [JsonPropertyName("job_id")]
    public string JobId { get; init; } = "";
}

/// <summary>Full record of an asynchronous job.</summary>
public sealed record JobRecord
{
    /// <summary>Job identifier (a UUID).</summary>
    [JsonPropertyName("job_id")]
    public string JobId { get; init; } = "";

    /// <summary>Current lifecycle status.</summary>
    [JsonPropertyName("status")]
    public JobStatus Status { get; init; }

    /// <summary>The original request.</summary>
    [JsonPropertyName("request")]
    public ExecuteRequest Request { get; init; } = new();

    /// <summary>The result, once the job has succeeded.</summary>
    [JsonPropertyName("result")]
    public ExecutionResult? Result { get; init; }

    /// <summary>Error detail, when the job failed.</summary>
    [JsonPropertyName("error")]
    public ErrorBody? Error { get; init; }

    /// <summary>Creation timestamp (RFC 3339 string).</summary>
    [JsonPropertyName("created_at")]
    public string CreatedAt { get; init; } = "";

    /// <summary>Last-update timestamp (RFC 3339 string).</summary>
    [JsonPropertyName("updated_at")]
    public string UpdatedAt { get; init; } = "";
}
