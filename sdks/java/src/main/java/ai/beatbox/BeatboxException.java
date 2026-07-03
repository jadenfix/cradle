package ai.beatbox;

/** Base type for all beatbox SDK errors. Unchecked so call sites stay clean. */
public abstract class BeatboxException extends RuntimeException {

    protected BeatboxException(String message) {
        super(message);
    }

    protected BeatboxException(String message, Throwable cause) {
        super(message, cause);
    }
}
