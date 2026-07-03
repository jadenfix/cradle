/**
 * Typed errors for the beatbox SDK.
 *
 * Neither error type ever carries the API key: only the HTTP status, the
 * server-supplied `{ code, message }`, or the underlying transport cause is
 * exposed.
 */

/** Base class for every error thrown by the SDK. */
export abstract class BeatboxError extends Error {}

/**
 * Thrown when the daemon returns a non-2xx response. Carries the HTTP
 * `status`, the machine-readable `code` from the `{ error: { code, message } }`
 * body (or `"unknown"` when the body is absent/unparseable) and the human
 * `message`.
 */
export class BeatboxApiError extends BeatboxError {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "BeatboxApiError";
    this.status = status;
    this.code = code;
    // Restore prototype chain for instanceof across compilation targets.
    Object.setPrototypeOf(this, BeatboxApiError.prototype);
  }
}

/**
 * Thrown on a transport-level failure: DNS/connection error, timeout/abort,
 * an unexpected redirect, or a response body that could not be read.
 */
export class BeatboxTransportError extends BeatboxError {
  /** The underlying error, when one is available. */
  override readonly cause?: unknown;

  constructor(message: string, cause?: unknown) {
    super(message);
    this.name = "BeatboxTransportError";
    this.cause = cause;
    Object.setPrototypeOf(this, BeatboxTransportError.prototype);
  }
}
