namespace Beatbox;

// Enum members serialize to their snake_case wire form via the
// JsonStringEnumConverter registered with a snake_case naming policy in
// BeatboxJson.Options. Do not annotate these enums with [JsonConverter]: an
// explicit converter attribute would bypass that naming policy.

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
}

/// <summary>Mount access mode.</summary>
public enum MountMode
{
    /// <summary><c>ro</c></summary>
    Ro,

    /// <summary><c>rw</c></summary>
    Rw,
}

/// <summary>How a secret is exposed to the guest.</summary>
public enum SecretExpose
{
    /// <summary><c>env</c></summary>
    Env,

    /// <summary><c>file</c></summary>
    File,
}
