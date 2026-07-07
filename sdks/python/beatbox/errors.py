"""Typed errors raised by the beatbox SDK.

Neither error ever embeds auth material, so it is safe to log or surface them.
"""

from __future__ import annotations

from typing import Optional


class BeatboxError(Exception):
    """Base class for every error raised by this SDK."""


class BeatboxApiError(BeatboxError):
    """Raised when the daemon returns a non-2xx HTTP response.

    Attributes:
        status: HTTP status code of the response.
        code: Machine-readable code from the ``{"error": {"code", "message"}}``
            body, or ``None`` if the body had no code.
        message: Human-readable message from the error body (falls back to the
            HTTP reason phrase).
    """

    def __init__(self, status: int, code: Optional[str], message: str) -> None:
        self.status = status
        self.code = code
        self.message = message
        detail = f"[{code}] {message}" if code else message
        super().__init__(f"beatbox API error (HTTP {status}): {detail}")


class BeatboxTransportError(BeatboxError):
    """Raised when the request never produced an HTTP response.

    Covers connection failures, DNS errors, timeouts, and malformed responses.
    """

    def __init__(self, message: str) -> None:
        self.message = message
        super().__init__(f"beatbox transport error: {message}")
