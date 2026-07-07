# frozen_string_literal: true

require "net/http"
require "uri"
require "json"

module Beatbox
  # HTTP client for the beatbox sandbox REST API.
  #
  #   client = Beatbox::Client.new(base_url: "http://127.0.0.1:7300",
  #                                token: ENV["CRADLE_TOKEN"])
  #   result = client.execute(Beatbox::ExecuteRequest.wasm_wat(wat, input: { "n" => 41 }))
  #   result.value # => 42
  #
  # Zero third-party dependencies: net/http + json from the standard library.
  class Client
    DEFAULT_TIMEOUT = 65
    AUTHORIZATION_HEADER = "Authorization"
    API_KEY_HEADER = "x-beatbox-api-key"

    # @return [String] the normalized base url (trailing slashes trimmed)
    attr_reader :base_url
    # @return [Numeric] per-request timeout in seconds
    attr_reader :timeout

    # @param base_url [String] required, e.g. "http://127.0.0.1:7300"
    # @param token [String, nil] sent as Authorization: Bearer <token>
    # @param api_key [String, nil] legacy x-beatbox-api-key compatibility alias
    # @param timeout [Numeric] open/read/write timeout in seconds (default 65)
    # @param timeout_ms [Numeric, nil] preferred shared config timeout in ms
    def initialize(base_url:, token: nil, api_key: nil, timeout: DEFAULT_TIMEOUT, timeout_ms: nil)
      @base_url = self.class.validate_base_url(base_url)
      @token = token
      @api_key = api_key
      @timeout = timeout_ms ? timeout_ms / 1000.0 : timeout
    end

    # GET /v1/health (unauthenticated). Returns raw JSON as a Hash.
    def health
      request_json(:get, "/v1/health", auth: false)
    end

    # GET /v1/capabilities. Returns raw JSON as a Hash.
    def capabilities
      request_json(:get, "/v1/capabilities", auth: true)
    end

    # GET /v1/integration. Returns raw ecosystem integration contract JSON.
    def integration
      request_json(:get, "/v1/integration", auth: true)
    end

    # GET /v1/browser/profiles. Returns browser sandbox discovery metadata.
    def browser_profiles
      request_json(:get, "/v1/browser/profiles", auth: true)
    end

    # POST /v1/browser/admit. Returns browser sandbox admission decision JSON.
    def browser_admit(request)
      request_json(:post, "/v1/browser/admit", body: request, auth: true)
    end

    # GET /v1/browser/adapter/contract. Returns browser adapter contract JSON.
    def browser_adapter_contract
      request_json(:get, "/v1/browser/adapter/contract", auth: true)
    end

    # POST /v1/browser/adapter/capability. Returns browser adapter capability JSON.
    def browser_adapter_capability(request)
      request_json(:post, "/v1/browser/adapter/capability", body: request, auth: true)
    end

    # POST /v1/browser/adapter/register. Returns browser adapter registration JSON.
    def browser_adapter_register(request)
      request_json(:post, "/v1/browser/adapter/register", body: request, auth: true)
    end

    # POST /v1/browser/adapter/launch/plan. Returns browser adapter launch plan JSON.
    def browser_adapter_launch_plan(request)
      request_json(:post, "/v1/browser/adapter/launch/plan", body: request, auth: true)
    end

    # POST /v1/browser/adapter/launch/claim. Returns browser adapter launch claim JSON.
    def browser_adapter_launch_claim(request)
      request_json(:post, "/v1/browser/adapter/launch/claim", body: request, auth: true)
    end

    # POST /v1/browser/adapter/validate. Returns browser adapter validation JSON.
    def validate_browser_adapter(request)
      request_json(:post, "/v1/browser/adapter/validate", body: request, auth: true)
    end

    # POST /v1/browser/adapter/completion/validate. Returns completion validation JSON.
    def validate_browser_adapter_completion(request)
      request_json(:post, "/v1/browser/adapter/completion/validate", body: request, auth: true)
    end

    # POST /v1/execute. @return [ExecutionResult]
    def execute(request)
      body = request_json(:post, "/v1/execute", body: request, auth: true)
      ExecutionResult.from_h(body)
    end

    # POST /v1/jobs. @return [CreateJobResponse]
    def create_job(request)
      body = request_json(:post, "/v1/jobs", body: request, auth: true)
      CreateJobResponse.from_h(body)
    end

    # GET /v1/jobs/{id}. @return [JobRecord]
    def get_job(job_id)
      path = "/v1/jobs/#{Util.encode_path_segment(job_id)}"
      JobRecord.from_h(request_json(:get, path, auth: true))
    end

    # DELETE /v1/jobs/{id} (204). @return [nil]
    def cancel_job(job_id)
      path = "/v1/jobs/#{Util.encode_path_segment(job_id)}"
      request_json(:delete, path, auth: true)
      nil
    end

    # GET /openapi.json (unauthenticated). Returns raw JSON as a Hash.
    def openapi
      request_json(:get, "/openapi.json", auth: false)
    end

    private

    def request_json(method, path, body: nil, auth: false)
      uri = build_uri(path)
      http = build_http(uri)
      req = build_request(method, uri, body, auth)

      begin
        response = http.request(req)
      rescue StandardError => e
        # Never include headers (and thus the api key) in the message.
        raise TransportError, "transport error contacting beatbox: #{e.class}: #{e.message}"
      end

      handle_response(response)
    end

    def build_uri(path)
      raise ArgumentError, "request path must be absolute" unless path.start_with?("/")

      URI.parse(@base_url + path)
    end

    def build_http(uri)
      http = Net::HTTP.new(uri.hostname, uri.port, nil)
      http.use_ssl = uri.scheme == "https"
      http.open_timeout = @timeout
      http.read_timeout = @timeout
      http.write_timeout = @timeout if http.respond_to?(:write_timeout=)
      http.max_retries = 0 if http.respond_to?(:max_retries=)
      http
    end

    def build_request(method, uri, body, auth)
      klass = request_class(method)
      req = klass.new(uri.request_uri)
      req["accept"] = "application/json"
      if auth && @token && !@token.empty?
        req[AUTHORIZATION_HEADER] = "Bearer #{@token}"
      elsif auth && @api_key && !@api_key.empty?
        req[API_KEY_HEADER] = @api_key
      end

      if body
        req["content-type"] = "application/json"
        payload = body.respond_to?(:to_h) ? body.to_h : body
        req.body = JSON.generate(payload)
      end

      req
    end

    def request_class(method)
      case method
      when :get then Net::HTTP::Get
      when :post then Net::HTTP::Post
      when :delete then Net::HTTP::Delete
      else raise ArgumentError, "unsupported method: #{method}"
      end
    end

    # We call Net::HTTP#request directly, which does NOT follow redirects, so a
    # 3xx is surfaced as an ApiError rather than replaying auth headers to
    # another origin.
    def handle_response(response)
      code = response.code.to_i
      body = response.body

      return parse_success(code, body) if code >= 200 && code < 300

      raise build_api_error(code, body)
    end

    def parse_success(code, body)
      return nil if code == 204 || body.nil? || body.strip.empty?

      JSON.parse(body)
    end

    def build_api_error(code, body)
      err_code = nil
      err_message = nil

      if body && !body.empty?
        begin
          parsed = JSON.parse(body)
          if parsed.is_a?(Hash) && parsed["error"].is_a?(Hash)
            err_code = parsed["error"]["code"]
            err_message = parsed["error"]["message"]
          end
        rescue JSON::ParserError
          # Non-JSON error body; fall back to a generic message.
        end
      end

      ApiError.new(status: code, code: err_code, message: err_message)
    end

    def self.validate_base_url(base_url)
      raw = base_url.to_s
      raise ArgumentError, "base_url is required" if raw.empty?
      raise ArgumentError, "base_url must not contain leading or trailing whitespace" if raw != raw.strip
      raise ArgumentError, "base_url must not contain backslashes" if raw.include?("\\")

      uri = parse_base_url(raw)
      unless uri.is_a?(URI::HTTP) && %w[http https].include?(uri.scheme)
        raise ArgumentError, "base_url must use http or https"
      end
      raise ArgumentError, "base_url must include a host" if uri.host.nil? || uri.host.empty?
      raise ArgumentError, "base_url must not include credentials" if uri.userinfo || authority_includes_userinfo?(raw)
      raise ArgumentError, "base_url must not include a query string" if uri.query
      raise ArgumentError, "base_url must not include a fragment" if uri.fragment
      if uri.scheme == "http" && !loopback_literal?(uri.hostname)
        raise ArgumentError, "http base_url is allowed only for 127.0.0.1 or [::1]"
      end

      validate_base_path(uri.path)
      raw.sub(%r{/+\z}, "")
    end

    def self.parse_base_url(raw)
      URI.parse(raw)
    rescue URI::InvalidURIError
      raise ArgumentError, "invalid base_url"
    end
    private_class_method :parse_base_url

    def self.authority_includes_userinfo?(raw)
      raw.match?(%r{\Ahttps?://[^/?#@]*@}i)
    end
    private_class_method :authority_includes_userinfo?

    def self.loopback_literal?(host)
      host == "127.0.0.1" || host == "::1"
    end
    private_class_method :loopback_literal?

    def self.validate_base_path(path)
      return if path.nil? || path.empty?

      if path.match?(/%(?:2f|5c)/i)
        raise ArgumentError, "base_url path must not include encoded path separators"
      end

      path.split("/", -1).each do |segment|
        decoded = URI::DEFAULT_PARSER.unescape(segment)
        if segment == "." || segment == ".." || decoded == "." || decoded == ".."
          raise ArgumentError, "base_url path must not include dot segments"
        end
      end
    end
    private_class_method :validate_base_path
  end
end
