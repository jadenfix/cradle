using System.Collections.Generic;
using System.Text.Json.Serialization;

namespace Beatbox;

/// <summary>
/// Resource limits for an execution. All fields are optional; a partial
/// <see cref="Limits"/> merges onto the daemon's defaults, so only the values you
/// set are sent (unset values are omitted from the JSON).
/// </summary>
public sealed record Limits
{
    /// <summary>Wall-clock time budget in milliseconds.</summary>
    [JsonPropertyName("wall_ms")]
    public long? WallMs { get; init; }

    /// <summary>CPU time budget in milliseconds.</summary>
    [JsonPropertyName("cpu_ms")]
    public long? CpuMs { get; init; }

    /// <summary>Wasm fuel budget.</summary>
    [JsonPropertyName("fuel")]
    public long? Fuel { get; init; }

    /// <summary>Memory budget in bytes.</summary>
    [JsonPropertyName("memory_bytes")]
    public long? MemoryBytes { get; init; }

    /// <summary>Disk budget in bytes.</summary>
    [JsonPropertyName("disk_bytes")]
    public long? DiskBytes { get; init; }

    /// <summary>Maximum captured output in bytes.</summary>
    [JsonPropertyName("output_bytes")]
    public long? OutputBytes { get; init; }

    /// <summary>Maximum number of processes/threads.</summary>
    [JsonPropertyName("pids")]
    public int? Pids { get; init; }
}

/// <summary>Determinism configuration (tagged union on <see cref="Kind"/>).</summary>
public sealed record Determinism
{
    /// <summary>Discriminator: <c>off</c> or <c>seeded</c>.</summary>
    [JsonPropertyName("kind")]
    public string Kind { get; init; } = "";

    /// <summary>Deterministic seed (kind <c>seeded</c>).</summary>
    [JsonPropertyName("seed")]
    public long? Seed { get; init; }

    /// <summary>Simulated wall-clock epoch in milliseconds (kind <c>seeded</c>).</summary>
    [JsonPropertyName("epoch_ms")]
    public long? EpochMs { get; init; }

    /// <summary>Non-deterministic execution.</summary>
    public static Determinism Off() => new() { Kind = "off" };

    /// <summary>Deterministic execution seeded with the given values.</summary>
    public static Determinism Seeded(long seed, long epochMs)
        => new() { Kind = "seeded", Seed = seed, EpochMs = epochMs };
}

/// <summary>A single host-to-guest filesystem mount.</summary>
public sealed record Mount
{
    /// <summary>Host path.</summary>
    [JsonPropertyName("host")]
    public string Host { get; init; } = "";

    /// <summary>Guest path.</summary>
    [JsonPropertyName("guest")]
    public string Guest { get; init; } = "";

    /// <summary>Access mode.</summary>
    [JsonPropertyName("mode")]
    public MountMode Mode { get; init; }
}

/// <summary>Filesystem policy.</summary>
public sealed record FsPolicy
{
    /// <summary>Workspace directory inside the guest.</summary>
    [JsonPropertyName("workspace")]
    public string? Workspace { get; init; }

    /// <summary>Extra mounts exposed to the guest.</summary>
    [JsonPropertyName("mounts")]
    public List<Mount>? Mounts { get; init; }
}

/// <summary>Network policy (tagged union on <see cref="Kind"/>).</summary>
public sealed record NetPolicy
{
    /// <summary>Discriminator: <c>deny</c> or <c>proxy</c>.</summary>
    [JsonPropertyName("kind")]
    public string Kind { get; init; } = "";

    /// <summary>Allowed domains (kind <c>proxy</c>).</summary>
    [JsonPropertyName("allow_domains")]
    public List<string>? AllowDomains { get; init; }

    /// <summary>Allowed ports (kind <c>proxy</c>).</summary>
    [JsonPropertyName("allow_ports")]
    public List<int>? AllowPorts { get; init; }

    /// <summary>Deny all network egress.</summary>
    public static NetPolicy Deny() => new() { Kind = "deny" };

    /// <summary>Allow egress through the proxy to the given domains and ports.</summary>
    public static NetPolicy Proxy(List<string>? allowDomains = null, List<int>? allowPorts = null)
        => new() { Kind = "proxy", AllowDomains = allowDomains, AllowPorts = allowPorts };
}

/// <summary>A secret made available to the guest.</summary>
public sealed record Secret
{
    /// <summary>Secret name.</summary>
    [JsonPropertyName("name")]
    public string Name { get; init; } = "";

    /// <summary>Reference to the secret's value in the daemon's store.</summary>
    [JsonPropertyName("value_ref")]
    public string ValueRef { get; init; } = "";

    /// <summary>How the secret is exposed to the guest.</summary>
    [JsonPropertyName("expose")]
    public SecretExpose Expose { get; init; }
}

/// <summary>
/// Execution policy. Every field is optional; a partial policy merges onto the
/// daemon's defaults.
/// </summary>
public sealed record Policy
{
    /// <summary>Resource limits.</summary>
    [JsonPropertyName("limits")]
    public Limits? Limits { get; init; }

    /// <summary>Determinism configuration.</summary>
    [JsonPropertyName("determinism")]
    public Determinism? Determinism { get; init; }

    /// <summary>Whether to apply an extra jail layer.</summary>
    [JsonPropertyName("double_jail")]
    public bool? DoubleJail { get; init; }

    /// <summary>Environment variables exposed to the guest.</summary>
    [JsonPropertyName("env")]
    public Dictionary<string, string>? Env { get; init; }

    /// <summary>Filesystem policy.</summary>
    [JsonPropertyName("fs")]
    public FsPolicy? Fs { get; init; }

    /// <summary>Network policy.</summary>
    [JsonPropertyName("net")]
    public NetPolicy? Net { get; init; }

    /// <summary>Secrets exposed to the guest.</summary>
    [JsonPropertyName("secrets")]
    public List<Secret>? Secrets { get; init; }
}
