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
/// reuse it. It never follows redirects (so the API key header cannot leak
/// cross-origin) and never embeds the API key in an exception message.
/// </para>
/// </summary>
public sealed class BeatboxClient : IDisposable
{
    private const string ApiKeyHeader = "x-beatbox-api-key";

    private readonly HttpClient _http;
    private readonly string _baseUrl;
    private readonly string? _apiKey;

    /// <summary>
    /// Creates a client.
    /// </summary>
    /// <param name="baseUrl">
    /// Base URL of the daemon, e.g. <c>http://127.0.0.1:7300</c>. Trailing slashes
    /// are trimmed.
    /// </param>
    /// <param name="apiKey">
    /// Optional API key. When set, it is sent as the <c>x-beatbox-api-key</c> header
    /// on every request except <c>health</c> and <c>openapi</c>.
    /// </param>
    /// <param name="timeout">Optional request timeout. Defaults to 65 seconds.</param>
    public BeatboxClient(string baseUrl, string? apiKey = null, TimeSpan? timeout = null)
    {
        if (string.IsNullOrWhiteSpace(baseUrl))
        {
            throw new ArgumentException("baseUrl is required", nameof(baseUrl));
        }

        _baseUrl = baseUrl.TrimEnd('/');
        _apiKey = apiKey;

        var handler = new HttpClientHandler { AllowAutoRedirect = false };
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

    /// <summary>POST <c>/v1/browser/adapter/validate</c>. Returns browser adapter manifest validation JSON.</summary>
    public async Task<JsonElement> ValidateBrowserAdapterAsync(JsonElement request, CancellationToken cancellationToken = default)
    {
        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/browser/adapter/validate", auth: true, content, cancellationToken)
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
    public async Task<CreateJobResponse> CreateJobAsync(ExecuteRequest request, CancellationToken cancellationToken = default)
    {
        if (request is null)
        {
            throw new ArgumentNullException(nameof(request));
        }

        using var content = JsonContent(request);
        using var response = await SendAsync(HttpMethod.Post, "/v1/jobs", auth: true, content, cancellationToken)
            .ConfigureAwait(false);
        return await ReadJsonAsync<CreateJobResponse>(response, cancellationToken).ConfigureAwait(false);
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

    private async Task<HttpResponseMessage> SendAsync(
        HttpMethod method,
        string path,
        bool auth,
        HttpContent? content,
        CancellationToken cancellationToken)
    {
        using var request = new HttpRequestMessage(method, _baseUrl + path);
        if (auth && _apiKey is not null)
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
}
