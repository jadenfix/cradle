/**
 * Typed models mirroring the beatbox `openapi.json` component schemas.
 *
 * Wire field names are snake_case (`wall_ms`, `cpu_time_ms`, ...); these types
 * use those exact names so `JSON.stringify`/`JSON.parse` round-trip to the wire
 * format with no translation layer.
 *
 * Deserialization is forward-compatible: every response model carries an index
 * signature so unknown/extra fields from a newer daemon are preserved rather
 * than crashing a strict consumer.
 */

// ---------------------------------------------------------------------------
// Enums (string literal unions + value maps for runtime use)
// ---------------------------------------------------------------------------

/** Execution engine / lane. */
export type Lane =
  | "wasm"
  | "python_wasi"
  | "python_native"
  | "js_wasm"
  | "js_native"
  | "exec";

export const Lane = {
  Wasm: "wasm",
  PythonWasi: "python_wasi",
  PythonNative: "python_native",
  JsWasm: "js_wasm",
  JsNative: "js_native",
  Exec: "exec",
} as const satisfies Record<string, Lane>;

/** Terminal status of a single execution. */
export type ExecutionStatus =
  | "ok"
  | "error"
  | "timeout"
  | "oom"
  | "killed"
  | "denied";

export const ExecutionStatus = {
  Ok: "ok",
  Error: "error",
  Timeout: "timeout",
  Oom: "oom",
  Killed: "killed",
  Denied: "denied",
} as const satisfies Record<string, ExecutionStatus>;

/** Lifecycle status of an asynchronous job. */
export type JobStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "canceled";

export const JobStatus = {
  Queued: "queued",
  Running: "running",
  Succeeded: "succeeded",
  Failed: "failed",
  Canceled: "canceled",
} as const satisfies Record<string, JobStatus>;

/** Mount access mode. */
export type MountMode = "ro" | "rw";

/** How a secret is exposed to the guest. */
export type SecretExpose = "env" | "file";

// ---------------------------------------------------------------------------
// Source (tagged union on `kind`)
// ---------------------------------------------------------------------------

export interface InlineSource {
  kind: "inline";
  code: string;
}
export interface WasmFileSource {
  kind: "wasm_file";
  path: string;
}
export interface WasmWatSource {
  kind: "wasm_wat";
  text: string;
}
export interface WasmBytesBase64Source {
  kind: "wasm_bytes_base64";
  bytes: string;
}
export interface ModuleRefSource {
  kind: "module_ref";
  sha256: string;
}

/** Program source, discriminated on `kind`. */
export type Source =
  | InlineSource
  | WasmFileSource
  | WasmWatSource
  | WasmBytesBase64Source
  | ModuleRefSource;

/** Per-variant constructors for {@link Source}. */
export const Source = {
  inline(code: string): InlineSource {
    return { kind: "inline", code };
  },
  wasmFile(path: string): WasmFileSource {
    return { kind: "wasm_file", path };
  },
  wasmWat(text: string): WasmWatSource {
    return { kind: "wasm_wat", text };
  },
  wasmBytesBase64(bytes: string): WasmBytesBase64Source {
    return { kind: "wasm_bytes_base64", bytes };
  },
  moduleRef(sha256: string): ModuleRefSource {
    return { kind: "module_ref", sha256 };
  },
} as const;

// ---------------------------------------------------------------------------
// Policy / Limits (all fields optional — partials merge onto server defaults)
// ---------------------------------------------------------------------------

/** Resource limits. Any subset is merged onto the daemon's defaults. */
export interface Limits {
  wall_ms?: number;
  cpu_ms?: number;
  memory_bytes?: number;
  disk_bytes?: number;
  output_bytes?: number;
  pids?: number;
  /** Wasm fuel budget; nullable to explicitly disable metering. */
  fuel?: number | null;
}

export interface Mount {
  host: string;
  guest: string;
  mode: MountMode;
}

export interface FsPolicy {
  workspace?: string | null;
  mounts?: Mount[];
}

export type NetPolicy =
  | { kind: "deny" }
  | { kind: "proxy"; allow_domains?: string[]; allow_ports?: number[] };

export type Determinism =
  | { kind: "off" }
  | { kind: "seeded"; seed: number; epoch_ms: number };

export interface Secret {
  name: string;
  value_ref: string;
  expose: SecretExpose;
}

/** Execution policy. All fields optional; partials merge onto defaults. */
export interface Policy {
  limits?: Limits;
  fs?: FsPolicy;
  net?: NetPolicy;
  determinism?: Determinism;
  env?: Record<string, string>;
  secrets?: Secret[];
  double_jail?: boolean;
}

// ---------------------------------------------------------------------------
// ExecuteRequest
// ---------------------------------------------------------------------------

/** A request to execute a program. `input` may be ANY JSON value. */
export interface ExecuteRequest {
  lane: Lane;
  source: Source;
  entrypoint?: string | null;
  input?: unknown;
  stdin?: string;
  policy?: Policy;
  idempotency_key?: string | null;
}

/** Optional fields for the ergonomic {@link ExecuteRequest} constructors. */
export interface ExecuteRequestOptions {
  entrypoint?: string | null;
  input?: unknown;
  stdin?: string;
  policy?: Policy;
  idempotencyKey?: string | null;
}

function buildRequest(
  lane: Lane,
  source: Source,
  opts: ExecuteRequestOptions = {},
): ExecuteRequest {
  const req: ExecuteRequest = { lane, source };
  if (opts.entrypoint !== undefined) req.entrypoint = opts.entrypoint;
  if (opts.input !== undefined) req.input = opts.input;
  if (opts.stdin !== undefined) req.stdin = opts.stdin;
  if (opts.policy !== undefined) req.policy = opts.policy;
  if (opts.idempotencyKey !== undefined)
    req.idempotency_key = opts.idempotencyKey;
  return req;
}

/**
 * Ergonomic constructors so the common case is one line, e.g.
 * `ExecuteRequest.wasmWat("(module ...)", { input: { n: 41 } })`.
 */
export const ExecuteRequest = {
  /** Build a request from an explicit {@link Source}. */
  of(lane: Lane, source: Source, opts?: ExecuteRequestOptions): ExecuteRequest {
    return buildRequest(lane, source, opts);
  },
  /** `wasm` lane from WAT text. */
  wasmWat(text: string, opts?: ExecuteRequestOptions): ExecuteRequest {
    return buildRequest("wasm", Source.wasmWat(text), opts);
  },
  /** `wasm` lane from base64-encoded module bytes. */
  wasmBytesBase64(bytes: string, opts?: ExecuteRequestOptions): ExecuteRequest {
    return buildRequest("wasm", Source.wasmBytesBase64(bytes), opts);
  },
  /** `wasm` lane from a content-addressed module reference. */
  moduleRef(sha256: string, opts?: ExecuteRequestOptions): ExecuteRequest {
    return buildRequest("wasm", Source.moduleRef(sha256), opts);
  },
  /** Inline source code on the given lane. */
  inline(
    lane: Lane,
    code: string,
    opts?: ExecuteRequestOptions,
  ): ExecuteRequest {
    return buildRequest(lane, Source.inline(code), opts);
  },
} as const;

// ---------------------------------------------------------------------------
// Response models
// ---------------------------------------------------------------------------

/**
 * Execution metrics. `cpu_time_ms`, `fuel_used` and `peak_memory_bytes` are
 * nullable: the W0 wasm lane does not measure CPU time separately from wall
 * time, so use `fuel_used` as the deterministic compute signal there.
 */
export interface Metrics {
  wall_time_ms: number;
  cpu_time_ms: number | null;
  fuel_used: number | null;
  peak_memory_bytes: number | null;
  [extra: string]: unknown;
}

export interface EffectiveIsolation {
  os: string;
  mechanisms: string[];
  downgrades: string[];
  landlock_abi?: number | null;
  [extra: string]: unknown;
}

export interface EgressRecord {
  domain: string;
  port: number;
  bytes: number;
  [extra: string]: unknown;
}

/** A single machine-readable error `{ code, message }`. */
export interface ErrorBody {
  code: string;
  message: string;
  status: number;
  request_id: string;
  retryable: boolean;
  details: Array<Record<string, unknown>>;
  [extra: string]: unknown;
}

/** Envelope wrapping an {@link ErrorBody} in error responses. */
export interface ErrorResponse {
  error: ErrorBody;
}

/** Result of a synchronous or completed execution. */
export interface ExecutionResult {
  status: ExecutionStatus;
  /** Program return value; ANY JSON value. */
  value: unknown;
  stdout: string;
  stdout_truncated: boolean;
  stderr: string;
  stderr_truncated: boolean;
  metrics: Metrics;
  lane: Lane;
  deterministic: boolean;
  inputs_digest: string;
  engine_version: string;
  beatbox_version: string;
  effective_isolation: EffectiveIsolation;
  egress: EgressRecord[];
  exit_code?: number | null;
  error?: ErrorBody | null;
  [extra: string]: unknown;
}

/** The record for an asynchronous job. */
export interface JobRecord {
  job_id: string;
  status: JobStatus;
  request: ExecuteRequest;
  created_at: string;
  updated_at: string;
  result?: ExecutionResult | null;
  error?: ErrorBody | null;
  [extra: string]: unknown;
}

/** Legacy pre-Operation create-job response shape. `createJob` now returns `Operation`. */
export interface CreateJobResponse {
  job_id: string;
  [extra: string]: unknown;
}

/** Metadata carried by a long-running operation. */
export interface OperationMetadata {
  target_resource?: string;
  create_time?: string;
  current_stage?: string;
  progress_ratio?: number;
  [extra: string]: unknown;
}

/** Shared long-running operation envelope. */
export interface Operation {
  name: string;
  done: boolean;
  metadata?: OperationMetadata | null;
  response?: unknown;
  error?: ErrorBody | null;
  [extra: string]: unknown;
}

/** `GET /v1/health` payload (loosely typed; extra fields tolerated). */
export interface HealthResponse {
  status: string;
  version: string;
  uptime_s: number;
  [extra: string]: unknown;
}
