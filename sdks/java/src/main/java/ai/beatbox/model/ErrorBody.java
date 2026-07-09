package ai.beatbox.model;

import com.fasterxml.jackson.databind.JsonNode;

/** The shared error payload returned by the API. */
public record ErrorBody(
        String code,
        String message,
        Integer status,
        String requestId,
        Boolean retryable,
        JsonNode details) {
}
