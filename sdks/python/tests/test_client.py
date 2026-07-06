import io
import json
import unittest
import urllib.error
from contextlib import contextmanager
from unittest import mock

from beatbox import BeatboxApiError, BeatboxTransportError, Client, ExecuteRequest


class _FakeResponse:
    def __init__(self, status, body):
        self.status = status
        self._body = body

    def read(self):
        return self._body

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


@contextmanager
def _patched_open(client, fake):
    with mock.patch.object(client._opener, "open", fake) as m:
        yield m


class TestClientRequest(unittest.TestCase):
    def test_base_url_trailing_slash_trimmed(self):
        c = Client("http://host:7300///")
        self.assertEqual(c.base_url, "http://host:7300")

    def test_execute_sends_auth_and_content_type(self):
        c = Client("http://host:7300", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "status": "ok", "value": 42, "stdout": "", "stdout_truncated": False,
                "stderr": "", "stderr_truncated": False,
                "metrics": {"wall_time_ms": 1}, "lane": "wasm", "deterministic": True,
                "inputs_digest": "d", "engine_version": "w0", "beatbox_version": "0.1.0",
                "effective_isolation": {"os": "linux", "mechanisms": [], "downgrades": []},
                "egress": [],
            }
            return _FakeResponse(200, json.dumps(body).encode())

        with _patched_open(c, fake_open):
            result = c.execute(ExecuteRequest.wasm_wat("(module)", input={"n": 41}))

        self.assertEqual(result.value, 42)
        req = captured["req"]
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertEqual(req.get_header("Content-type"), "application/json")

    def test_health_is_unauthenticated(self):
        c = Client("http://host:7300", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            return _FakeResponse(200, b'{"status": "ok"}')

        with _patched_open(c, fake_open):
            c.health()

        self.assertIsNone(captured["req"].get_header("X-beatbox-api-key"))

    def test_browser_admit_sends_auth_json_preflight(self):
        c = Client("http://host:7300/", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "decision": "rejected",
                "runnable_browser_sessions": False,
                "requested_level": "os_isolated",
                "selected_level": None,
                "actor": "agent",
                "sensitivity": "sensitive",
                "target_origins": ["https://example.com"],
                "credential_mode": "no_credentials",
                "artifact_mode": "discard",
                "requested_controls": ["egress_policy", "remote_worker_isolation"],
                "requested_profile_controls": [
                    "fresh_profile",
                    "no_ambient_credentials",
                    "egress_policy",
                    "local_network_block",
                    "os_process_isolation",
                    "teardown_proof",
                ],
                "missing_controls": ["remote_worker_isolation"],
                "level_satisfies_requested_controls": False,
                "intent_warnings": [],
                "downgrade_allowed": False,
                "reasons": ["no runnable browser sandbox"],
                "required_next_steps": ["implement a browser launcher"],
                "profiles_endpoint": "/v1/browser/profiles",
            }
            return _FakeResponse(200, json.dumps(body).encode())

        with _patched_open(c, fake_open):
            decision = c.browser_admit({
                "requested_level": "os_isolated",
                "actor": "agent",
                "sensitivity": "sensitive",
                "target_origins": ["https://example.com"],
                "credential_mode": "no_credentials",
                "artifact_mode": "discard",
                "required_controls": ["egress_policy", "remote_worker_isolation"],
            })

        req = captured["req"]
        self.assertEqual(req.full_url, "http://host:7300/v1/browser/admit")
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertEqual(req.get_header("Content-type"), "application/json")
        self.assertEqual(json.loads(req.data.decode()), {
            "requested_level": "os_isolated",
            "actor": "agent",
            "sensitivity": "sensitive",
            "target_origins": ["https://example.com"],
            "credential_mode": "no_credentials",
            "artifact_mode": "discard",
            "required_controls": ["egress_policy", "remote_worker_isolation"],
        })
        self.assertEqual(decision["decision"], "rejected")
        self.assertEqual(decision["missing_controls"], ["remote_worker_isolation"])
        self.assertEqual(decision["target_origins"], ["https://example.com"])

    def test_cancel_job_204_returns_none(self):
        c = Client("http://host:7300")

        def fake_open(req, timeout=None):
            return _FakeResponse(204, b"")

        with _patched_open(c, fake_open):
            self.assertIsNone(c.cancel_job("550e8400-e29b-41d4-a716-446655440000"))

    def test_api_error_maps_code_and_message(self):
        c = Client("http://host:7300", api_key="secret-key")
        body = json.dumps({"error": {"code": "bad_source", "message": "nope"}}).encode()

        def fake_open(req, timeout=None):
            raise urllib.error.HTTPError(
                "http://host:7300/v1/execute", 422, "Unprocessable", {}, io.BytesIO(body)
            )

        with _patched_open(c, fake_open):
            with self.assertRaises(BeatboxApiError) as ctx:
                c.execute(ExecuteRequest.wasm_wat("(module)"))

        err = ctx.exception
        self.assertEqual(err.status, 422)
        self.assertEqual(err.code, "bad_source")
        self.assertEqual(err.message, "nope")
        # The API key must never leak into the error text.
        self.assertNotIn("secret-key", str(err))

    def test_transport_error_on_urlerror(self):
        c = Client("http://host:7300")

        def fake_open(req, timeout=None):
            raise urllib.error.URLError("connection refused")

        with _patched_open(c, fake_open):
            with self.assertRaises(BeatboxTransportError):
                c.health()

    def test_no_redirect_handler_installed(self):
        c = Client("http://host:7300")
        from beatbox.client import _NoRedirectHandler

        handlers = c._opener.handlers
        self.assertTrue(any(isinstance(h, _NoRedirectHandler) for h in handlers))


if __name__ == "__main__":
    unittest.main()
