using System.Text.Json;
using Beatbox;
using Xunit;

namespace Beatbox.Tests;

public class EnumToleranceTests
{
    private static JsonSerializerOptions Options => BeatboxJson.Options;

    [Fact]
    public void Known_enum_values_round_trip_as_snake_case()
    {
        Assert.Equal("\"python_wasi\"", JsonSerializer.Serialize(Lane.PythonWasi, Options));
        Assert.Equal("\"timeout\"", JsonSerializer.Serialize(ExecutionStatus.Timeout, Options));
        Assert.Equal("\"succeeded\"", JsonSerializer.Serialize(JobStatus.Succeeded, Options));

        Assert.Equal(Lane.JsWasm, JsonSerializer.Deserialize<Lane>("\"js_wasm\"", Options));
        Assert.Equal(ExecutionStatus.Denied, JsonSerializer.Deserialize<ExecutionStatus>("\"denied\"", Options));
        Assert.Equal(JobStatus.Canceled, JsonSerializer.Deserialize<JobStatus>("\"canceled\"", Options));
    }

    [Fact]
    public void Unknown_wire_value_degrades_to_Unknown_instead_of_throwing()
    {
        Assert.Equal(Lane.Unknown, JsonSerializer.Deserialize<Lane>("\"quantum\"", Options));
        Assert.Equal(ExecutionStatus.Unknown, JsonSerializer.Deserialize<ExecutionStatus>("\"teleported\"", Options));
        Assert.Equal(JobStatus.Unknown, JsonSerializer.Deserialize<JobStatus>("\"paused\"", Options));
        Assert.Equal(MountMode.Unknown, JsonSerializer.Deserialize<MountMode>("\"append\"", Options));
        Assert.Equal(SecretExpose.Unknown, JsonSerializer.Deserialize<SecretExpose>("\"vault\"", Options));
    }

    [Fact]
    public void Unknown_status_on_a_full_result_does_not_throw()
    {
        // A newer daemon returns a status this client has never heard of.
        const string body = """
        {
          "status": "quarantined",
          "value": null,
          "stdout": "",
          "stdout_truncated": false,
          "stderr": "",
          "stderr_truncated": false,
          "metrics": {"wall_time_ms": 1},
          "lane": "wasm",
          "deterministic": false,
          "inputs_digest": "",
          "engine_version": "",
          "beatbox_version": "",
          "effective_isolation": {"os": "linux", "mechanisms": [], "downgrades": []},
          "egress": []
        }
        """;

        var result = JsonSerializer.Deserialize<ExecutionResult>(body, Options);

        Assert.NotNull(result);
        Assert.Equal(ExecutionStatus.Unknown, result!.Status);
        Assert.Equal(Lane.Wasm, result.Lane);
    }

    [Fact]
    public void Serializing_the_Unknown_sentinel_throws()
    {
        // Unknown only absorbs values coming from the server; it must never be sent.
        Assert.Throws<JsonException>(() => JsonSerializer.Serialize(Lane.Unknown, Options));
        Assert.Throws<JsonException>(() => JsonSerializer.Serialize(ExecutionStatus.Unknown, Options));
        Assert.Throws<JsonException>(() => JsonSerializer.Serialize(JobStatus.Unknown, Options));
    }
}
