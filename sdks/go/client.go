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

// Client is a beatbox daemon HTTP client. It is safe for concurrent use.
type Client struct {
	baseURL    string
	apiKey     string
	httpClient *http.Client
}

// Option configures a Client in New.
type Option func(*options)

type options struct {
	apiKey     string
	timeout    time.Duration
	timeoutSet bool
	httpClient *http.Client
}

// WithAPIKey sets the API key sent as the x-beatbox-api-key header on every
// request except Health and OpenAPI.
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
// installed on it if it does not already define one, so the API key cannot leak
// across a redirect.
func WithHTTPClient(hc *http.Client) Option {
	return func(o *options) { o.httpClient = hc }
}

// New constructs a Client for the daemon at baseURL (e.g.
// "http://127.0.0.1:7300"). Trailing slashes are trimmed.
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
	// response back to us as-is, so the API key is never re-sent to a new host.
	if hc.CheckRedirect == nil {
		hc.CheckRedirect = func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		}
	}

	return &Client{
		baseURL:    strings.TrimRight(baseURL, "/"),
		apiKey:     o.apiKey,
		httpClient: hc,
	}
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
	u, err := url.Parse(c.baseURL)
	if err != nil {
		return "", fmt.Errorf("beatbox: invalid base url: %w", err)
	}
	base := strings.TrimRight(u.Path, "/")
	// Set both the decoded Path and the escaped RawPath so String() emits the
	// escaped form: id becomes exactly one segment (any '/' or '?' in it is
	// percent-encoded), which prevents path traversal or query injection.
	u.Path = base + "/v1/jobs/" + id
	u.RawPath = base + "/v1/jobs/" + url.PathEscape(id)
	return u.String(), nil
}

// do performs a request, marshaling body as JSON (when non-nil) and decoding a
// successful response into out (a *json.RawMessage receives the raw bytes).
func (c *Client) do(ctx context.Context, method, rawURL string, auth bool, body, out any) error {
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
	if auth && c.apiKey != "" {
		req.Header.Set(apiKeyHeader, c.apiKey)
	}

	resp, err := c.httpClient.Do(req)
	if err != nil {
		// The API key is only ever sent as a header, never in the URL, so the
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
