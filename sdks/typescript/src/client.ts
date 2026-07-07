/**
 * Beatbox REST API client. Zero dependencies — built on the global `fetch`
 * and `AbortController` (Node 18+ / browsers).
 */

import { BeatboxApiError, BeatboxTransportError } from "./errors.js";
import type {
  CreateJobResponse,
  ExecuteRequest,
  ExecutionResult,
  HealthResponse,
  JobRecord,
} from "./models.js";

/** The HTTP header used to carry the API key. */
export const API_KEY_HEADER = "x-beatbox-api-key";

/** Default request timeout: 65 seconds. */
export const DEFAULT_TIMEOUT_MS = 65_000;

/** Configuration for {@link BeatboxClient}. */
export interface BeatboxClientConfig {
  /** Base URL of the daemon, e.g. `http://127.0.0.1:7300`. */
  baseUrl: string;
  /** Optional API key; sent as `x-beatbox-api-key` on authenticated routes. */
  apiKey?: string;
  /** Request timeout in milliseconds (default 65000). */
  timeoutMs?: number;
}

/**
 * Percent-encode a job id as a single path segment.
 *
 * The id is rejected if it is empty, `.` or `..`: those values can retarget
 * the request at a different resource once joined into the URL path. Every
 * other character is percent-encoded (including `/`, `?`, `#`, `%`) via
 * `encodeURIComponent`, which leaves ids like `../execute` or `x?k=v` inert.
 */
export function encodeJobId(jobId: string): string {
  if (jobId === "" || jobId === "." || jobId === "..") {
    throw new BeatboxTransportError(
      `invalid job id: ${JSON.stringify(jobId)}`,
    );
  }
  return encodeURIComponent(jobId);
}

function normalizeBaseUrl(input: string): string {
  if (input.trim() !== input) {
    throw new TypeError("BeatboxClient baseUrl must not contain leading or trailing whitespace");
  }
  let parsed: URL;
  try {
    parsed = new URL(input);
  } catch {
    throw new TypeError("BeatboxClient baseUrl must be an absolute URL");
  }

  if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
    throw new TypeError("BeatboxClient baseUrl must use http or https");
  }
  if (parsed.username !== "" || parsed.password !== "") {
    throw new TypeError("BeatboxClient baseUrl must not include credentials");
  }
  if (parsed.search !== "" || parsed.hash !== "") {
    throw new TypeError("BeatboxClient baseUrl must not include query or fragment");
  }
  if (input.includes("\\")) {
    throw new TypeError("BeatboxClient baseUrl path must not include backslashes");
  }
  if (parsed.protocol === "http:" && !isAllowedPlaintextLoopback(input)) {
    throw new TypeError("BeatboxClient baseUrl may use plaintext http only with loopback IP literals");
  }

  validateRawPathPrefix(input);
  return parsed.href.replace(/\/+$/, "");
}

function isAllowedPlaintextLoopback(input: string): boolean {
  const schemeEnd = input.indexOf("://");
  const authorityStart = schemeEnd + 3;
  const authorityEndCandidates = [
    input.indexOf("/", authorityStart),
    input.indexOf("?", authorityStart),
    input.indexOf("#", authorityStart),
  ].filter((idx) => idx >= 0);
  const authorityEnd = authorityEndCandidates.length === 0 ? input.length : Math.min(...authorityEndCandidates);
  const authority = input.slice(authorityStart, authorityEnd);
  const hostPort = authority.includes("@") ? authority.slice(authority.lastIndexOf("@") + 1) : authority;
  const host = hostPort.startsWith("[")
    ? hostPort.slice(0, hostPort.indexOf("]") + 1)
    : hostPort.split(":", 1)[0] ?? "";
  return host === "127.0.0.1" || host.toLowerCase() === "[::1]";
}

function validateRawPathPrefix(input: string): void {
  const schemeEnd = input.indexOf("://");
  if (schemeEnd < 0) {
    return;
  }
  const authorityStart = schemeEnd + 3;
  const pathStart = input.indexOf("/", authorityStart);
  if (pathStart < 0) {
    return;
  }
  const queryStart = input.indexOf("?", pathStart);
  const fragmentStart = input.indexOf("#", pathStart);
  const pathEndCandidates = [queryStart, fragmentStart].filter((idx) => idx >= 0);
  const pathEnd = pathEndCandidates.length === 0 ? input.length : Math.min(...pathEndCandidates);
  const rawPath = input.slice(pathStart, pathEnd);

  for (const segment of rawPath.split("/")) {
    if (segment === "" || segment === "." || segment === "..") {
      if (segment === "." || segment === "..") {
        throw new TypeError("BeatboxClient baseUrl path must not contain dot segments");
      }
      continue;
    }
    let decoded: string;
    try {
      decoded = decodeURIComponent(segment);
    } catch {
      throw new TypeError("BeatboxClient baseUrl path contains invalid percent encoding");
    }
    if (decoded === "." || decoded === "..") {
      throw new TypeError("BeatboxClient baseUrl path must not contain encoded dot segments");
    }
    if (decoded.includes("/") || decoded.includes("\\")) {
      throw new TypeError("BeatboxClient baseUrl path segments must not encode separators");
    }
  }
}

interface RequestOptions {
  method: string;
  path: string;
  auth: boolean;
  body?: unknown;
  /** Expected success status. `expectNoContent` returns void. */
  expectNoContent?: boolean;
}

export class BeatboxClient {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly timeoutMs: number;

  constructor(config: BeatboxClientConfig) {
    if (!config || typeof config.baseUrl !== "string" || config.baseUrl === "") {
      throw new TypeError("BeatboxClient requires a non-empty baseUrl");
    }
    this.baseUrl = normalizeBaseUrl(config.baseUrl);
    this.apiKey = config.apiKey;
    this.timeoutMs =
      config.timeoutMs === undefined ? DEFAULT_TIMEOUT_MS : config.timeoutMs;
  }

  // --- Endpoints ---------------------------------------------------------

  /** `GET /v1/health` — unauthenticated. */
  health(): Promise<HealthResponse> {
    return this.request<HealthResponse>({
      method: "GET",
      path: "/v1/health",
      auth: false,
    });
  }

  /** `GET /v1/capabilities` — lane availability and host limits. */
  capabilities(): Promise<unknown> {
    return this.request<unknown>({
      method: "GET",
      path: "/v1/capabilities",
      auth: true,
    });
  }

  /** `GET /v1/browser/profiles` — browser sandbox profile discovery. */
  browserProfiles(): Promise<unknown> {
    return this.request<unknown>({
      method: "GET",
      path: "/v1/browser/profiles",
      auth: true,
    });
  }

  /** `POST /v1/browser/admit` — browser sandbox admission preflight. */
  admitBrowserSession(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/admit",
      auth: true,
      body: request,
    });
  }

  /** `GET /v1/browser/adapter/contract` — browser adapter contract and conformance profile. */
  browserAdapterContract(): Promise<unknown> {
    return this.request<unknown>({
      method: "GET",
      path: "/v1/browser/adapter/contract",
      auth: true,
    });
  }

  /** `POST /v1/browser/adapter/capability` — issue a one-time adapter registration capability. */
  issueBrowserAdapterCapability(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/adapter/capability",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/browser/adapter/register` — fail-closed browser adapter registration preflight. */
  registerBrowserAdapter(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/adapter/register",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/browser/adapter/launch/plan` — fail-closed browser adapter launch plan preflight. */
  planBrowserAdapterLaunch(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/adapter/launch/plan",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/browser/adapter/launch/claim` — claim a server-issued launch request id once. */
  claimBrowserAdapterLaunch(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/adapter/launch/claim",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/browser/adapter/validate` — validate a proposed browser adapter manifest. */
  validateBrowserAdapter(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/adapter/validate",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/browser/adapter/completion/validate` — validate a completion report without trusting it. */
  validateBrowserAdapterCompletion(request: unknown): Promise<unknown> {
    return this.request<unknown>({
      method: "POST",
      path: "/v1/browser/adapter/completion/validate",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/execute` — run a program synchronously. */
  execute(request: ExecuteRequest): Promise<ExecutionResult> {
    return this.request<ExecutionResult>({
      method: "POST",
      path: "/v1/execute",
      auth: true,
      body: request,
    });
  }

  /** `POST /v1/jobs` — enqueue an asynchronous job (HTTP 202). */
  createJob(request: ExecuteRequest): Promise<CreateJobResponse> {
    return this.request<CreateJobResponse>({
      method: "POST",
      path: "/v1/jobs",
      auth: true,
      body: request,
    });
  }

  /** `GET /v1/jobs/{id}` — fetch a job record. */
  async getJob(jobId: string): Promise<JobRecord> {
    // `async` so an invalid job id surfaces as a rejected promise, not a
    // synchronous throw.
    return this.request<JobRecord>({
      method: "GET",
      path: `/v1/jobs/${encodeJobId(jobId)}`,
      auth: true,
    });
  }

  /** `DELETE /v1/jobs/{id}` — cancel a job (HTTP 204, returns void). */
  async cancelJob(jobId: string): Promise<void> {
    await this.request<void>({
      method: "DELETE",
      path: `/v1/jobs/${encodeJobId(jobId)}`,
      auth: true,
      expectNoContent: true,
    });
  }

  /** `GET /openapi.json` — the canonical spec (unauthenticated). */
  openapi(): Promise<unknown> {
    return this.request<unknown>({
      method: "GET",
      path: "/openapi.json",
      auth: false,
    });
  }

  // --- Transport ---------------------------------------------------------

  private async request<T>(opts: RequestOptions): Promise<T> {
    const url = this.baseUrl + opts.path;
    const headers: Record<string, string> = {};
    if (opts.auth && this.apiKey !== undefined) {
      headers[API_KEY_HEADER] = this.apiKey;
    }
    let payload: string | undefined;
    if (opts.body !== undefined) {
      headers["content-type"] = "application/json";
      payload = JSON.stringify(opts.body);
    }

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);

    let response: Response;
    try {
      response = await fetch(url, {
        method: opts.method,
        headers,
        body: payload,
        // Never follow redirects: a 3xx to another origin must not replay the
        // api-key header. `manual` surfaces the redirect as an opaque response.
        redirect: "manual",
        signal: controller.signal,
      });
    } catch (err) {
      if (err instanceof Error && err.name === "AbortError") {
        throw new BeatboxTransportError(
          `request timed out after ${this.timeoutMs}ms`,
          err,
        );
      }
      throw new BeatboxTransportError("network request failed", err);
    } finally {
      clearTimeout(timer);
    }

    // A redirect leaks through as an opaque (type "opaqueredirect", status 0)
    // or 3xx response; treat it as a transport failure rather than following.
    if (response.type === "opaqueredirect" || (response.status >= 300 && response.status < 400)) {
      throw new BeatboxTransportError(
        `unexpected redirect (status ${response.status})`,
      );
    }

    if (response.status < 200 || response.status >= 300) {
      throw await this.toApiError(response);
    }

    if (opts.expectNoContent || response.status === 204) {
      return undefined as T;
    }

    let text: string;
    try {
      text = await response.text();
    } catch (err) {
      throw new BeatboxTransportError("failed to read response body", err);
    }
    if (text === "") {
      return undefined as T;
    }
    try {
      return JSON.parse(text) as T;
    } catch (err) {
      throw new BeatboxTransportError("failed to parse JSON response", err);
    }
  }

  private async toApiError(response: Response): Promise<BeatboxApiError> {
    let code = "unknown";
    let message = `HTTP ${response.status}`;
    try {
      const text = await response.text();
      if (text !== "") {
        const parsed = JSON.parse(text) as {
          error?: { code?: unknown; message?: unknown };
        };
        const body = parsed?.error;
        if (body && typeof body.code === "string") code = body.code;
        if (body && typeof body.message === "string") message = body.message;
      }
    } catch {
      // Non-JSON or empty error body: keep the status-derived defaults.
    }
    return new BeatboxApiError(response.status, code, message);
  }
}
