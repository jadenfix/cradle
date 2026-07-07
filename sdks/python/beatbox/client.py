"""HTTP client for the beatbox sandbox REST API.

Zero dependencies: built on :mod:`urllib.request` and :mod:`json` from the
standard library. Works on Python 3.9+.
"""

from __future__ import annotations

import json
import ipaddress
import socket
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Dict, Optional, Tuple

from .errors import BeatboxApiError, BeatboxTransportError
from .models import (
    CreateJobResponse,
    ExecuteRequest,
    ExecutionResult,
    JobRecord,
)

__all__ = ["Client"]

DEFAULT_TIMEOUT = 65.0
_API_KEY_HEADER = "x-beatbox-api-key"


class _NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Refuse to follow redirects so the API key cannot leak cross-origin."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D401
        return None


def _encode_job_id(job_id: str) -> str:
    """Percent-encode a job id as a single path segment.

    Rejects ``""``, ``"."`` and ``".."`` because they could retarget the
    request onto a different resource.
    """
    if job_id in ("", ".", ".."):
        raise ValueError(f"invalid job id: {job_id!r}")
    return urllib.parse.quote(job_id, safe="")


def _validate_base_url(base_url: str) -> str:
    if not isinstance(base_url, str):
        raise ValueError("base_url must be a string")
    trimmed = base_url.rstrip("/")
    parsed = urllib.parse.urlsplit(trimmed)
    if not parsed.scheme or not parsed.netloc:
        raise ValueError("base_url must be an absolute HTTP(S) origin")
    if "@" in parsed.netloc or parsed.username is not None or parsed.password is not None:
        raise ValueError("base_url must not contain credentials")
    if parsed.query:
        raise ValueError("base_url must not contain a query")
    if parsed.fragment:
        raise ValueError("base_url must not contain a fragment")
    _validate_base_path(parsed.path)
    try:
        parsed.port
    except ValueError as exc:
        raise ValueError(f"base_url has an invalid port: {exc}") from None
    if parsed.scheme == "https":
        return urllib.parse.urlunsplit(
            (parsed.scheme, parsed.netloc.lower(), parsed.path, "", "")
        )
    if parsed.scheme != "http":
        raise ValueError("base_url must use http or https")
    hostname = parsed.hostname
    if hostname is None:
        raise ValueError("base_url must include a host")
    if not _is_loopback_ip_literal(hostname):
        raise ValueError(
            "http base_url is allowed only for loopback IP literal addresses"
        )
    return urllib.parse.urlunsplit(
        (parsed.scheme, parsed.netloc.lower(), parsed.path, "", "")
    )


def _is_loopback_ip_literal(hostname: str) -> bool:
    try:
        return ipaddress.ip_address(hostname).is_loopback
    except ValueError:
        return False


def _validate_base_path(path: str) -> None:
    if "\\" in path:
        raise ValueError("base_url path must not contain backslashes")
    for segment in path.split("/"):
        decoded = urllib.parse.unquote(segment)
        if decoded in (".", ".."):
            raise ValueError("base_url path must not contain dot segments")
        if "/" in decoded or "\\" in decoded:
            raise ValueError("base_url path must not contain encoded slashes")


class Client:
    """A client for a single beatbox daemon.

    Args:
        base_url: Daemon base URL, e.g. ``http://127.0.0.1:7300``. Trailing
            slashes are trimmed. HTTPS URLs are accepted; plain HTTP is
            accepted only for loopback IP literal addresses. Userinfo, query
            strings, fragments, relative URLs, non-HTTP schemes, and path
            prefixes with dot segments or encoded slashes are rejected before
            any API-key-bearing request can be built.
        api_key: Optional API key. When set it is sent as the
            ``x-beatbox-api-key`` header on every request except ``health`` and
            ``openapi``.
        timeout: Per-request timeout in seconds (default 65.0).
    """

    def __init__(
        self,
        base_url: str,
        api_key: Optional[str] = None,
        timeout: float = DEFAULT_TIMEOUT,
    ) -> None:
        self.base_url = _validate_base_url(base_url)
        self.api_key = api_key
        self.timeout = timeout
        self._opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            _NoRedirectHandler,
        )

    # -- public API ---------------------------------------------------------

    def health(self) -> Dict[str, Any]:
        """GET /v1/health (unauthenticated). Returns raw JSON."""
        return self._request("GET", "/v1/health", auth=False)

    def capabilities(self) -> Dict[str, Any]:
        """GET /v1/capabilities. Returns raw JSON."""
        return self._request("GET", "/v1/capabilities", auth=True)

    def browser_profiles(self) -> Dict[str, Any]:
        """GET /v1/browser/profiles. Returns raw JSON."""
        return self._request("GET", "/v1/browser/profiles", auth=True)

    def browser_admit(self, request: Dict[str, Any]) -> Dict[str, Any]:
        """POST /v1/browser/admit. Returns raw admission decision JSON."""
        return self._request("POST", "/v1/browser/admit", auth=True, body=request)

    def browser_adapter_contract(self) -> Dict[str, Any]:
        """GET /v1/browser/adapter/contract. Returns raw contract JSON."""
        return self._request("GET", "/v1/browser/adapter/contract", auth=True)

    def browser_adapter_capability(self, request: Dict[str, Any]) -> Dict[str, Any]:
        """POST /v1/browser/adapter/capability. Returns raw capability JSON."""
        return self._request(
            "POST", "/v1/browser/adapter/capability", auth=True, body=request
        )

    def browser_adapter_register(self, request: Dict[str, Any]) -> Dict[str, Any]:
        """POST /v1/browser/adapter/register. Returns raw registration JSON."""
        return self._request(
            "POST", "/v1/browser/adapter/register", auth=True, body=request
        )

    def browser_adapter_launch_plan(self, request: Dict[str, Any]) -> Dict[str, Any]:
        """POST /v1/browser/adapter/launch/plan. Returns raw launch plan JSON."""
        return self._request(
            "POST", "/v1/browser/adapter/launch/plan", auth=True, body=request
        )

    def browser_adapter_launch_claim(self, request: Dict[str, Any]) -> Dict[str, Any]:
        """POST /v1/browser/adapter/launch/claim. Returns raw launch claim JSON."""
        return self._request(
            "POST", "/v1/browser/adapter/launch/claim", auth=True, body=request
        )

    def browser_adapter_validate(self, request: Dict[str, Any]) -> Dict[str, Any]:
        """POST /v1/browser/adapter/validate. Returns raw validation JSON."""
        return self._request(
            "POST", "/v1/browser/adapter/validate", auth=True, body=request
        )

    def browser_adapter_completion_validate(
        self, request: Dict[str, Any]
    ) -> Dict[str, Any]:
        """POST /v1/browser/adapter/completion/validate. Returns raw validation JSON."""
        return self._request(
            "POST", "/v1/browser/adapter/completion/validate", auth=True, body=request
        )

    def execute(self, request: ExecuteRequest) -> ExecutionResult:
        """POST /v1/execute. Returns an :class:`ExecutionResult`."""
        body = self._request("POST", "/v1/execute", auth=True, body=request.to_dict())
        return ExecutionResult.from_dict(body)

    def create_job(self, request: ExecuteRequest) -> CreateJobResponse:
        """POST /v1/jobs. Returns a :class:`CreateJobResponse` (HTTP 202)."""
        body = self._request("POST", "/v1/jobs", auth=True, body=request.to_dict())
        return CreateJobResponse.from_dict(body)

    def get_job(self, job_id: str) -> JobRecord:
        """GET /v1/jobs/{id}. Returns a :class:`JobRecord`."""
        path = "/v1/jobs/" + _encode_job_id(job_id)
        body = self._request("GET", path, auth=True)
        return JobRecord.from_dict(body)

    def cancel_job(self, job_id: str) -> None:
        """DELETE /v1/jobs/{id}. Returns nothing (HTTP 204)."""
        path = "/v1/jobs/" + _encode_job_id(job_id)
        self._request("DELETE", path, auth=True)

    def openapi(self) -> Dict[str, Any]:
        """GET /openapi.json (unauthenticated). Returns raw JSON."""
        return self._request("GET", "/openapi.json", auth=False)

    # -- internals ----------------------------------------------------------

    def _request(
        self,
        method: str,
        path: str,
        *,
        auth: bool,
        body: Optional[Dict[str, Any]] = None,
    ) -> Any:
        url = self.base_url + path
        data: Optional[bytes] = None
        headers: Dict[str, str] = {}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["content-type"] = "application/json"
        if auth and self.api_key:
            headers[_API_KEY_HEADER] = self.api_key

        req = urllib.request.Request(url, data=data, method=method, headers=headers)

        try:
            with self._opener.open(req, timeout=self.timeout) as resp:
                status = resp.status
                raw = resp.read()
        except urllib.error.HTTPError as exc:
            raise self._api_error(exc) from None
        except (urllib.error.URLError, socket.timeout, OSError) as exc:
            raise BeatboxTransportError(self._transport_reason(exc)) from None

        return self._parse_body(status, raw)

    @staticmethod
    def _parse_body(status: int, raw: bytes) -> Any:
        if status == 204 or not raw:
            return None
        try:
            return json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as exc:
            raise BeatboxTransportError(f"invalid JSON in response: {exc}") from None

    @staticmethod
    def _api_error(exc: urllib.error.HTTPError) -> BeatboxApiError:
        status = exc.code
        code: Optional[str] = None
        message = exc.reason if isinstance(exc.reason, str) else str(exc.reason)
        try:
            raw = exc.read()
        except Exception:
            raw = b""
        if raw:
            try:
                parsed = json.loads(raw.decode("utf-8"))
                err = parsed.get("error") if isinstance(parsed, dict) else None
                if isinstance(err, dict):
                    code = err.get("code")
                    message = err.get("message", message)
            except (ValueError, UnicodeDecodeError, AttributeError):
                pass
        return BeatboxApiError(status, code, message)

    @staticmethod
    def _transport_reason(exc: Exception) -> str:
        if isinstance(exc, urllib.error.URLError):
            return str(exc.reason)
        return str(exc)
