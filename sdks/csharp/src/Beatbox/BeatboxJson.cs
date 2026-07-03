using System.Text.Json;
using System.Text.Json.Serialization;

namespace Beatbox;

/// <summary>
/// Shared <see cref="JsonSerializerOptions"/> used by the SDK. Field names on the
/// wire are snake_case; enums serialize to their snake_case string form and an
/// unrecognized enum value from a newer server degrades to <c>Unknown</c> rather
/// than throwing; unknown members are ignored on read for forward compatibility;
/// and <see langword="null"/> values are omitted when writing so partial payloads
/// (e.g. merged policy limits) stay minimal.
/// </summary>
public static class BeatboxJson
{
    /// <summary>The canonical serializer options for beatbox wire types.</summary>
    public static JsonSerializerOptions Options { get; } = Create();

    private static JsonSerializerOptions Create()
    {
        var options = new JsonSerializerOptions
        {
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
            // No DictionaryKeyPolicy: dictionary keys (e.g. env var names) are
            // arbitrary user data and must pass through verbatim.
            PropertyNameCaseInsensitive = true,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            // System.Text.Json ignores unknown members on read by default; being
            // explicit documents the forward-compatibility guarantee.
            UnmappedMemberHandling = JsonUnmappedMemberHandling.Skip,
        };
        // Tolerant, per-enum converters: snake_case on the wire, and an
        // unrecognized value degrades to the enum's Unknown member instead of
        // throwing (forward compatibility with a newer daemon).
        options.Converters.Add(new TolerantEnumConverter<Lane>());
        options.Converters.Add(new TolerantEnumConverter<ExecutionStatus>());
        options.Converters.Add(new TolerantEnumConverter<JobStatus>());
        options.Converters.Add(new TolerantEnumConverter<MountMode>());
        options.Converters.Add(new TolerantEnumConverter<SecretExpose>());
        return options;
    }
}
