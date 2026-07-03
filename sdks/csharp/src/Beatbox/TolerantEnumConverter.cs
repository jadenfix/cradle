using System.Text.Json;
using System.Text.Json.Serialization;

namespace Beatbox;

/// <summary>
/// Serializes an enum to its snake_case wire form, and — for forward
/// compatibility — deserializes an <em>unrecognized</em> wire value to the
/// enum's <c>Unknown</c> member instead of throwing. A newer daemon can add a
/// lane or status without breaking an older client, matching the other beatbox
/// SDKs, which degrade rather than crash on unknown enum values.
/// </summary>
/// <remarks>
/// Every enum registered with this converter must declare an <c>Unknown</c>
/// member. Serializing <c>Unknown</c> throws, so a client can never send a value
/// the server would not understand — it only exists to absorb values coming the
/// other way.
/// </remarks>
internal sealed class TolerantEnumConverter<TEnum> : JsonConverter<TEnum>
    where TEnum : struct, Enum
{
    private const string UnknownName = "Unknown";

    // Reuse the framework's snake_case policy so the mapping is byte-identical to
    // what JsonNamingPolicy.SnakeCaseLower would have produced.
    private static readonly JsonNamingPolicy Policy = JsonNamingPolicy.SnakeCaseLower;
    private static readonly TEnum UnknownValue = Enum.Parse<TEnum>(UnknownName);
    private static readonly Dictionary<string, TEnum> FromWire = BuildFromWire();
    private static readonly Dictionary<TEnum, string> ToWire = BuildToWire();

    private static Dictionary<string, TEnum> BuildFromWire()
    {
        var map = new Dictionary<string, TEnum>(StringComparer.Ordinal);
        foreach (var name in Enum.GetNames<TEnum>())
        {
            if (name == UnknownName)
            {
                continue;
            }
            map[Policy.ConvertName(name)] = Enum.Parse<TEnum>(name);
        }
        return map;
    }

    private static Dictionary<TEnum, string> BuildToWire()
    {
        var map = new Dictionary<TEnum, string>();
        foreach (var name in Enum.GetNames<TEnum>())
        {
            if (name == UnknownName)
            {
                continue;
            }
            map[Enum.Parse<TEnum>(name)] = Policy.ConvertName(name);
        }
        return map;
    }

    public override TEnum Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        if (reader.TokenType == JsonTokenType.String)
        {
            var raw = reader.GetString();
            if (raw is not null && FromWire.TryGetValue(raw, out var value))
            {
                return value;
            }
        }
        // Anything unrecognized (a future enum value, or an unexpected token)
        // degrades to Unknown rather than throwing.
        return UnknownValue;
    }

    public override void Write(Utf8JsonWriter writer, TEnum value, JsonSerializerOptions options)
    {
        if (ToWire.TryGetValue(value, out var wire))
        {
            writer.WriteStringValue(wire);
            return;
        }
        throw new JsonException(
            $"cannot serialize the sentinel {typeof(TEnum).Name}.{UnknownName} value; " +
            "it only exists to absorb enum values from a newer server");
    }
}
