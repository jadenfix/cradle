package ai.beatbox;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.net.URI;
import org.junit.jupiter.api.Test;

/** Verifies job ids become a single, safe, percent-encoded path segment. */
class JobUriTest {

    private final BeatboxClient client = BeatboxClient.builder()
            .baseUrl("http://127.0.0.1:7300/")
            .build();

    @Test
    void trimsTrailingSlashOnBaseUrl() {
        // The trailing slash on the configured baseUrl must not double up in the path.
        assertEquals("http://127.0.0.1:7300", client.baseUrl());
    }

    @Test
    void encodesNormalUuidUnchanged() {
        String id = "550e8400-e29b-41d4-a716-446655440000";
        URI uri = client.jobUri(id);
        assertEquals("/v1/jobs/" + id, uri.getRawPath());
    }

    @Test
    void encodesSlashSoPathCannotTraverse() {
        // "../execute" must not turn into the /v1/execute endpoint: the slash is encoded.
        URI uri = client.jobUri("../execute");
        assertEquals("/v1/jobs/..%2Fexecute", uri.getRawPath());
        // Single decoded segment, not two path components.
        assertEquals("/v1/jobs/../execute", uri.getPath());
    }

    @Test
    void encodesQueryDelimitersIntoTheSegment() {
        URI uri = client.jobUri("x?k=v");
        assertEquals("/v1/jobs/x%3Fk%3Dv", uri.getRawPath());
        assertNull(uri.getRawQuery());
    }

    @Test
    void rejectsEmptyId() {
        assertThrows(IllegalArgumentException.class, () -> client.jobUri(""));
    }

    @Test
    void rejectsDotId() {
        assertThrows(IllegalArgumentException.class, () -> client.jobUri("."));
    }

    @Test
    void rejectsDotDotId() {
        assertThrows(IllegalArgumentException.class, () -> client.jobUri(".."));
    }
}
