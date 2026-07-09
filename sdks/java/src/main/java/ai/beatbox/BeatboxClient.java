package ai.beatbox;

import ai.beatbox.internal.Json;
import ai.beatbox.model.ErrorResponse;
import ai.beatbox.model.ExecuteRequest;
import ai.beatbox.model.ExecutionResult;
import ai.beatbox.model.JobRecord;
import ai.beatbox.model.Operation;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.Objects;

/**
 * Client for the beatbox sandbox REST API.
 *
 * <p>Construct with the {@linkplain #builder() builder}:
 * <pre>{@code
 * BeatboxClient client = BeatboxClient.builder()
 *     .baseUrl("http://127.0.0.1:7300")
 *     .token(System.getenv("CRADLE_TOKEN"))
 *     .build();
 * ExecutionResult r = client.execute(ExecuteRequest.wasmWat(wat, Map.of("n", 41)));
 * }</pre>
 *
 * <p>Instances are thread-safe and may be shared. Redirects are never followed so the token
 * header cannot leak cross-origin.
 */
public final class BeatboxClient {

    private static final String AUTHORIZATION_HEADER = "Authorization";
    private static final String API_KEY_HEADER = "x-beatbox-api-key";
    private static final String JSON_CONTENT_TYPE = "application/json";

    private final String baseUrl;
    private final String token;
    private final String apiKey;
    private final Duration timeout;
    private final HttpClient httpClient;
    private final ObjectMapper mapper;

    private BeatboxClient(Builder builder) {
        this.baseUrl = builder.baseUrl;
        this.token = builder.token;
        this.apiKey = builder.apiKey;
        this.timeout = builder.timeout;
        this.mapper = builder.mapper != null ? builder.mapper : Json.newMapper();
        this.httpClient = builder.httpClient != null
                ? builder.httpClient
                : HttpClient.newBuilder()
                        .followRedirects(HttpClient.Redirect.NEVER)
                        .proxy(HttpClient.Builder.NO_PROXY)
                        .connectTimeout(builder.timeout)
                        .build();
    }

    public static Builder builder() {
        return new Builder();
    }

    // --- API methods ---------------------------------------------------------

    /** {@code GET /v1/health} (unauthenticated). Returns the raw JSON body. */
    public JsonNode health() {
        return sendForJson("GET", uri("/v1/health"), false, null);
    }

    /** {@code GET /v1/capabilities}. Returns the raw JSON body. */
    public JsonNode capabilities() {
        return sendForJson("GET", uri("/v1/capabilities"), true, null);
    }

    /** {@code GET /v1/integration}. Returns the raw ecosystem integration contract JSON. */
    public JsonNode integration() {
        return sendForJson("GET", uri("/v1/integration"), true, null);
    }

    /** {@code GET /v1/browser/profiles}. Returns browser sandbox discovery metadata. */
    public JsonNode browserProfiles() {
        return sendForJson("GET", uri("/v1/browser/profiles"), true, null);
    }

    /** {@code POST /v1/browser/admit}. Returns browser sandbox admission decision JSON. */
    public JsonNode browserAdmit(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/admit"), true, encode(request));
    }

    /** {@code GET /v1/browser/adapter/contract}. Returns browser adapter contract JSON. */
    public JsonNode browserAdapterContract() {
        return sendForJson("GET", uri("/v1/browser/adapter/contract"), true, null);
    }

    /** {@code POST /v1/browser/adapter/capability}. Returns browser adapter capability JSON. */
    public JsonNode issueBrowserAdapterCapability(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/adapter/capability"), true, encode(request));
    }

    /** {@code POST /v1/browser/adapter/register}. Returns browser adapter registration JSON. */
    public JsonNode registerBrowserAdapter(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/adapter/register"), true, encode(request));
    }

    /** {@code POST /v1/browser/adapter/launch/plan}. Returns launch plan JSON. */
    public JsonNode planBrowserAdapterLaunch(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/adapter/launch/plan"), true, encode(request));
    }

    /** {@code POST /v1/browser/adapter/launch/claim}. Returns launch claim JSON. */
    public JsonNode claimBrowserAdapterLaunch(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/adapter/launch/claim"), true, encode(request));
    }

    /** {@code POST /v1/browser/adapter/validate}. Returns browser adapter manifest validation JSON. */
    public JsonNode validateBrowserAdapter(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/adapter/validate"), true, encode(request));
    }

    /** {@code POST /v1/browser/adapter/completion/validate}. Returns completion validation JSON. */
    public JsonNode validateBrowserAdapterCompletion(JsonNode request) {
        Objects.requireNonNull(request, "request");
        return sendForJson("POST", uri("/v1/browser/adapter/completion/validate"), true, encode(request));
    }

    /** {@code POST /v1/execute}. Runs the request synchronously. */
    public ExecutionResult execute(ExecuteRequest request) {
        Objects.requireNonNull(request, "request");
        return send("POST", uri("/v1/execute"), true, encode(request), ExecutionResult.class);
    }

    /** {@code POST /v1/jobs}. Enqueues an asynchronous job (HTTP 202). */
    public Operation createJob(ExecuteRequest request) {
        Objects.requireNonNull(request, "request");
        return send("POST", uri("/v1/jobs"), true, encode(request), Operation.class);
    }

    /** {@code GET /v1/jobs/{id}}. Fetches the current record for a job. */
    public JobRecord getJob(String jobId) {
        return send("GET", jobUri(jobId), true, null, JobRecord.class);
    }

    /** {@code DELETE /v1/jobs/{id}}. Cancels a job (HTTP 204; no body). */
    public void cancelJob(String jobId) {
        HttpResponse<byte[]> response = exchange("DELETE", jobUri(jobId), true, null);
        ensureSuccess(response);
    }

    /** {@code GET /openapi.json} (unauthenticated). Returns the raw spec as JSON. */
    public JsonNode openapi() {
        return sendForJson("GET", uri("/openapi.json"), false, null);
    }

    // --- URI construction ----------------------------------------------------

    private URI uri(String path) {
        return URI.create(baseUrl + path);
    }

    /**
     * Builds the job URI so that {@code jobId} is a single, fully percent-encoded path segment.
     * Rejects ids that could retarget the request ({@code ""}, {@code "."}, {@code ".."}).
     */
    URI jobUri(String jobId) {
        Objects.requireNonNull(jobId, "jobId");
        if (jobId.isEmpty() || jobId.equals(".") || jobId.equals("..")) {
            throw new IllegalArgumentException("Invalid job id: '" + jobId + "'");
        }
        return URI.create(baseUrl + "/v1/jobs/" + encodePathSegment(jobId));
    }

    /** Percent-encodes everything outside the RFC 3986 unreserved set, so it is one path segment. */
    static String encodePathSegment(String segment) {
        StringBuilder sb = new StringBuilder(segment.length());
        for (byte b : segment.getBytes(StandardCharsets.UTF_8)) {
            int c = b & 0xFF;
            boolean unreserved = (c >= 'A' && c <= 'Z')
                    || (c >= 'a' && c <= 'z')
                    || (c >= '0' && c <= '9')
                    || c == '-' || c == '_' || c == '.' || c == '~';
            if (unreserved) {
                sb.append((char) c);
            } else {
                sb.append('%');
                sb.append(Character.toUpperCase(Character.forDigit((c >> 4) & 0xF, 16)));
                sb.append(Character.toUpperCase(Character.forDigit(c & 0xF, 16)));
            }
        }
        return sb.toString();
    }

    // --- HTTP plumbing -------------------------------------------------------

    private <T> T send(String method, URI uri, boolean auth, byte[] body, Class<T> type) {
        HttpResponse<byte[]> response = exchange(method, uri, auth, body);
        ensureSuccess(response);
        return decode(response, type);
    }

    private JsonNode sendForJson(String method, URI uri, boolean auth, byte[] body) {
        HttpResponse<byte[]> response = exchange(method, uri, auth, body);
        ensureSuccess(response);
        try {
            return mapper.readTree(response.body());
        } catch (IOException e) {
            throw new BeatboxTransportException("Failed to parse JSON response body", e);
        }
    }

    private HttpResponse<byte[]> exchange(String method, URI uri, boolean auth, byte[] body) {
        HttpRequest.Builder builder = HttpRequest.newBuilder(uri).timeout(timeout);
        builder.header("accept", JSON_CONTENT_TYPE);
        if (body != null) {
            builder.method(method, HttpRequest.BodyPublishers.ofByteArray(body));
            builder.header("content-type", JSON_CONTENT_TYPE);
        } else {
            builder.method(method, HttpRequest.BodyPublishers.noBody());
        }
        if (auth && token != null && !token.isEmpty()) {
            builder.header(AUTHORIZATION_HEADER, "Bearer " + token);
        } else if (auth && apiKey != null && !apiKey.isEmpty()) {
            builder.header(API_KEY_HEADER, apiKey);
        }
        try {
            return httpClient.send(builder.build(), HttpResponse.BodyHandlers.ofByteArray());
        } catch (IOException e) {
            // The URI carries no credentials, so it is safe to name the endpoint.
            throw new BeatboxTransportException("Transport error for " + method + " " + uri, e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new BeatboxTransportException("Request interrupted for " + method + " " + uri, e);
        }
    }

    private void ensureSuccess(HttpResponse<byte[]> response) {
        int status = response.statusCode();
        if (status / 100 == 2) {
            return;
        }
        throw apiExceptionFor(status, response.body(), mapper);
    }

    /**
     * Builds the typed error for a non-2xx response. Any error body shape must
     * still yield a {@link BeatboxApiException} carrying the status — an empty,
     * non-JSON, or unexpectedly-shaped body (including the JSON literal {@code
     * null}) falls back to a generic message rather than throwing.
     */
    static BeatboxApiException apiExceptionFor(int status, byte[] body, ObjectMapper mapper) {
        String code = null;
        String message = null;
        try {
            ErrorResponse parsed = mapper.readValue(body, ErrorResponse.class);
            // readValue("null", ...) returns null; guard before dereferencing.
            if (parsed != null && parsed.error() != null) {
                code = parsed.error().code();
                message = parsed.error().message();
            }
        } catch (IOException ignored) {
            // Non-JSON or empty error body; fall through to a generic message.
        }
        if (message == null || message.isBlank()) {
            message = "HTTP " + status;
        }
        return new BeatboxApiException(status, code, message);
    }

    private <T> T decode(HttpResponse<byte[]> response, Class<T> type) {
        try {
            return mapper.readValue(response.body(), type);
        } catch (IOException e) {
            throw new BeatboxTransportException("Failed to decode " + type.getSimpleName() + " response", e);
        }
    }

    private byte[] encode(Object value) {
        try {
            return mapper.writeValueAsBytes(value);
        } catch (IOException e) {
            throw new BeatboxTransportException("Failed to encode request body", e);
        }
    }

    // --- Accessors -----------------------------------------------------------

    public String baseUrl() {
        return baseUrl;
    }

    public Duration timeout() {
        return timeout;
    }

    /** The JSON mapper used by this client (pre-configured for the beatbox wire format). */
    public ObjectMapper objectMapper() {
        return mapper;
    }

    // --- Builder -------------------------------------------------------------

    /** Builder for {@link BeatboxClient}. {@code baseUrl} is required; the rest have defaults. */
    public static final class Builder {
        private String baseUrl;
        private String token;
        private String apiKey;
        private Duration timeout = Duration.ofSeconds(65);
        private HttpClient httpClient;
        private ObjectMapper mapper;

        private Builder() {
        }

        /** Required. Must be HTTPS, or loopback-literal HTTP for local development. */
        public Builder baseUrl(String baseUrl) {
            this.baseUrl = baseUrl;
            return this;
        }

        /** Optional Bearer token sent as {@code Authorization: Bearer <token>} on authenticated requests. */
        public Builder token(String token) {
            this.token = token;
            return this;
        }

        /** Legacy API-key compatibility alias used only when {@code token} is not set. */
        public Builder apiKey(String apiKey) {
            this.apiKey = apiKey;
            return this;
        }

        /** Per-request timeout. Defaults to 65 seconds. */
        public Builder timeout(Duration timeout) {
            this.timeout = Objects.requireNonNull(timeout, "timeout");
            return this;
        }

        /** Supply a custom {@link HttpClient}. It must not follow redirects or use a proxy. */
        public Builder httpClient(HttpClient httpClient) {
            this.httpClient = httpClient;
            return this;
        }

        /** Supply a custom {@link ObjectMapper} (must serialize the beatbox wire format). */
        public Builder objectMapper(ObjectMapper mapper) {
            this.mapper = mapper;
            return this;
        }

        public BeatboxClient build() {
            if (baseUrl == null || baseUrl.isBlank()) {
                throw new IllegalArgumentException("baseUrl is required");
            }
            this.baseUrl = normalizeBaseUrl(baseUrl);
            if (httpClient != null && httpClient.followRedirects() != HttpClient.Redirect.NEVER) {
                throw new IllegalArgumentException("httpClient must not follow redirects");
            }
            if (httpClient != null && httpClient.proxy().isPresent()) {
                throw new IllegalArgumentException("httpClient must not use a proxy");
            }
            if (httpClient != null && URI.create(this.baseUrl).getScheme().equalsIgnoreCase("http")) {
                throw new IllegalArgumentException("custom httpClient is not allowed with plaintext http baseUrl");
            }
            return new BeatboxClient(this);
        }

        private static String normalizeBaseUrl(String value) {
            if (!value.strip().equals(value)) {
                throw new IllegalArgumentException("baseUrl must not contain leading or trailing whitespace");
            }

            URI uri;
            try {
                uri = URI.create(value);
            } catch (IllegalArgumentException e) {
                throw new IllegalArgumentException("baseUrl must be an absolute URL", e);
            }

            String scheme = uri.getScheme();
            if (scheme == null || uri.getHost() == null) {
                throw new IllegalArgumentException("baseUrl must be an absolute URL");
            }
            if (!scheme.equalsIgnoreCase("https") && !scheme.equalsIgnoreCase("http")) {
                throw new IllegalArgumentException("baseUrl must use http or https");
            }
            if (uri.getUserInfo() != null) {
                throw new IllegalArgumentException("baseUrl must not include credentials");
            }
            if (uri.getRawQuery() != null || uri.getRawFragment() != null) {
                throw new IllegalArgumentException("baseUrl must not include query or fragment");
            }
            if (value.contains("\\")) {
                throw new IllegalArgumentException("baseUrl path must not include backslashes");
            }
            if (scheme.equalsIgnoreCase("http")
                    && !uri.getHost().equals("127.0.0.1")
                    && !uri.getHost().equals("::1")
                    && !uri.getHost().equals("[::1]")) {
                throw new IllegalArgumentException(
                        "baseUrl may use plaintext http only with loopback IP literals");
            }
            validateBasePath(uri.getRawPath());
            return trimTrailingSlashes(uri.toASCIIString());
        }

        private static void validateBasePath(String rawPath) {
            if (rawPath == null || rawPath.isEmpty()) {
                return;
            }
            for (String segment : rawPath.split("/", -1)) {
                if (segment.isEmpty()) {
                    continue;
                }
                String decoded = decodePathSegment(segment);
                if (decoded.equals(".") || decoded.equals("..")) {
                    throw new IllegalArgumentException("baseUrl path must not contain dot segments");
                }
                if (decoded.contains("/") || decoded.contains("\\")) {
                    throw new IllegalArgumentException("baseUrl path segments must not encode separators");
                }
            }
        }

        private static String decodePathSegment(String segment) {
            ByteArrayOutputStream out = new ByteArrayOutputStream(segment.length());
            for (int i = 0; i < segment.length(); i++) {
                char ch = segment.charAt(i);
                if (ch != '%') {
                    out.write((byte) ch);
                    continue;
                }
                if (i + 2 >= segment.length()) {
                    throw new IllegalArgumentException("baseUrl path contains invalid percent encoding");
                }
                int hi = Character.digit(segment.charAt(i + 1), 16);
                int lo = Character.digit(segment.charAt(i + 2), 16);
                if (hi < 0 || lo < 0) {
                    throw new IllegalArgumentException("baseUrl path contains invalid percent encoding");
                }
                out.write((hi << 4) + lo);
                i += 2;
            }
            return new String(out.toByteArray(), StandardCharsets.UTF_8);
        }

        private static String trimTrailingSlashes(String value) {
            int end = value.length();
            while (end > 0 && value.charAt(end - 1) == '/') {
                end--;
            }
            return value.substring(0, end);
        }
    }
}
