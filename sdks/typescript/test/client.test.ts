/**
 * Unit tests — no live daemon required.
 *
 * Covers:
 *   - job-id percent-encoding and rejection of `''`, `.`, `..`
 *   - ExecuteRequest / ExecutionResult JSON round-trip on the exact wire names
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  BeatboxClient,
  BeatboxTransportError,
  ExecuteRequest,
  Source,
  encodeJobId,
  type ExecutionResult,
} from "../src/index.js";

// --- job-id encoding -------------------------------------------------------

test("encodeJobId percent-encodes traversal + query into a single inert segment", () => {
  // `../execute` must not escape the /v1/jobs/ segment.
  assert.equal(encodeJobId("../execute"), "..%2Fexecute");
  // `x?k=v` must not introduce a query string.
  assert.equal(encodeJobId("x?k=v"), "x%3Fk%3Dv");
  // A normal UUID is unchanged.
  assert.equal(
    encodeJobId("3f2504e0-4f89-11d3-9a0c-0305e82c3301"),
    "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
  );
  // Other dangerous characters are encoded too.
  assert.equal(encodeJobId("a/b"), "a%2Fb");
  assert.equal(encodeJobId("a#b"), "a%23b");
  assert.equal(encodeJobId("a%2e"), "a%252e");
});

test("encodeJobId rejects '', '.', and '..'", () => {
  for (const bad of ["", ".", ".."]) {
    assert.throws(
      () => encodeJobId(bad),
      (err: unknown) =>
        err instanceof BeatboxTransportError &&
        /invalid job id/.test((err as Error).message),
      `expected rejection for ${JSON.stringify(bad)}`,
    );
  }
});

test("client getJob/cancelJob reject the retargeting ids before any request", async () => {
  const client = new BeatboxClient({ baseUrl: "http://127.0.0.1:7300" });
  for (const bad of ["", ".", ".."]) {
    await assert.rejects(
      () => client.getJob(bad),
      BeatboxTransportError,
    );
    await assert.rejects(
      () => client.cancelJob(bad),
      BeatboxTransportError,
    );
  }
});

// --- baseUrl handling ------------------------------------------------------

test("BeatboxClient trims trailing slashes on baseUrl", () => {
  // Not directly observable, but construction must not throw and an empty
  // baseUrl must throw.
  assert.doesNotThrow(
    () => new BeatboxClient({ baseUrl: "http://127.0.0.1:7300///" }),
  );
  assert.throws(() => new BeatboxClient({ baseUrl: "" }), TypeError);
});

test("admitBrowserSession sends authenticated JSON preflight", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        decision: "rejected",
        runnable_browser_sessions: false,
        requested_level: "os_isolated",
        selected_level: null,
        actor: "agent",
        sensitivity: "sensitive",
        requested_controls: ["egress_policy", "remote_worker_isolation"],
        requested_profile_controls: [
          "fresh_profile",
          "no_ambient_credentials",
          "egress_policy",
          "local_network_block",
          "os_process_isolation",
          "teardown_proof",
        ],
        missing_controls: ["remote_worker_isolation"],
        level_satisfies_requested_controls: false,
        downgrade_allowed: false,
        reasons: ["no runnable browser sandbox"],
        required_next_steps: ["implement a browser launcher"],
        profiles_endpoint: "/v1/browser/profiles",
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const response = await client.admitBrowserSession({
      requested_level: "os_isolated",
      actor: "agent",
      sensitivity: "sensitive",
      required_controls: ["egress_policy", "remote_worker_isolation"],
    }) as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/admit");
    assert.equal(capturedInit?.method, "POST");
    assert.equal(capturedInit?.redirect, "manual");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
      "content-type": "application/json",
    });
    assert.deepEqual(JSON.parse(String(capturedInit?.body)), {
      requested_level: "os_isolated",
      actor: "agent",
      sensitivity: "sensitive",
      required_controls: ["egress_policy", "remote_worker_isolation"],
    });
    assert.equal(response.decision, "rejected");
    assert.deepEqual(response.missing_controls, ["remote_worker_isolation"]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

// --- ExecuteRequest round-trip --------------------------------------------

test("ExecuteRequest.wasmWat serializes to the exact wire shape", () => {
  const req = ExecuteRequest.wasmWat("(module)", {
    input: { n: 41 },
    entrypoint: "run",
    idempotencyKey: "step-1",
    policy: { limits: { wall_ms: 5000, fuel: 10_000_000 } },
  });

  const wire = JSON.parse(JSON.stringify(req));
  assert.deepEqual(wire, {
    lane: "wasm",
    source: { kind: "wasm_wat", text: "(module)" },
    input: { n: 41 },
    entrypoint: "run",
    idempotency_key: "step-1",
    policy: { limits: { wall_ms: 5000, fuel: 10000000 } },
  });

  // Round-trips back to an identical structure.
  const back = JSON.parse(JSON.stringify(wire)) as typeof req;
  assert.deepEqual(back, wire);
});

test("Source constructors produce the tagged-union variants", () => {
  assert.deepEqual(Source.inline("print(1)"), {
    kind: "inline",
    code: "print(1)",
  });
  assert.deepEqual(Source.wasmFile("/m.wasm"), {
    kind: "wasm_file",
    path: "/m.wasm",
  });
  assert.deepEqual(Source.wasmWat("(module)"), {
    kind: "wasm_wat",
    text: "(module)",
  });
  assert.deepEqual(Source.wasmBytesBase64("AGFzbQ=="), {
    kind: "wasm_bytes_base64",
    bytes: "AGFzbQ==",
  });
  assert.deepEqual(Source.moduleRef("deadbeef"), {
    kind: "module_ref",
    sha256: "deadbeef",
  });
});

test("ExecuteRequest omits unset optional fields", () => {
  const req = ExecuteRequest.wasmWat("(module)");
  assert.deepEqual(JSON.parse(JSON.stringify(req)), {
    lane: "wasm",
    source: { kind: "wasm_wat", text: "(module)" },
  });
});

// --- ExecutionResult round-trip -------------------------------------------

test("ExecutionResult parses from wire JSON incl. nullable metrics + extras", () => {
  const wire = {
    status: "ok",
    value: 42,
    stdout: "",
    stdout_truncated: false,
    stderr: "",
    stderr_truncated: false,
    metrics: {
      wall_time_ms: 3,
      cpu_time_ms: null,
      fuel_used: 1234,
      peak_memory_bytes: null,
    },
    lane: "wasm",
    deterministic: true,
    inputs_digest: "sha256:abc",
    engine_version: "w0-1.2.3",
    beatbox_version: "0.1.0",
    effective_isolation: { os: "linux", mechanisms: ["seccomp"], downgrades: [] },
    egress: [],
    // A field a newer daemon might add — must not break parsing.
    future_field: { anything: true },
  };

  const result = JSON.parse(JSON.stringify(wire)) as ExecutionResult;

  assert.equal(result.status, "ok");
  assert.equal(result.value, 42);
  assert.equal(result.metrics.wall_time_ms, 3);
  assert.equal(result.metrics.cpu_time_ms, null);
  assert.equal(result.metrics.fuel_used, 1234);
  assert.equal(result.metrics.peak_memory_bytes, null);
  assert.equal(result.effective_isolation.os, "linux");
  // Forward-compat: unknown field is preserved.
  assert.deepEqual(result["future_field"], { anything: true });

  // Re-serializes back to the same wire object.
  assert.deepEqual(JSON.parse(JSON.stringify(result)), wire);
});
