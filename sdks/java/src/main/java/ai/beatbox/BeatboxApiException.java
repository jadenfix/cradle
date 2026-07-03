package ai.beatbox;

/**
 * Raised on a non-2xx HTTP response. Carries the HTTP {@code status}, the API error {@code code}
 * from the {@code {error:{code,message}}} body (may be {@code null} if absent), and a message.
 *
 * <p>The api key is never included in the message or anywhere on this exception.
 */
public final class BeatboxApiException extends BeatboxException {

    private final int status;
    private final String code;

    public BeatboxApiException(int status, String code, String message) {
        super(message);
        this.status = status;
        this.code = code;
    }

    /** HTTP status code of the failing response. */
    public int status() {
        return status;
    }

    /** Machine-readable error code from the response body, or {@code null} if none was present. */
    public String code() {
        return code;
    }

    @Override
    public String toString() {
        return "BeatboxApiException{status=" + status + ", code=" + code + ", message=" + getMessage() + "}";
    }
}
