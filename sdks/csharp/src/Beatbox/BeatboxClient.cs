using System;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Beatbox;

/// <summary>
/// A client for the beatbox sandbox REST API.
///
/// <para>
/// The client owns its <see cref="HttpClient"/>; construct one per base URL and
/// reuse it. It never follows redirects (so the token header cannot leak
/// cross-origin) and never embeds the token in an exception message.
/// </para>
/// </summary>
public sealed class BeatboxClient : IDisposable
{
    private const string AuthorizationHeader = "Authorization";
    private const string ApiKeyHeader = "x-beatbox-api-key";

    private readonly HttpClient _http;
    private readonly string _baseUrl;
    private readonly string? _token;
    private readonly string? _apiKey;

    /// <summary>
    /// Creates a client.
    /// </summary>
    /// <param name="baseUrl">
    /// Base URL of the daemon, e.g. <c>http://127.0.0.1:7300</c>. HTTPS is
    /// required except for exact loopback IP literals. Trailing slashes are trimmed.
    /// </param>
    /// <param name="apiKey">
    /// Legacy API-key compatibility alias. Used only when <paramref name="token"/> is not set.
    /// </param>
    /// <param name="timeout">Optional request timeout. Defaults to 65 seconds.</param>
    public BeatboxClient(string baseUrl, string? apiKey = null, TimeSpan? timeout = null)
        : this(baseUrl, apiKey, timeout, token: null)
    {
    }

    /// <summary>
    /// Creates a client with a Bearer token.
    /// </summary>
    /// <param name="baseUrl">
    /// Base URL of the daemon, e.g. <c>http://127.0.0.1:7300</c>. HTTPS is
    /// required except for exact loopback IP literals. Trailing slashes are trimmed.
    /// </param>
    /// <param name="apiKey">
    /// Legacy API-key compatibility alias. Used only when <paramref name="token"/> is not set.
    /// </param>
    /// <param name="timeout">Optional request timeout. Defaults to 65 seconds.</param>
    /// <param name="token">
    /// Optional Bearer token. When set, it is sent as <c>Authorization: Bearer &lt;token&gt;</c>
    /// on every request except <c>health</c> and <c>openapi</c>.
    /// </param>
    public BeatboxClient(string baseUrl, string? apiKey = null, TimeSpan? timeout = null, string? token = null)
    {
        _baseUrl = ValidateBaseUrl(baseUrl);
        _token = token;
        _apiKey = apiKey;

        var handler = CreateHttpHandler();
        _http = new HttpClient(handler, disposeHandler: true)
        {
            Timeout = timeout ?? TimeSpan.FromSeconds(65),
        };
    }

    /// <summary>GET <c>/v1/health</c> (unauthenticated). Returns the raw JSON body.</summary>
    public async Task<JsonElement> HealthAsync(CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, "/v1/health", auth: false, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>GET <c>/v1/capabilities</c>. Returns the raw JSON body.</summary>
    public async Task<JsonElement> CapabilitiesAsync(CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, "/v1/capabilities", auth: true, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>GET <c>/v1/integration</c>. Returns the raw ecosystem integration contract JSON.</summary>
    public async Task<JsonElement> IntegrationAsync(CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, "/v1/integration", auth: true, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>GET <c>/v1/browser/profiles</c>. Returns browser sandbox discovery metadata.</summary>
    public async Task<JsonElement> BrowserProfilesAsync(CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, "/v1/browser/profiles", auth: true, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/admit</c>. Returns a browser sandbox admission preflight decision.</summary>
    public async Task<JsonElement> AdmitBrowserSessionAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/admit", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>GET <c>/v1/browser/adapter/contract</c>. Returns browser adapter contract JSON.</summary>
    public async Task<JsonElement> BrowserAdapterContractAsync(CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, "/v1/browser/adapter/contract", auth: true, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/adapter/capability</c>. Returns browser adapter capability JSON.</summary>
    public async Task<JsonElement> IssueBrowserAdapterCapabilityAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/capability", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/adapter/register</c>. Returns browser adapter registration JSON.</summary>
    public async Task<JsonElement> RegisterBrowserAdapterAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/register", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/adapter/launch/plan</c>. Returns launch plan JSON.</summary>
    public async Task<JsonElement> PlanBrowserAdapterLaunchAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/launch/plan", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/adapter/launch/claim</c>. Returns launch claim JSON.</summary>
    public async Task<JsonElement> ClaimBrowserAdapterLaunchAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/launch/claim", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/adapter/validate</c>. Returns browser adapter manifest validation JSON.</summary>
    public async Task<JsonElement> ValidateBrowserAdapterAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/validate", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/browser/adapter/completion/validate</c>. Returns completion validation JSON.</summary>
    public async Task<JsonElement> ValidateBrowserAdapterCompletionAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/completion/validate", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/execute</c>. Runs the request synchronously.</summary>
    public async Task<ExecutionResult> ExecuteAsync(ExecuteRequest request, CancellationToken cancellationToken = default)
    {
        if (request is null)
        {
            throw new ArgumentNullException(nameof(request));
        }

        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/execute", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<ExecutionResult>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>POST <c>/v1/jobs</c>. Enqueues an asynchronous job (HTTP 202).</summary>
    public async Task<Operation> CreateJobAsync(ExecuteRequest request, CancellationToken cancellationToken = default)
    {
        if (request is null)
        {
            throw new ArgumentNullException(nameof(request));
        }

        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/jobs", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<Operation>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>GET <c>/v1/jobs/{id}</c>. Fetches a job record.</summary>
    public async Task<JobRecord> GetJobAsync(string jobId, CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, BuildJobPath(jobId), auth: true, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JobRecord>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>DELETE <c>/v1/jobs/{id}</c>. Cancels a job (HTTP 204).</summary>
    public async Task CancelJobAsync(string jobId, CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Delete, BuildJobPath(jobId), auth: true, content: null, cancellationToken)
            .ConfigureAwait(false);
        // No body on success (204); the response is disposed by the using block.
    }

    /// <summary>GET <c>/openapi.json</c> (unauthenticated). Returns the raw spec.</summary>
    public async Task<JsonElement> OpenApiAsync(CancellationToken cancellationToken = default)
    {
        using var response = await SendAsync(HttpMethod.Get, "/openapi.json", auth: false, content: null, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<JsonElement>(response, cancellationToken).ConfigureAwait(false);
    }

    /// <summary>Disposes the underlying <see cref="HttpClient"/> and its handler.</summary>
    public void Dispose() => _http.Dispose();

    /// <summary>
    /// Builds the path for a job endpoint, percent-encoding <paramref name="jobId"/>
    /// as a single path segment. Rejects ids that could retarget the request
    /// (<c>""</c>, <c>.</c>, <c>..</c>).
    /// </summary>
    internal static string BuildJobPath(string jobId)
    {
        if (jobId is null)
        {
            throw new ArgumentNullException(nameof(jobId));
        }

        if (jobId.Length == 0 || jobId == "." || jobId == "..")
        {
            throw new ArgumentException($"invalid job id: \"{jobId}\"", nameof(jobId));
        }

        return "/v1/jobs/" + Uri.EscapeDataString(jobId);
    }

    internal static string ValidateBaseUrl(string baseUrl)
    {
        if (string.IsNullOrEmpty(baseUrl))
        {
            throw new ArgumentException("baseUrl is required", nameof(baseUrl));
        }

        if (!string.Equals(baseUrl, baseUrl.Trim(), StringComparison.Ordinal))
        {
            throw new ArgumentException("baseUrl must not contain leading or trailing whitespace", nameof(baseUrl));
        }

        if (baseUrl.Contains('\\', StringComparison.Ordinal))
        {
            throw new ArgumentException("baseUrl must not contain backslashes", nameof(baseUrl));
        }

        if (!Uri.TryCreate(baseUrl, UriKind.Absolute, out var uri))
        {
            throw new ArgumentException("invalid baseUrl", nameof(baseUrl));
        }

        if (uri.Scheme != Uri.UriSchemeHttp && uri.Scheme != Uri.UriSchemeHttps)
        {
            throw new ArgumentException("baseUrl must use http or https", nameof(baseUrl));
        }

        if (string.IsNullOrEmpty(uri.Host))
        {
            throw new ArgumentException("baseUrl must include a host", nameof(baseUrl));
        }

        if (!string.IsNullOrEmpty(uri.UserInfo) || AuthorityIncludesUserInfo(baseUrl))
        {
            throw new ArgumentException("baseUrl must not include credentials", nameof(baseUrl));
        }

        if (!string.IsNullOrEmpty(uri.Query))
        {
            throw new ArgumentException("baseUrl must not include a query string", nameof(baseUrl));
        }

        if (!string.IsNullOrEmpty(uri.Fragment))
        {
            throw new ArgumentException("baseUrl must not include a fragment", nameof(baseUrl));
        }

        if (uri.Scheme == Uri.UriSchemeHttp && !IsLoopbackLiteral(ExtractRawHost(baseUrl)))
        {
            throw new ArgumentException("http baseUrl is allowed only for 127.0.0.1 or [::1]", nameof(baseUrl));
        }

        ValidateBasePath(ExtractRawPath(baseUrl));
        return baseUrl.TrimEnd('/');
    }

    internal static Uri BuildRequestUri(string baseUrl, string path)
    {
        if (!path.StartsWith("/", StringComparison.Ordinal))
        {
            throw new ArgumentException("request path must be absolute", nameof(path));
        }

        return new Uri(baseUrl + path, UriKind.Absolute);
    }

    internal static HttpClientHandler CreateHttpHandler() => new()
    {
        AllowAutoRedirect = false,
        UseProxy = false,
    };

    private async Task<HttpResponseMessage> SendAsync(
        HttpMethod method,
        string path,
        bool auth,
        HttpContent? content,
        CancellationToken cancellationToken)
    {
        using var request = new HttpRequestMessage(method, BuildRequestUri(_baseUrl, path));
        if (auth && !string.IsNullOrEmpty(_token))
        {
            request.Headers.TryAddWithoutValidation(AuthorizationHeader, $"Bearer {_token}");
        }
        else if (auth && !string.IsNullOrEmpty(_apiKey))
        {
            request.Headers.TryAddWithoutValidation(ApiKeyHeader, _apiKey);
        }

        request.Content = content;

        HttpResponseMessage response;
        try
        {
            response = await _http.SendAsync(request, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            // Caller-initiated cancellation: propagate as-is.
            throw;
        }
        catch (OperationCanceledException ex)
        {
            // HttpClient surfaces its own timeout as a canceled operation.
            throw new BeatboxTransportException(
                $"request timed out after {_http.Timeout.TotalSeconds:0.##}s", ex);
        }
        catch (HttpRequestException ex)
        {
            throw new BeatboxTransportException(ex.Message, ex);
        }

        if (!response.IsSuccessStatusCode)
        {
            try
            {
                await ThrowApiErrorAsync(response, cancellationToken).ConfigureAwait(false);
            }
            finally
            {
                response.Dispose();
            }
        }

        return response;
    }

    private static async Task ThrowApiErrorAsync(HttpResponseMessage response, CancellationToken cancellationToken)
    {
        int status = (int)response.StatusCode;
        string? code = null;
        string message = response.ReasonPhrase ?? $"HTTP {status}";

        try
        {
            var body = await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
            if (!string.IsNullOrWhiteSpace(body))
            {
                var error = JsonSerializer.Deserialize<ErrorResponse>(body, BeatboxJson.Options);
                if (error?.Error is { } detail)
                {
                    code = string.IsNullOrEmpty(detail.Code) ? null : detail.Code;
                    if (!string.IsNullOrEmpty(detail.Message))
                    {
                        message = detail.Message;
                    }
                }
            }
        }
        catch (JsonException)
        {
            // Non-JSON or malformed error body: keep the HTTP reason phrase.
        }
        catch (Exception ex) when (ex is HttpRequestException or System.IO.IOException)
        {
            // Body could not be read: keep the HTTP reason phrase.
        }

        throw new BeatboxApiException(status, code, message);
    }

    private static async Task<T> ReadJsonAsync<T>(HttpResponseMessage response, CancellationToken cancellationToken)
    {
        string body;
        try
        {
            body = await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is HttpRequestException or System.IO.IOException)
        {
            throw new BeatboxTransportException("failed to read response body", ex);
        }

        try
        {
            var value = JsonSerializer.Deserialize<T>(body, BeatboxJson.Options);
            if (value is null)
            {
                throw new BeatboxTransportException("server returned an empty or null JSON body");
            }

            return value;
        }
        catch (JsonException ex)
        {
            throw new BeatboxTransportException("failed to parse response body as JSON", ex);
        }
    }

    private static StringContent JsonContent<T>(T value)
    {
        var json = JsonSerializer.Serialize(value, BeatboxJson.Options);
        return new StringContent(json, Encoding.UTF8, "application/json");
    }

    private static bool IsLoopbackLiteral(string host) => host is "127.0.0.1" or "[::1]";

    private static bool AuthorityIncludesUserInfo(string raw)
    {
        var schemeEnd = raw.IndexOf("://", StringComparison.Ordinal);
        if (schemeEnd < 0)
        {
            return false;
        }

        var authorityStart = schemeEnd + 3;
        var authorityEnd = raw.IndexOfAny(['/', '?', '#'], authorityStart);
        var authority = authorityEnd < 0
            ? raw[authorityStart..]
            : raw[authorityStart..authorityEnd];
        return authority.Contains('@', StringComparison.Ordinal);
    }

    private static string ExtractRawPath(string raw)
    {
        var schemeEnd = raw.IndexOf("://", StringComparison.Ordinal);
        if (schemeEnd < 0)
        {
            return string.Empty;
        }

        var pathStart = raw.IndexOfAny(['/', '?', '#'], schemeEnd + 3);
        if (pathStart < 0 || raw[pathStart] != '/')
        {
            return string.Empty;
        }

        var pathEnd = raw.IndexOfAny(['?', '#'], pathStart);
        return pathEnd < 0 ? raw[pathStart..] : raw[pathStart..pathEnd];
    }

    private static string ExtractRawHost(string raw)
    {
        var schemeEnd = raw.IndexOf("://", StringComparison.Ordinal);
        if (schemeEnd < 0)
        {
            return string.Empty;
        }

        var authorityStart = schemeEnd + 3;
        var authorityEnd = raw.IndexOfAny(['/', '?', '#'], authorityStart);
        var authority = authorityEnd < 0
            ? raw[authorityStart..]
            : raw[authorityStart..authorityEnd];
        var userInfoEnd = authority.LastIndexOf('@');
        if (userInfoEnd >= 0)
        {
            authority = authority[(userInfoEnd + 1)..];
        }

        if (authority.StartsWith("[", StringComparison.Ordinal))
        {
            var bracketEnd = authority.IndexOf(']', StringComparison.Ordinal);
            if (bracketEnd < 0)
            {
                return authority;
            }

            var rest = authority[(bracketEnd + 1)..];
            return rest.Length == 0 || rest.StartsWith(":", StringComparison.Ordinal)
                ? authority[..(bracketEnd + 1)]
                : authority;
        }

        var portStart = authority.IndexOf(':', StringComparison.Ordinal);
        return portStart < 0 ? authority : authority[..portStart];
    }

    private static void ValidateBasePath(string rawPath)
    {
        if (string.IsNullOrEmpty(rawPath))
        {
            return;
        }

        if (rawPath.Contains("%2f", StringComparison.OrdinalIgnoreCase) ||
            rawPath.Contains("%5c", StringComparison.OrdinalIgnoreCase))
        {
            throw new ArgumentException("baseUrl path must not include encoded path separators", nameof(rawPath));
        }

        foreach (var segment in rawPath.Split('/'))
        {
            string decoded;
            try
            {
                decoded = Uri.UnescapeDataString(segment);
            }
            catch (UriFormatException ex)
            {
                throw new ArgumentException("invalid baseUrl path escape", nameof(rawPath), ex);
            }

            if (segment is "." or ".." || decoded is "." or "..")
            {
                throw new ArgumentException("baseUrl path must not include dot segments", nameof(rawPath));
            }
        }
    }
}
