namespace Beatbox;

// Enum members serialize to their snake_case wire form via TolerantEnumConverter,
// registered per type in BeatboxJson.Options. That converter also degrades an
// unrecognized wire value to the enum's `Unknown` member instead of throwing, so
// a newer server can add values without breaking this client. Each enum therefore
// ends with an `Unknown` sentinel (which is never sent — serializing it throws).
// Do not annotate these enums with [JsonConverter]: an attribute would bypass the
// registered converter and its snake_case + tolerant behavior.

/// <summary>Execution lane requested for a run.</summary>
public enum Lane
{
    /// <summary><c>wasm</c></summary>
    Wasm,

    /// <summary><c>python_wasi</c></summary>
    PythonWasi,

    /// <summary><c>python_native</c></summary>
    PythonNative,

    /// <summary><c>js_wasm</c></summary>
    JsWasm,

    /// <summary><c>js_native</c></summary>
    JsNative,

    /// <summary><c>exec</c></summary>
    Exec,

    /// <summary>
    /// A lane value returned by a newer server that this client does not
    /// recognize. Deserialization degrades to this instead of throwing;
    /// serializing it throws.
    /// </summary>
    Unknown,
}

/// <summary>Terminal status of a single execution.</summary>
public enum ExecutionStatus
{
    /// <summary><c>ok</c></summary>
    Ok,

    /// <summary><c>error</c></summary>
    Error,

    /// <summary><c>timeout</c></summary>
    Timeout,

    /// <summary><c>oom</c></summary>
    Oom,

    /// <summary><c>killed</c></summary>
    Killed,

    /// <summary><c>denied</c></summary>
    Denied,

    /// <summary>
    /// A status returned by a newer server that this client does not recognize.
    /// Deserialization degrades to this instead of throwing; serializing it throws.
    /// </summary>
    Unknown,
}

/// <summary>Lifecycle status of an asynchronous job.</summary>
public enum JobStatus
{
    /// <summary><c>queued</c></summary>
    Queued,

    /// <summary><c>running</c></summary>
    Running,

    /// <summary><c>succeeded</c></summary>
    Succeeded,

    /// <summary><c>failed</c></summary>
    Failed,

    /// <summary><c>canceled</c></summary>
    Canceled,

    /// <summary>
    /// A status returned by a newer server that this client does not recognize.
    /// Deserialization degrades to this instead of throwing; serializing it throws.
    /// </summary>
    Unknown,
}

/// <summary>Mount access mode.</summary>
public enum MountMode
{
    /// <summary><c>ro</c></summary>
    Ro,

    /// <summary><c>rw</c></summary>
    Rw,

    /// <summary>
    /// An unrecognized mode from a newer server (e.g. echoed back in a job's
    /// request). Deserialization degrades to this instead of throwing;
    /// serializing it throws.
    /// </summary>
    Unknown,
}

/// <summary>How a secret is exposed to the guest.</summary>
public enum SecretExpose
{
    /// <summary><c>env</c></summary>
    Env,

    /// <summary><c>file</c></summary>
    File,

    /// <summary>
    /// An unrecognized exposure from a newer server (e.g. echoed back in a job's
    /// request). Deserialization degrades to this instead of throwing;
    /// serializing it throws.
    /// </summary>
    Unknown,
}
