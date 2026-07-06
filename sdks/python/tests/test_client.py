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
                "guard_plan": {
                    "network": {
                        "allowed_origins": ["https://example.com"],
                        "deny_private_networks": True,
                        "deny_localhost": True,
                        "deny_metadata_endpoints": True,
                        "require_dns_rebinding_protection": True,
                        "require_redirect_revalidation": True,
                        "require_proxy_enforcement": True,
                        "outbound_network_disabled_without_proxy": True,
                    },
                    "credentials": {
                        "mode": "no_credentials",
                        "ambient_credentials_allowed": False,
                        "user_mediation_required": False,
                        "scoped_secret_channel_required": False,
                    },
                    "storage": {
                        "mode": "discard",
                        "plaintext_persistence_allowed": False,
                        "explicit_artifact_allowlist_required": False,
                        "encryption_required_for_persistence": False,
                        "teardown_proof_required": True,
                    },
                    "required_runtime_guards": [
                        "browser launcher bound to the selected sandbox profile",
                        "production-path admission check before launch",
                        "teardown proof before reporting session completion",
                        "fresh profile directory with no host browser state",
                        "deny-by-default egress proxy that revalidates DNS, redirects, and final socket targets",
                        "loopback, LAN, shared, link-local, and metadata address block",
                        "OS jail or microVM boundary around the browser process",
                    ],
                },
                "adapter_handoff": {
                    "contract_version": "browser-adapter-v1",
                    "launch_endpoint": None,
                    "launchable": False,
                    "handoff_fields": [
                        "requested_level",
                        "actor",
                        "sensitivity",
                        "target_origins",
                        "credential_mode",
                        "artifact_mode",
                        "requested_controls",
                        "guard_plan",
                    ],
                    "required_completion_proofs": [
                        "browser process exited or was killed",
                        "temporary profile directory removed",
                        "plaintext artifacts outside the explicit allowlist removed",
                        "egress proxy log sealed or discarded according to artifact_mode",
                    ],
                    "unavailable_reason": "no browser adapter launch endpoint is implemented by this daemon",
                },
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
        self.assertTrue(decision["guard_plan"]["network"]["require_proxy_enforcement"])
        self.assertTrue(any(
            "final socket targets" in guard
            for guard in decision["guard_plan"]["required_runtime_guards"]
        ))
        self.assertTrue(any(
            "OS jail" in guard
            for guard in decision["guard_plan"]["required_runtime_guards"]
        ))
        self.assertFalse(decision["adapter_handoff"]["launchable"])
        self.assertIsNone(decision["adapter_handoff"]["launch_endpoint"])
        self.assertIn("guard_plan", decision["adapter_handoff"]["handoff_fields"])
        self.assertTrue(any(
            "temporary profile directory" in proof
            for proof in decision["adapter_handoff"]["required_completion_proofs"]
        ))

    def test_browser_adapter_validate_sends_auth_json(self):
        c = Client("http://host:7300/", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "decision": "rejected",
                "manifest_complete": False,
                "launchable": False,
                "trusted_for_sensitive_work": False,
                "adapter_id": "tempo-os-jail-v1",
                "launch_endpoint": "https://adapter.example/launch",
                "endpoint_network_policy_bound": False,
                "missing_levels": [],
                "missing_controls": [],
                "missing_guard_fields": [],
                "missing_completion_proofs": [],
                "reasons": [
                    "no trusted adapter registration, endpoint binding, or launch path is implemented"
                ],
                "required_next_steps": ["implement authenticated adapter registration"],
                "adapter_contract": {
                    "version": "browser-adapter-v1",
                    "status": "planned",
                    "launch_endpoint": None,
                    "handoff_fields": ["guard_plan"],
                    "required_guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                    "required_completion_proofs": ["temporary profile directory removed"],
                    "unavailable_reason": "no browser adapter launch endpoint is implemented by this daemon",
                },
                "conformance_profile": {
                    "profile_version": "browser-adapter-conformance-v1",
                    "field_complete_manifest": {
                        "adapter_id": "tempo-conformance-adapter-v1",
                        "contract_version": "browser-adapter-v1",
                        "launch_endpoint": "https://adapter.example/launch",
                        "supported_levels": ["os_isolated"],
                        "supported_controls": ["os_process_isolation"],
                        "guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                        "completion_proofs": ["temporary profile directory removed"],
                    },
                    "field_complete_expectation": {
                        "decision": "rejected",
                        "manifest_complete": False,
                        "launchable": False,
                        "trusted_for_sensitive_work": False,
                        "endpoint_network_policy_bound": False,
                        "missing_levels": [],
                        "missing_controls": [],
                        "missing_guard_fields": [],
                        "missing_completion_proofs": [],
                    },
                    "required_cases": [
                        {
                            "name": "dns_rebinding_hostname_stays_incomplete",
                            "expected_rest_status": 200,
                            "expected_rest_error_code": None,
                            "expected_mcp_error_code": None,
                            "expected_mcp_error_message_contains": [],
                            "expected_validation": {
                                "decision": "rejected",
                                "manifest_complete": False,
                                "launchable": False,
                                "trusted_for_sensitive_work": False,
                                "endpoint_network_policy_bound": False,
                                "missing_levels": [],
                                "missing_controls": [],
                                "missing_guard_fields": [],
                                "missing_completion_proofs": [],
                            },
                        }
                    ],
                    "notes": ["not a launch grant"],
                },
            }
            return _FakeResponse(200, json.dumps(body).encode())

        request = {
            "adapter_id": "tempo-os-jail-v1",
            "contract_version": "browser-adapter-v1",
            "launch_endpoint": "https://adapter.example/launch",
        }
        with _patched_open(c, fake_open):
            validation = c.browser_adapter_validate(request)

        req = captured["req"]
        self.assertEqual(req.full_url, "http://host:7300/v1/browser/adapter/validate")
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertEqual(req.get_header("Content-type"), "application/json")
        self.assertEqual(json.loads(req.data.decode()), request)
        self.assertEqual(validation["decision"], "rejected")
        self.assertFalse(validation["manifest_complete"])
        self.assertFalse(validation["launchable"])
        self.assertEqual(
            validation["conformance_profile"]["profile_version"],
            "browser-adapter-conformance-v1",
        )

    def test_browser_adapter_completion_validate_sends_auth_json(self):
        c = Client("http://host:7300/", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "decision": "rejected",
                "report_shape_complete": True,
                "verified_on_production_path": False,
                "trusted_for_sensitive_work": False,
                "request_id": "browser-adapter-conformance-launch-v1",
                "adapter_id": "tempo-conformance-adapter-v1",
                "contract_version": "browser-adapter-v1",
                "missing_proof_ids": [],
                "unexpected_proof_ids": [],
                "failed_evidence_fields": [],
                "required_completion_proofs": ["temporary profile directory removed"],
                "completion_proof_contract": [],
                "reasons": ["shape only"],
                "required_next_steps": ["verify production teardown"],
                "adapter_contract": {
                    "version": "browser-adapter-v1",
                    "status": "planned",
                    "launch_endpoint": None,
                    "handoff_fields": ["completion_report_template"],
                    "required_guard_fields": [],
                    "required_completion_proofs": ["temporary profile directory removed"],
                    "completion_proof_contract": [],
                    "unavailable_reason": "no browser adapter launch endpoint is implemented by this daemon",
                },
            }
            return _FakeResponse(200, json.dumps(body).encode())

        request = {
            "request_id": "browser-adapter-conformance-launch-v1",
            "adapter_id": "tempo-conformance-adapter-v1",
            "contract_version": "browser-adapter-v1",
            "process_terminated": True,
            "temporary_profile_removed": True,
            "plaintext_artifacts_removed": True,
            "egress_log_sealed_or_discarded": True,
            "sealed_artifact_handles": [],
            "proof_ids": ["temporary_profile_removed"],
            "notes": ["shape fixture only"],
        }
        with _patched_open(c, fake_open):
            validation = c.browser_adapter_completion_validate(request)

        req = captured["req"]
        self.assertEqual(req.full_url, "http://host:7300/v1/browser/adapter/completion/validate")
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertEqual(req.get_header("Content-type"), "application/json")
        self.assertEqual(json.loads(req.data.decode()), request)
        self.assertEqual(validation["decision"], "rejected")
        self.assertTrue(validation["report_shape_complete"])
        self.assertFalse(validation["verified_on_production_path"])

    def test_browser_adapter_contract_sends_auth_get(self):
        c = Client("http://host:7300/", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "adapter_contract": {
                    "version": "browser-adapter-v1",
                    "status": "planned",
                    "launch_endpoint": None,
                    "handoff_fields": ["guard_plan"],
                    "required_guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                    "required_completion_proofs": ["temporary profile directory removed"],
                    "unavailable_reason": "no browser adapter launch endpoint is implemented by this daemon",
                },
                "conformance_profile": {
                    "profile_version": "browser-adapter-conformance-v1",
                    "field_complete_manifest": {
                        "adapter_id": "tempo-conformance-adapter-v1",
                        "contract_version": "browser-adapter-v1",
                        "launch_endpoint": "https://adapter.example/launch",
                        "supported_levels": ["os_isolated"],
                        "supported_controls": ["os_process_isolation"],
                        "guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                        "completion_proofs": ["temporary profile directory removed"],
                    },
                    "field_complete_expectation": {
                        "decision": "rejected",
                        "manifest_complete": False,
                        "launchable": False,
                        "trusted_for_sensitive_work": False,
                        "endpoint_network_policy_bound": False,
                        "missing_levels": [],
                        "missing_controls": [],
                        "missing_guard_fields": [],
                        "missing_completion_proofs": [],
                    },
                    "required_cases": [],
                    "notes": ["not a launch grant"],
                },
                "required_levels": ["os_isolated"],
                "required_controls": ["os_process_isolation"],
                "launchable": False,
                "trusted_for_sensitive_work": False,
                "endpoint_network_policy_bound": False,
                "notes": ["not adapter registration"],
            }
            return _FakeResponse(200, json.dumps(body).encode())

        with _patched_open(c, fake_open):
            contract = c.browser_adapter_contract()

        req = captured["req"]
        self.assertEqual(req.full_url, "http://host:7300/v1/browser/adapter/contract")
        self.assertEqual(req.get_method(), "GET")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertIsNone(req.data)
        self.assertFalse(contract["launchable"])
        self.assertFalse(contract["trusted_for_sensitive_work"])
        self.assertFalse(contract["endpoint_network_policy_bound"])
        self.assertEqual(
            contract["conformance_profile"]["profile_version"],
            "browser-adapter-conformance-v1",
        )

    def test_browser_adapter_capability_sends_auth_json(self):
        c = Client("http://host:7300/", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "same_user_capability": "bbx-browser-adapter-cap-v1.fixture.not-a-secret",
                "expires_at": "2026-07-06T20:00:00Z",
                "ttl_seconds": 60,
                "actor": "agent",
                "sensitivity": "sensitive",
                "adapter_id": "tempo-os-jail-v1",
                "registration_endpoint": "/v1/browser/adapter/register",
                "notes": ["keep it out of logs"],
            }
            return _FakeResponse(200, json.dumps(body).encode())

        request = {
            "actor": "agent",
            "sensitivity": "sensitive",
            "adapter_id": "tempo-os-jail-v1",
            "ttl_seconds": 60,
        }
        with _patched_open(c, fake_open):
            issued = c.browser_adapter_capability(request)

        req = captured["req"]
        self.assertEqual(req.full_url, "http://host:7300/v1/browser/adapter/capability")
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertEqual(req.get_header("Content-type"), "application/json")
        self.assertEqual(json.loads(req.data.decode()), request)
        self.assertEqual(
            issued["same_user_capability"],
            "bbx-browser-adapter-cap-v1.fixture.not-a-secret",
        )
        self.assertEqual(issued["registration_endpoint"], "/v1/browser/adapter/register")

    def test_browser_adapter_register_sends_auth_json(self):
        c = Client("http://host:7300/", api_key="secret-key")
        captured = {}

        def fake_open(req, timeout=None):
            captured["req"] = req
            body = {
                "decision": "rejected",
                "adapter_id": "tempo-os-jail-v1",
                "actor": "agent",
                "sensitivity": "sensitive",
                "registered": False,
                "launchable": False,
                "trusted_for_sensitive_work": False,
                "endpoint_network_policy_bound": False,
                "same_user_capability_bound": False,
                "manifest_validation": {
                    "decision": "rejected",
                    "manifest_complete": False,
                    "launchable": False,
                    "trusted_for_sensitive_work": False,
                    "adapter_id": "tempo-os-jail-v1",
                    "launch_endpoint": "https://adapter.example/launch",
                    "endpoint_network_policy_bound": False,
                    "missing_levels": [],
                    "missing_controls": [],
                    "missing_guard_fields": [],
                    "missing_completion_proofs": [],
                    "reasons": ["validation metadata only"],
                    "required_next_steps": ["implement registration"],
                    "adapter_contract": {
                        "version": "browser-adapter-v1",
                        "status": "planned",
                        "launch_endpoint": None,
                        "handoff_fields": ["guard_plan"],
                        "required_guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                        "required_completion_proofs": ["temporary profile directory removed"],
                        "unavailable_reason": "no browser adapter launch endpoint is implemented by this daemon",
                    },
                    "conformance_profile": {
                        "profile_version": "browser-adapter-conformance-v1",
                        "field_complete_manifest": {
                            "adapter_id": "tempo-conformance-adapter-v1",
                            "contract_version": "browser-adapter-v1",
                            "launch_endpoint": "https://adapter.example/launch",
                            "supported_levels": ["os_isolated"],
                            "supported_controls": ["os_process_isolation"],
                            "guard_fields": ["guard_plan.network.deny_metadata_endpoints"],
                            "completion_proofs": ["temporary profile directory removed"],
                        },
                        "field_complete_expectation": {
                            "decision": "rejected",
                            "manifest_complete": False,
                            "launchable": False,
                            "trusted_for_sensitive_work": False,
                            "endpoint_network_policy_bound": False,
                            "missing_levels": [],
                            "missing_controls": [],
                            "missing_guard_fields": [],
                            "missing_completion_proofs": [],
                        },
                        "required_cases": [],
                        "notes": ["not a launch grant"],
                    },
                },
                "reasons": ["does not persist or trust adapters yet"],
                "required_next_steps": ["issue a same-user capability"],
            }
            return _FakeResponse(200, json.dumps(body).encode())

        request = {
            "actor": "agent",
            "sensitivity": "sensitive",
            "same_user_capability": "test-capability-fixture",
            "manifest": {"adapter_id": "tempo-os-jail-v1"},
        }
        with _patched_open(c, fake_open):
            registration = c.browser_adapter_register(request)

        req = captured["req"]
        self.assertEqual(req.full_url, "http://host:7300/v1/browser/adapter/register")
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("X-beatbox-api-key"), "secret-key")
        self.assertEqual(req.get_header("Content-type"), "application/json")
        self.assertEqual(json.loads(req.data.decode()), request)
        self.assertFalse(registration["registered"])
        self.assertFalse(registration["launchable"])
        self.assertFalse(registration["same_user_capability_bound"])
        self.assertFalse(registration["manifest_validation"]["launchable"])

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
