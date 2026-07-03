package ai.beatbox;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;

import ai.beatbox.internal.Json;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.nio.charset.StandardCharsets;
import org.junit.jupiter.api.Test;

/**
 * A non-2xx response must always surface a typed {@link BeatboxApiException}
 * carrying the status, whatever shape the error body takes — never an uncaught
 * crash (BRIEF rule: "No panics/uncaught crashes on API errors").
 */
class ErrorBodyTest {
    private static final ObjectMapper MAPPER = Json.newMapper();

    private static BeatboxApiException errorFor(int status, String body) {
        return BeatboxClient.apiExceptionFor(status, body.getBytes(StandardCharsets.UTF_8), MAPPER);
    }

    @Test
    void wellFormedErrorEnvelopeIsParsed() {
        BeatboxApiException e = errorFor(429, "{\"error\":{\"code\":\"rate_limited\",\"message\":\"slow down\"}}");
        assertEquals(429, e.status());
        assertEquals("rate_limited", e.code());
        assertEquals("slow down", e.getMessage());
    }

    @Test
    void jsonNullBodyDoesNotCrash() {
        // Regression: mapper.readValue("null", ...) returns null; dereferencing
        // it used to throw an uncaught NullPointerException.
        BeatboxApiException e = errorFor(500, "null");
        assertEquals(500, e.status());
        assertNull(e.code());
        assertEquals("HTTP 500", e.getMessage());
    }

    @Test
    void emptyBodyFallsBackToStatus() {
        BeatboxApiException e = errorFor(503, "");
        assertEquals(503, e.status());
        assertNull(e.code());
        assertEquals("HTTP 503", e.getMessage());
    }

    @Test
    void nonJsonBodyFallsBackToStatus() {
        BeatboxApiException e = errorFor(502, "<html>Bad Gateway</html>");
        assertEquals(502, e.status());
        assertEquals("HTTP 502", e.getMessage());
    }

    @Test
    void unexpectedJsonShapesFallBackToStatus() {
        // A JSON scalar and an array both deserialize without an `error` object.
        assertEquals("HTTP 400", errorFor(400, "123").getMessage());
        assertEquals("HTTP 400", errorFor(400, "[1,2,3]").getMessage());
        assertEquals("HTTP 400", errorFor(400, "{\"error\":\"boom\"}").getMessage());
    }
}
