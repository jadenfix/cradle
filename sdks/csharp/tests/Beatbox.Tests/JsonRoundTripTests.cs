using System.Text.Json;
using Beatbox;
using Xunit;

namespace Beatbox.Tests;

public class JsonRoundTripTests
{
    private static JsonSerializerOptions Options => BeatboxJson.Options;

    [Fact]
    public void ExecuteRequest_serializes_wire_names_and_enums()
    {
        var request = ExecuteRequest.WasmWat(
            "(module)", input: new { n = 41 }, entrypoint: "run");

        var json = JsonSerializer.Serialize(request, Options);

        Assert.Contains("\"lane\":\"wasm\"", json);
        Assert.Contains("\"kind\":\"wasm_wat\"", json);
        Assert.Contains("\"text\":\"(module)\"", json);
        Assert.Contains("\"entrypoint\":\"run\"", json);
        Assert.Contains("\"input\":{\"n\":41}", json);
        // Optional, unset fields are omitted (WhenWritingNull).
        Assert.DoesNotContain("stdin", json);
        Assert.DoesNotContain("idempotency_key", json);
        Assert.DoesNotContain("policy", json);
    }

    [Fact]
    public void ExecuteRequest_round_trips()
    {
        var original = new ExecuteRequest
        {
            Lane = Lane.PythonWasi,
            Source = Source.Inline("print(1)"),
            Stdin = "hello",
            IdempotencyKey = "step-1",
            Policy = new Policy { Limits = new Limits { WallMs = 5000 } },
        };

        var json = JsonSerializer.Serialize(original, Options);
        var back = JsonSerializer.Deserialize<ExecuteRequest>(json, Options);

        Assert.NotNull(back);
        Assert.Equal(Lane.PythonWasi, back!.Lane);
        Assert.Equal("inline", back.Source.Kind);
        Assert.Equal("print(1)", back.Source.Code);
        Assert.Equal("hello", back.Stdin);
        Assert.Equal("step-1", back.IdempotencyKey);
        Assert.Equal(5000, back.Policy!.Limits!.WallMs);
    }

    [Fact]
    public void Limits_omits_unset_fields()
    {
        var json = JsonSerializer.Serialize(new Limits { WallMs = 1000 }, Options);

        Assert.Contains("\"wall_ms\":1000", json);
        Assert.DoesNotContain("cpu_ms", json);
        Assert.DoesNotContain("memory_bytes", json);
        Assert.DoesNotContain("fuel", json);
    }

    [Fact]
    public void ExecutionResult_deserializes_and_tolerates_unknown_fields()
    {
        const string body = """
        {
          "status": "ok",
          "value": 42,
          "stdout": "",
          "stdout_truncated": false,
          "stderr": "",
          "stderr_truncated": false,
          "error": null,
          "exit_code": null,
          "metrics": {
            "wall_time_ms": 12,
            "cpu_time_ms": null,
            "fuel_used": 7,
            "peak_memory_bytes": null
          },
          "lane": "wasm",
          "deterministic": true,
          "inputs_digest": "sha256:abc",
          "engine_version": "w0-1.2.3",
          "beatbox_version": "0.1.0",
          "effective_isolation": {
            "os": "linux",
            "mechanisms": ["seccomp", "landlock"],
            "downgrades": [],
            "landlock_abi": 4
          },
          "egress": [],
          "some_future_field": {"nested": true}
        }
        """;

        var result = JsonSerializer.Deserialize<ExecutionResult>(body, Options);

        Assert.NotNull(result);
        Assert.Equal(ExecutionStatus.Ok, result!.Status);
        Assert.NotNull(result.Value);
        Assert.Equal(JsonValueKind.Number, result.Value!.Value.ValueKind);
        Assert.Equal(42, result.Value.Value.GetInt64());
        Assert.Equal(12, result.Metrics.WallTimeMs);
        Assert.Null(result.Metrics.CpuTimeMs);
        Assert.Equal(7, result.Metrics.FuelUsed);
        Assert.Null(result.Metrics.PeakMemoryBytes);
        Assert.Equal(Lane.Wasm, result.Lane);
        Assert.True(result.Deterministic);
        Assert.Equal(4, result.EffectiveIsolation.LandlockAbi);
        Assert.Equal(2, result.EffectiveIsolation.Mechanisms.Count);
    }

    [Fact]
    public void JobRecord_deserializes_nested_request_and_result()
    {
        const string body = """
        {
          "job_id": "11111111-2222-3333-4444-555555555555",
          "status": "succeeded",
          "request": {
            "lane": "wasm",
            "source": {"kind": "wasm_wat", "text": "(module)"}
          },
          "result": {
            "status": "ok",
            "value": null,
            "stdout": "",
            "stdout_truncated": false,
            "stderr": "",
            "stderr_truncated": false,
            "metrics": {"wall_time_ms": 3},
            "lane": "wasm",
            "deterministic": false,
            "inputs_digest": "",
            "engine_version": "",
            "beatbox_version": "",
            "effective_isolation": {"os": "linux", "mechanisms": [], "downgrades": []},
            "egress": []
          },
          "error": null,
          "created_at": "2026-07-03T00:00:00Z",
          "updated_at": "2026-07-03T00:00:01Z"
        }
        """;

        var job = JsonSerializer.Deserialize<JobRecord>(body, Options);

        Assert.NotNull(job);
        Assert.Equal(JobStatus.Succeeded, job!.Status);
        Assert.Equal(Lane.Wasm, job.Request.Lane);
        Assert.Equal("wasm_wat", job.Request.Source.Kind);
        Assert.NotNull(job.Result);
        Assert.Equal(ExecutionStatus.Ok, job.Result!.Status);
        Assert.Equal(3, job.Result.Metrics.WallTimeMs);
    }
}
