package ai.beatbox.model;

/** Envelope for error responses: {@code {"error": {...}}}. */
public record ErrorResponse(ErrorBody error) {
}
