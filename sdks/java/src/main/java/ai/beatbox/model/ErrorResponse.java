package ai.beatbox.model;

/** Envelope for error responses: {@code {"error": {code, message}}}. */
public record ErrorResponse(ErrorBody error) {
}
