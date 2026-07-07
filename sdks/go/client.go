package beatbox

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// DefaultTimeout is the request timeout used when WithTimeout is not supplied.
const DefaultTimeout = 65 * time.Second

const apiKeyHeader = "x-beatbox-api-key"
const authorizationHeader = "Authorization"

// Client is a beatbox daemon HTTP client. It is safe for concurrent use.
type Client struct {
	baseURL    string
	baseURLErr error
	token      string
	apiKey     string
	httpClient *http.Client
}

// Option configures a Client in New.
type Option func(*options)

type options struct {
	token      string
	apiKey     string
	timeout    time.Duration
	timeoutSet bool
	httpClient *http.Client
}

// WithToken sets the Bearer token sent as Authorization: Bearer <token> on
// every request except Health and OpenAPI. It is the canonical auth option for
// shared ecosystem clients.
func WithToken(token string) Option {
	return func(o *options) { o.token = token }
}

// WithAPIKey sets the legacy API-key compatibility header on every request
// except Health and OpenAPI. WithToken takes precedence when both are set.
func WithAPIKey(key string) Option {
	return func(o *options) { o.apiKey = key }
}

// WithTimeout sets the per-request timeout (default DefaultTimeout). When
// combined with WithHTTPClient, it overrides that client's Timeout.
func WithTimeout(d time.Duration) Option {
	return func(o *options) {
		o.timeout = d
		o.timeoutSet = true
	}
}

// WithHTTPClient supplies a custom *http.Client. A no-follow redirect policy is
// installed on it, so auth headers cannot leak across a redirect.
func WithHTTPClient(hc *http.Client) Option {
	return func(o *options) { o.httpClient = hc }
}

// New constructs a Client for the daemon at baseURL (e.g.
// "http://127.0.0.1:7300"). Trailing slashes are trimmed. Invalid base URLs
// are rejected by every request method before an HTTP request is built.
func New(baseURL string, opts ...Option) *Client {
	o := options{timeout: DefaultTimeout}
	for _, opt := range opts {
		opt(&o)
	}

	hc := o.httpClient
	if hc == nil {
		hc = &http.Client{Timeout: o.timeout}
	} else if o.timeoutSet {
		hc.Timeout = o.timeout
	}
	// Never follow redirects: returning ErrUseLastResponse hands the redirect
	// response back to us as-is, so auth is never re-sent to a new host.
	hc.CheckRedirect = func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	}

	normalizedBaseURL, baseURLErr := normalizeBaseURL(baseURL)
	return &Client{
		baseURL:    normalizedBaseURL,
		baseURLErr: baseURLErr,
		token:      o.token,
		apiKey:     o.apiKey,
		httpClient: hc,
	}
}

func normalizeBaseURL(raw string) (string, error) {
	if raw == "" {
		return "", fmt.Errorf("beatbox: base url is required")
	}
	if strings.TrimSpace(raw) != raw {
		return "", fmt.Errorf("beatbox: invalid base url: must not contain leading or trailing whitespace")
	}
	u, err := url.Parse(raw)
	if err != nil || !u.IsAbs() || u.Host == "" {
		return "", fmt.Errorf("beatbox: invalid base url: must be an absolute URL")
	}
	if u.Scheme != "https" && u.Scheme != "http" {
		return "", fmt.Errorf("beatbox: invalid base url: must use http or https")
	}
	if u.User != nil {
		return "", fmt.Errorf("beatbox: invalid base url: must not include credentials")
	}
	if u.RawQuery != "" || u.Fragment != "" {
		return "", fmt.Errorf("beatbox: invalid base url: must not include query or fragment")
	}
	if strings.Contains(raw, "\\") {
		return "", fmt.Errorf("beatbox: invalid base url: path must not include backslashes")
	}
	if u.Scheme == "http" && u.Hostname() != "127.0.0.1" && u.Hostname() != "::1" {
		return "", fmt.Errorf("beatbox: invalid base url: plaintext http is allowed only with loopback IP literals")
	}
	if err := validateBaseURLPath(u); err != nil {
		return "", err
	}
	u.Path = strings.TrimRight(u.Path, "/")
	u.RawPath = strings.TrimRight(u.RawPath, "/")
	return u.String(), nil
}

func validateBaseURLPath(u *url.URL) error {
	for _, segment := range strings.Split(u.EscapedPath(), "/") {
		if segment == "" {
			continue
		}
		decoded, err := url.PathUnescape(segment)
		if err != nil {
			return fmt.Errorf("beatbox: invalid base url: path contains invalid percent encoding")
		}
		if decoded == "." || decoded == ".." {
			return fmt.Errorf("beatbox: invalid base url: path must not contain dot segments")
		}
		if strings.Contains(decoded, "/") || strings.Contains(decoded, "\\") {
			return fmt.Errorf("beatbox: invalid base url: path segments must not encode separators")
		}
	}
	return nil
}

// Health returns the daemon health payload (GET /v1/health, unauthenticated).
func (c *Client) Health(ctx context.Context) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodGet, c.baseURL+"/v1/health", false, nil, &out)
	return out, err
}

// Capabilities returns lane availability and host limits (GET /v1/capabilities).
func (c *Client) Capabilities(ctx context.Context) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodGet, c.baseURL+"/v1/capabilities", true, nil, &out)
	return out, err
}

// Integration returns the ecosystem integration contract (GET /v1/integration).
func (c *Client) Integration(ctx context.Context) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodGet, c.baseURL+"/v1/integration", true, nil, &out)
	return out, err
}

// BrowserProfiles returns browser sandbox profile discovery metadata
// (GET /v1/browser/profiles).
func (c *Client) BrowserProfiles(ctx context.Context) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodGet, c.baseURL+"/v1/browser/profiles", true, nil, &out)
	return out, err
}

// AdmitBrowserSession returns a browser sandbox admission preflight decision
// (POST /v1/browser/admit).
func (c *Client) AdmitBrowserSession(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/admit", true, req, &out)
	return out, err
}

// BrowserAdapterContract returns the planned browser adapter contract and
// conformance profile without trusting or launching an adapter
// (GET /v1/browser/adapter/contract).
func (c *Client) BrowserAdapterContract(ctx context.Context) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodGet, c.baseURL+"/v1/browser/adapter/contract", true, nil, &out)
	return out, err
}

// IssueBrowserAdapterCapability issues a short-lived one-time same-user
// capability for browser adapter registration (POST /v1/browser/adapter/capability).
func (c *Client) IssueBrowserAdapterCapability(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/adapter/capability", true, req, &out)
	return out, err
}

// RegisterBrowserAdapter submits a fail-closed browser adapter registration
// preflight without trusting or launching it (POST /v1/browser/adapter/register).
func (c *Client) RegisterBrowserAdapter(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/adapter/register", true, req, &out)
	return out, err
}

// PlanBrowserAdapterLaunch prepares a fail-closed browser adapter launch
// envelope without trusting or launching it (POST /v1/browser/adapter/launch/plan).
func (c *Client) PlanBrowserAdapterLaunch(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/adapter/launch/plan", true, req, &out)
	return out, err
}

// ClaimBrowserAdapterLaunch claims a server-issued browser adapter launch
// request once without trusting or launching it (POST /v1/browser/adapter/launch/claim).
func (c *Client) ClaimBrowserAdapterLaunch(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/adapter/launch/claim", true, req, &out)
	return out, err
}

// ValidateBrowserAdapter validates a proposed browser adapter manifest without
// trusting or launching it (POST /v1/browser/adapter/validate).
func (c *Client) ValidateBrowserAdapter(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/adapter/validate", true, req, &out)
	return out, err
}

// ValidateBrowserAdapterCompletion validates a browser adapter completion
// report without trusting it (POST /v1/browser/adapter/completion/validate).
func (c *Client) ValidateBrowserAdapterCompletion(ctx context.Context, req any) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/browser/adapter/completion/validate", true, req, &out)
	return out, err
}

// Execute runs the request synchronously (POST /v1/execute) and returns the
// ExecutionResult.
func (c *Client) Execute(ctx context.Context, req ExecuteRequest) (*ExecutionResult, error) {
	var out ExecutionResult
	if err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/execute", true, req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// CreateJob enqueues an asynchronous job (POST /v1/jobs) and returns its id.
func (c *Client) CreateJob(ctx context.Context, req ExecuteRequest) (*CreateJobResponse, error) {
	var out CreateJobResponse
	if err := c.do(ctx, http.MethodPost, c.baseURL+"/v1/jobs", true, req, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// GetJob fetches a job record by id (GET /v1/jobs/{id}).
func (c *Client) GetJob(ctx context.Context, id string) (*JobRecord, error) {
	u, err := c.jobURL(id)
	if err != nil {
		return nil, err
	}
	var out JobRecord
	if err := c.do(ctx, http.MethodGet, u, true, nil, &out); err != nil {
		return nil, err
	}
	return &out, nil
}

// CancelJob cancels a job by id (DELETE /v1/jobs/{id}).
func (c *Client) CancelJob(ctx context.Context, id string) error {
	u, err := c.jobURL(id)
	if err != nil {
		return err
	}
	return c.do(ctx, http.MethodDelete, u, true, nil, nil)
}

// OpenAPI returns the daemon's OpenAPI document (GET /openapi.json,
// unauthenticated).
func (c *Client) OpenAPI(ctx context.Context) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.do(ctx, http.MethodGet, c.baseURL+"/openapi.json", false, nil, &out)
	return out, err
}

// jobURL builds the URL for /v1/jobs/{id}, encoding id as a single path segment
// so it cannot retarget the request. It rejects "", "." and ".." outright.
func (c *Client) jobURL(id string) (string, error) {
	if id == "" || id == "." || id == ".." {
		return "", fmt.Errorf("beatbox: invalid job id %q", id)
	}
	if c.baseURLErr != nil {
		return "", c.baseURLErr
	}
	u, err := url.Parse(c.baseURL)
	if err != nil {
		return "", fmt.Errorf("beatbox: invalid base url: %w", err)
	}
	basePath := strings.TrimRight(u.Path, "/")
	baseRawPath := strings.TrimRight(u.EscapedPath(), "/")
	// Set both the decoded Path and the escaped RawPath so String() emits the
	// escaped form: id becomes exactly one segment (any '/' or '?' in it is
	// percent-encoded), which prevents path traversal or query injection.
	u.Path = basePath + "/v1/jobs/" + id
	u.RawPath = baseRawPath + "/v1/jobs/" + url.PathEscape(id)
	return u.String(), nil
}

// do performs a request, marshaling body as JSON (when non-nil) and decoding a
// successful response into out (a *json.RawMessage receives the raw bytes).
func (c *Client) do(ctx context.Context, method, rawURL string, auth bool, body, out any) error {
	if c.baseURLErr != nil {
		return c.baseURLErr
	}

	var reader io.Reader
	if body != nil {
		encoded, err := json.Marshal(body)
		if err != nil {
			return fmt.Errorf("beatbox: encode request: %w", err)
		}
		reader = bytes.NewReader(encoded)
	}

	req, err := http.NewRequestWithContext(ctx, method, rawURL, reader)
	if err != nil {
		return fmt.Errorf("beatbox: build request: %w", err)
	}
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	if auth && c.token != "" {
		req.Header.Set(authorizationHeader, "Bearer "+c.token)
	} else if auth && c.apiKey != "" {
		req.Header.Set(apiKeyHeader, c.apiKey)
	}

	resp, err := c.httpClient.Do(req)
	if err != nil {
		// Auth is only ever sent as a header, never in the URL, so the
		// wrapped transport error cannot contain it.
		return fmt.Errorf("beatbox: request failed: %w", err)
	}
	defer resp.Body.Close()

	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return fmt.Errorf("beatbox: read response: %w", err)
	}

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return parseAPIError(resp.StatusCode, data)
	}

	if out == nil || len(data) == 0 {
		return nil
	}
	if raw, ok := out.(*json.RawMessage); ok {
		*raw = append((*raw)[:0], data...)
		return nil
	}
	if err := json.Unmarshal(data, out); err != nil {
		return fmt.Errorf("beatbox: decode response: %w", err)
	}
	return nil
}
