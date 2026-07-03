import json
import unittest

from beatbox import (
    ExecuteRequest,
    ExecutionResult,
    Lane,
    Limits,
    Policy,
    Source,
)
from beatbox.models import ExecutionStatus


class TestExecuteRequestSerialization(unittest.TestCase):
    def test_wasm_wat_minimal_wire_shape(self):
        req = ExecuteRequest.wasm_wat("(module)", input={"n": 41})
        self.assertEqual(
            req.to_dict(),
            {
                "lane": "wasm",
                "source": {"kind": "wasm_wat", "text": "(module)"},
                "input": {"n": 41},
            },
        )

    def test_optional_fields_omitted_when_unset(self):
        req = ExecuteRequest(lane=Lane.WASM, source=Source.wasm_wat("(module)"))
        d = req.to_dict()
        self.assertNotIn("input", d)  # UNSET input is omitted
        self.assertNotIn("entrypoint", d)
        self.assertNotIn("stdin", d)
        self.assertNotIn("policy", d)
        self.assertNotIn("idempotency_key", d)

    def test_explicit_null_input_is_sent(self):
        req = ExecuteRequest.wasm_wat("(module)", input=None)
        self.assertIn("input", req.to_dict())
        self.assertIsNone(req.to_dict()["input"])

    def test_partial_policy_and_limits(self):
        req = ExecuteRequest.wasm_wat(
            "(module)",
            policy=Policy(limits=Limits(wall_ms=5000)),
        )
        self.assertEqual(req.to_dict()["policy"], {"limits": {"wall_ms": 5000}})

    def test_request_json_round_trip(self):
        req = ExecuteRequest.wasm_wat(
            "(module ...)",
            entrypoint="run",
            input={"n": 10},
            stdin="hi",
            policy=Policy(limits=Limits(wall_ms=5000, fuel=1_000_000)),
            idempotency_key="step-1",
        )
        wire = json.loads(json.dumps(req.to_dict()))
        parsed = ExecuteRequest.from_dict(wire)
        self.assertEqual(parsed.to_dict(), req.to_dict())
        self.assertEqual(parsed.lane, Lane.WASM)
        self.assertEqual(parsed.source.text, "(module ...)")
        self.assertEqual(parsed.policy.limits.fuel, 1_000_000)


class TestSourceVariants(unittest.TestCase):
    def test_all_variants(self):
        self.assertEqual(
            Source.inline("x").to_dict(), {"kind": "inline", "code": "x"}
        )
        self.assertEqual(
            Source.wasm_file("/p").to_dict(), {"kind": "wasm_file", "path": "/p"}
        )
        self.assertEqual(
            Source.wasm_wat("(module)").to_dict(),
            {"kind": "wasm_wat", "text": "(module)"},
        )
        self.assertEqual(
            Source.wasm_bytes_base64("AA==").to_dict(),
            {"kind": "wasm_bytes_base64", "bytes": "AA=="},
        )
        self.assertEqual(
            Source.module_ref("abc").to_dict(),
            {"kind": "module_ref", "sha256": "abc"},
        )


class TestExecutionResultParsing(unittest.TestCase):
    def _sample(self):
        return {
            "status": "ok",
            "value": 42,
            "stdout": "",
            "stdout_truncated": False,
            "stderr": "",
            "stderr_truncated": False,
            "metrics": {
                "wall_time_ms": 3,
                "cpu_time_ms": None,
                "fuel_used": 120,
                "peak_memory_bytes": None,
            },
            "lane": "wasm",
            "deterministic": True,
            "inputs_digest": "sha256:abc",
            "engine_version": "w0",
            "beatbox_version": "0.1.0",
            "effective_isolation": {
                "os": "linux",
                "mechanisms": ["seccomp"],
                "downgrades": [],
                "landlock_abi": 3,
            },
            "egress": [],
            "error": None,
            "exit_code": None,
        }

    def test_parses_expected_fields(self):
        r = ExecutionResult.from_dict(self._sample())
        self.assertEqual(r.status, ExecutionStatus.OK)
        self.assertEqual(r.value, 42)
        self.assertEqual(r.lane, Lane.WASM)
        self.assertIsNone(r.metrics.cpu_time_ms)
        self.assertEqual(r.metrics.fuel_used, 120)
        self.assertIsNone(r.metrics.peak_memory_bytes)
        self.assertEqual(r.effective_isolation.os, "linux")
        self.assertIsNone(r.error)

    def test_result_json_round_trip(self):
        sample = self._sample()
        r = ExecutionResult.from_dict(sample)
        self.assertEqual(r.to_dict(), sample)

    def test_ignores_unknown_fields(self):
        sample = self._sample()
        sample["some_future_field"] = {"nested": True}
        sample["metrics"]["future_metric"] = 9
        r = ExecutionResult.from_dict(sample)  # must not raise
        self.assertEqual(r.value, 42)

    def test_unknown_enum_value_kept_raw(self):
        sample = self._sample()
        sample["status"] = "brand_new_status"
        r = ExecutionResult.from_dict(sample)
        self.assertEqual(r.status, "brand_new_status")


if __name__ == "__main__":
    unittest.main()
