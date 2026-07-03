package ai.beatbox;

/**
 * Raised on a transport-level failure (connection refused, timeout, TLS, malformed response body,
 * ...) before a well-formed API error could be observed. The api key is never leaked.
 */
public final class BeatboxTransportException extends BeatboxException {

    public BeatboxTransportException(String message, Throwable cause) {
        super(message, cause);
    }

    public BeatboxTransportException(String message) {
        super(message);
    }
}
