/**
 * beatbox — zero-dependency TypeScript SDK for the beatbox sandbox REST API.
 *
 * @example
 * ```ts
 * import { BeatboxClient, ExecuteRequest } from "beatbox";
 *
 * const client = new BeatboxClient({
 *   baseUrl: "http://127.0.0.1:7300",
 *   apiKey: process.env.BEATBOX_API_KEY,
 * });
 *
 * const result = await client.execute(
 *   ExecuteRequest.wasmWat(
 *     '(module (func (export "run") (param i64) (result i64) local.get 0 i64.const 1 i64.add))',
 *     { input: { n: 41 } },
 *   ),
 * );
 * console.log(result.value); // 42
 * ```
 */

export {
  BeatboxClient,
  encodeJobId,
  API_KEY_HEADER,
  DEFAULT_TIMEOUT_MS,
} from "./client.js";
export type { BeatboxClientConfig } from "./client.js";

export {
  BeatboxError,
  BeatboxApiError,
  BeatboxTransportError,
} from "./errors.js";

export {
  Lane,
  ExecutionStatus,
  JobStatus,
  Source,
  ExecuteRequest,
} from "./models.js";

export type {
  MountMode,
  SecretExpose,
  InlineSource,
  WasmFileSource,
  WasmWatSource,
  WasmBytesBase64Source,
  ModuleRefSource,
  Limits,
  Mount,
  FsPolicy,
  NetPolicy,
  Determinism,
  Secret,
  Policy,
  ExecuteRequestOptions,
  Metrics,
  EffectiveIsolation,
  EgressRecord,
  ErrorBody,
  ErrorResponse,
  ExecutionResult,
  JobRecord,
  CreateJobResponse,
  HealthResponse,
} from "./models.js";
