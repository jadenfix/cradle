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
        target_origins: ["https://example.com"],
        credential_mode: "no_credentials",
        artifact_mode: "discard",
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
        intent_warnings: [],
        guard_plan: {
          network: {
            allowed_origins: ["https://example.com"],
            deny_private_networks: true,
            deny_localhost: true,
            deny_metadata_endpoints: true,
            require_dns_rebinding_protection: true,
            require_redirect_revalidation: true,
            require_proxy_enforcement: true,
            outbound_network_disabled_without_proxy: true,
          },
          credentials: {
            mode: "no_credentials",
            ambient_credentials_allowed: false,
            user_mediation_required: false,
            scoped_secret_channel_required: false,
          },
          storage: {
            mode: "discard",
            plaintext_persistence_allowed: false,
            explicit_artifact_allowlist_required: false,
            encryption_required_for_persistence: false,
            teardown_proof_required: true,
          },
          required_runtime_guards: [
            "browser launcher bound to the selected sandbox profile",
            "production-path admission check before launch",
            "teardown proof before reporting session completion",
            "fresh profile directory with no host browser state",
            "deny-by-default egress proxy that revalidates DNS, redirects, and final socket targets",
            "loopback, LAN, shared, link-local, and metadata address block",
            "OS jail or microVM boundary around the browser process",
          ],
        },
        adapter_handoff: {
          contract_version: "browser-adapter-v1",
          launch_endpoint: null,
          launchable: false,
          handoff_fields: [
            "requested_level",
            "actor",
            "sensitivity",
            "target_origins",
            "credential_mode",
            "artifact_mode",
            "requested_controls",
            "guard_plan",
          ],
          required_completion_proofs: [
            "browser process exited or was killed",
            "temporary profile directory removed",
            "plaintext artifacts outside the explicit allowlist removed",
            "egress proxy log sealed or discarded according to artifact_mode",
          ],
          unavailable_reason: "no browser adapter launch endpoint is implemented by this daemon",
        },
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
      target_origins: ["https://example.com"],
      credential_mode: "no_credentials",
      artifact_mode: "discard",
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
      target_origins: ["https://example.com"],
      credential_mode: "no_credentials",
      artifact_mode: "discard",
      required_controls: ["egress_policy", "remote_worker_isolation"],
    });
    assert.equal(response.decision, "rejected");
    assert.deepEqual(response.missing_controls, ["remote_worker_isolation"]);
    assert.deepEqual(response.target_origins, ["https://example.com"]);
    const guardPlan = response.guard_plan as {
      network: { require_proxy_enforcement: boolean };
      required_runtime_guards: string[];
    };
    assert.equal(guardPlan.network.require_proxy_enforcement, true);
    assert.equal(
      guardPlan.required_runtime_guards.some((guard) => guard.includes("final socket targets")),
      true,
    );
    assert.equal(
      guardPlan.required_runtime_guards.some((guard) => guard.includes("OS jail")),
      true,
    );
    const adapterHandoff = response.adapter_handoff as {
      launch_endpoint: null;
      launchable: boolean;
      handoff_fields: string[];
      required_completion_proofs: string[];
    };
    assert.equal(adapterHandoff.launchable, false);
    assert.equal(adapterHandoff.launch_endpoint, null);
    assert.equal(adapterHandoff.handoff_fields.includes("guard_plan"), true);
    assert.equal(
      adapterHandoff.required_completion_proofs.some((proof) =>
        proof.includes("temporary profile directory"),
      ),
      true,
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("validateBrowserAdapter sends authenticated JSON manifest", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        decision: "rejected",
        manifest_complete: false,
        launchable: false,
        trusted_for_sensitive_work: false,
        adapter_id: "tempo-os-jail-v1",
        launch_endpoint: "https://adapter.example/launch",
        endpoint_network_policy_bound: false,
        missing_levels: [],
        missing_controls: [],
        missing_guard_fields: [],
        missing_completion_proofs: [],
        reasons: ["no trusted adapter registration, endpoint binding, or launch path is implemented"],
        required_next_steps: ["implement authenticated adapter registration"],
        adapter_contract: {
          version: "browser-adapter-v1",
          status: "planned",
          launch_endpoint: null,
          handoff_fields: ["guard_plan"],
          required_guard_fields: ["guard_plan.network.deny_metadata_endpoints"],
          required_completion_proofs: ["temporary profile directory removed"],
          unavailable_reason: "no browser adapter launch endpoint is implemented by this daemon",
        },
        conformance_profile: {
          profile_version: "browser-adapter-conformance-v1",
          field_complete_manifest: {
            adapter_id: "tempo-conformance-adapter-v1",
            contract_version: "browser-adapter-v1",
            launch_endpoint: "https://adapter.example/launch",
            supported_levels: ["os_isolated"],
            supported_controls: ["os_process_isolation"],
            guard_fields: ["guard_plan.network.deny_metadata_endpoints"],
            completion_proofs: ["temporary profile directory removed"],
          },
          field_complete_expectation: {
            decision: "rejected",
            manifest_complete: false,
            launchable: false,
            trusted_for_sensitive_work: false,
            endpoint_network_policy_bound: false,
            missing_levels: [],
            missing_controls: [],
            missing_guard_fields: [],
            missing_completion_proofs: [],
          },
          required_cases: [{
            name: "dns_rebinding_hostname_stays_incomplete",
            expected_rest_status: 200,
            expected_rest_error_code: null,
            expected_mcp_error_code: null,
            expected_mcp_error_message_contains: [],
            expected_validation: {
              decision: "rejected",
              manifest_complete: false,
              launchable: false,
              trusted_for_sensitive_work: false,
              endpoint_network_policy_bound: false,
              missing_levels: [],
              missing_controls: [],
              missing_guard_fields: [],
              missing_completion_proofs: [],
            },
          }],
          notes: ["not a launch grant"],
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const request = {
      adapter_id: "tempo-os-jail-v1",
      contract_version: "browser-adapter-v1",
      launch_endpoint: "https://adapter.example/launch",
    };
    const response = await client.validateBrowserAdapter(request) as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/adapter/validate");
    assert.equal(capturedInit?.method, "POST");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
      "content-type": "application/json",
    });
    assert.deepEqual(JSON.parse(String(capturedInit?.body)), request);
    assert.equal(response.decision, "rejected");
    assert.equal(response.manifest_complete, false);
    assert.equal(response.launchable, false);
    assert.equal(
      (response.conformance_profile as Record<string, unknown>).profile_version,
      "browser-adapter-conformance-v1",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("validateBrowserAdapterCompletion sends authenticated JSON report", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        decision: "rejected",
        report_shape_complete: true,
        verified_on_production_path: false,
        trusted_for_sensitive_work: false,
        request_id: "browser-adapter-conformance-launch-v1",
        adapter_id: "tempo-conformance-adapter-v1",
        contract_version: "browser-adapter-v1",
        missing_proof_ids: [],
        unexpected_proof_ids: [],
        failed_evidence_fields: [],
        required_completion_proofs: ["temporary profile directory removed"],
        completion_proof_contract: [],
        reasons: ["shape only"],
        required_next_steps: ["verify production teardown"],
        adapter_contract: {
          version: "browser-adapter-v1",
          status: "planned",
          launch_endpoint: null,
          handoff_fields: ["completion_report_template"],
          required_guard_fields: [],
          required_completion_proofs: ["temporary profile directory removed"],
          completion_proof_contract: [],
          unavailable_reason: "no browser adapter launch endpoint is implemented by this daemon",
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const request = {
      request_id: "browser-adapter-conformance-launch-v1",
      adapter_id: "tempo-conformance-adapter-v1",
      contract_version: "browser-adapter-v1",
      process_terminated: true,
      temporary_profile_removed: true,
      plaintext_artifacts_removed: true,
      egress_log_sealed_or_discarded: true,
      sealed_artifact_handles: [],
      proof_ids: ["temporary_profile_removed"],
      notes: ["shape fixture only"],
    };
    const response = await client.validateBrowserAdapterCompletion(request) as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/adapter/completion/validate");
    assert.equal(capturedInit?.method, "POST");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
      "content-type": "application/json",
    });
    assert.deepEqual(JSON.parse(String(capturedInit?.body)), request);
    assert.equal(response.decision, "rejected");
    assert.equal(response.report_shape_complete, true);
    assert.equal(response.verified_on_production_path, false);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("browserAdapterContract sends authenticated GET", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        adapter_contract: {
          version: "browser-adapter-v1",
          status: "planned",
          launch_endpoint: null,
          handoff_fields: ["guard_plan"],
          required_guard_fields: ["guard_plan.network.deny_metadata_endpoints"],
          required_completion_proofs: ["temporary profile directory removed"],
          unavailable_reason: "no browser adapter launch endpoint is implemented by this daemon",
        },
        conformance_profile: {
          profile_version: "browser-adapter-conformance-v1",
          field_complete_manifest: {
            adapter_id: "tempo-conformance-adapter-v1",
            contract_version: "browser-adapter-v1",
            launch_endpoint: "https://adapter.example/launch",
            supported_levels: ["os_isolated"],
            supported_controls: ["os_process_isolation"],
            guard_fields: ["guard_plan.network.deny_metadata_endpoints"],
            completion_proofs: ["temporary profile directory removed"],
          },
          field_complete_expectation: {
            decision: "rejected",
            manifest_complete: false,
            launchable: false,
            trusted_for_sensitive_work: false,
            endpoint_network_policy_bound: false,
            missing_levels: [],
            missing_controls: [],
            missing_guard_fields: [],
            missing_completion_proofs: [],
          },
          required_cases: [],
          notes: ["not a launch grant"],
        },
        required_levels: ["os_isolated"],
        required_controls: ["os_process_isolation"],
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        notes: ["not adapter registration"],
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const response = await client.browserAdapterContract() as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/adapter/contract");
    assert.equal(capturedInit?.method, "GET");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
    });
    assert.equal(capturedInit?.body, undefined);
    assert.equal(response.launchable, false);
    assert.equal(response.trusted_for_sensitive_work, false);
    assert.equal(response.endpoint_network_policy_bound, false);
    assert.equal(
      (response.conformance_profile as Record<string, unknown>).profile_version,
      "browser-adapter-conformance-v1",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("issueBrowserAdapterCapability sends authenticated JSON", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        same_user_capability: "bbx-browser-adapter-cap-v1.fixture.not-a-secret",
        expires_at: "2026-07-06T20:00:00Z",
        ttl_seconds: 60,
        actor: "agent",
        sensitivity: "sensitive",
        adapter_id: "tempo-os-jail-v1",
        registration_endpoint: "/v1/browser/adapter/register",
        notes: ["keep it out of logs"],
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const request = {
      actor: "agent",
      sensitivity: "sensitive",
      adapter_id: "tempo-os-jail-v1",
      ttl_seconds: 60,
    };
    const response = await client.issueBrowserAdapterCapability(request) as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/adapter/capability");
    assert.equal(capturedInit?.method, "POST");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
      "content-type": "application/json",
    });
    assert.deepEqual(JSON.parse(String(capturedInit?.body)), request);
    assert.equal(
      response.same_user_capability,
      "bbx-browser-adapter-cap-v1.fixture.not-a-secret",
    );
    assert.equal(response.registration_endpoint, "/v1/browser/adapter/register");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("registerBrowserAdapter sends authenticated JSON preflight", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        decision: "rejected",
        adapter_id: "tempo-os-jail-v1",
        actor: "agent",
        sensitivity: "sensitive",
        registered: false,
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        same_user_capability_bound: false,
        manifest_validation: { launchable: false, endpoint_network_policy_bound: false },
        reasons: ["does not persist or trust adapters yet"],
        required_next_steps: ["issue a same-user capability"],
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const request = {
      actor: "agent",
      sensitivity: "sensitive",
      same_user_capability: "test-capability-fixture",
      manifest: { adapter_id: "tempo-os-jail-v1" },
    };
    const response = await client.registerBrowserAdapter(request) as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/adapter/register");
    assert.equal(capturedInit?.method, "POST");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
      "content-type": "application/json",
    });
    assert.deepEqual(JSON.parse(String(capturedInit?.body)), request);
    assert.equal(response.registered, false);
    assert.equal(response.launchable, false);
    assert.equal(response.same_user_capability_bound, false);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("planBrowserAdapterLaunch sends authenticated JSON preflight", async () => {
  const originalFetch = globalThis.fetch;
  let capturedUrl = "";
  let capturedInit: RequestInit | undefined;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    capturedUrl = String(input);
    capturedInit = init;
    return new Response(
      JSON.stringify({
        decision: "rejected",
        request_id: "bbx-browser-launch-plan-v1.fixture",
        adapter_id: "tempo-os-jail-v1",
        actor: "agent",
        sensitivity: "sensitive",
        launchable: false,
        trusted_for_sensitive_work: false,
        endpoint_network_policy_bound: false,
        same_user_capability_bound: true,
        launch_request: { request_id: "bbx-browser-launch-plan-v1.fixture" },
        completion_validation_endpoint: "/v1/browser/adapter/completion/validate",
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  }) as typeof fetch;
  try {
    const client = new BeatboxClient({
      baseUrl: "http://127.0.0.1:7300/",
      apiKey: "secret-key",
    });
    const request = {
      same_user_capability: "test-capability-fixture",
      admission: { actor: "agent", sensitivity: "sensitive" },
      manifest: { adapter_id: "tempo-os-jail-v1" },
    };
    const response = await client.planBrowserAdapterLaunch(request) as Record<string, unknown>;

    assert.equal(capturedUrl, "http://127.0.0.1:7300/v1/browser/adapter/launch/plan");
    assert.equal(capturedInit?.method, "POST");
    assert.deepEqual(capturedInit?.headers, {
      "x-beatbox-api-key": "secret-key",
      "content-type": "application/json",
    });
    assert.deepEqual(JSON.parse(String(capturedInit?.body)), request);
    assert.equal(response.launchable, false);
    assert.equal(response.same_user_capability_bound, true);
    assert.equal(response.completion_validation_endpoint, "/v1/browser/adapter/completion/validate");
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
