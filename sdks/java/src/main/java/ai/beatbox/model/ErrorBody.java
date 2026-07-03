package ai.beatbox.model;

/** The {@code {code, message}} error payload returned by the API. */
public record ErrorBody(String code, String message) {
}
