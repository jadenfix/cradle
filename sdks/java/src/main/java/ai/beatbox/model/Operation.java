package ai.beatbox.model;

import com.fasterxml.jackson.databind.JsonNode;

/** Shared long-running operation envelope. */
public record Operation(
        String name,
        boolean done,
        OperationMetadata metadata,
        JsonNode response,
        ErrorBody error) {
}
