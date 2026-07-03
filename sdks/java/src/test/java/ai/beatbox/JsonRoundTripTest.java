package ai.beatbox;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import ai.beatbox.internal.Json;
import ai.beatbox.model.ExecuteRequest;
import ai.beatbox.model.ExecutionResult;
import ai.beatbox.model.ExecutionStatus;
import ai.beatbox.model.Lane;
import ai.beatbox.model.Limits;
import ai.beatbox.model.Policy;
import ai.beatbox.model.Source;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.util.Map;
import org.junit.jupiter.api.Test;

/** Wire-format (de)serialization: snake_case names, tagged source, nullable metrics, forward-compat. */
class JsonRoundTripTest {

    private final ObjectMapper mapper = Json.newMapper();

    @Test
    void executeRequestSerializesWithSnakeCaseAndTaggedSource() throws Exception {
        ExecuteRequest request = ExecuteRequest.builder(Lane.WASM, Source.wasmWat("(module)"))
                .input(Map.of("n", 41))
                .idempotencyKey("step-1")
                .policy(Policy.withLimits(Limits.wallMs(5000)))
                .build();

        JsonNode json = mapper.valueToTree(request);

        assertEquals("wasm", json.get("lane").asText());
        assertEquals("wasm_wat", json.get("source").get("kind").asText());
        assertEquals("(module)", json.get("source").get("text").asText());
        assertEquals("step-1", json.get("idempotency_key").asText());
        assertEquals(5000, json.get("policy").get("limits").get("wall_ms").asLong());
        // Null-valued optionals are omitted, not serialized as null.
        assertFalse(json.has("entrypoint"));
        assertFalse(json.has("stdin"));
    }

    @Test
    void executeRequestRoundTrips() throws Exception {
        ExecuteRequest request = ExecuteRequest.builder(Lane.WASM, Source.wasmWat("(module)"))
                .input(Map.of("n", 41))
                .policy(Policy.withLimits(Limits.wallMs(5000)))
                .build();

        byte[] bytes = mapper.writeValueAsBytes(request);
        ExecuteRequest back = mapper.readValue(bytes, ExecuteRequest.class);

        assertEquals(request, back);
        assertInstanceOf(Source.WasmWat.class, back.source());
    }

    @Test
    void executionResultParsesWithNullableMetricsAndUnknownFields() throws Exception {
        // cpu_time_ms is null on the wasm lane; an unknown field must not break parsing.
        String body = "{"
                + "\"status\":\"ok\","
                + "\"value\":42,"
                + "\"stdout\":\"\",\"stdout_truncated\":false,"
                + "\"stderr\":\"\",\"stderr_truncated\":false,"
                + "\"metrics\":{\"wall_time_ms\":12,\"cpu_time_ms\":null,\"fuel_used\":9001,\"peak_memory_bytes\":null},"
                + "\"lane\":\"wasm\",\"deterministic\":true,"
                + "\"inputs_digest\":\"sha256:abc\",\"engine_version\":\"w0\",\"beatbox_version\":\"0.1.0\","
                + "\"effective_isolation\":{\"os\":\"linux\",\"mechanisms\":[\"seccomp\"],\"downgrades\":[]},"
                + "\"egress\":[],"
                + "\"a_new_server_field\":\"ignored\""
                + "}";

        ExecutionResult result = mapper.readValue(body, ExecutionResult.class);

        assertEquals(ExecutionStatus.OK, result.status());
        assertTrue(result.isOk());
        assertEquals(42, ((Number) result.value()).intValue());
        assertEquals(12L, result.metrics().wallTimeMs());
        assertNull(result.metrics().cpuTimeMs());
        assertEquals(9001L, result.metrics().fuelUsed());
        assertNull(result.metrics().peakMemoryBytes());
        assertEquals(Lane.WASM, result.lane());
        assertEquals("linux", result.effectiveIsolation().os());
    }
}
