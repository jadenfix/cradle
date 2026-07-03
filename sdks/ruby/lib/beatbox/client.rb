# frozen_string_literal: true

require "net/http"
require "uri"
require "json"

module Beatbox
  # HTTP client for the beatbox sandbox REST API.
  #
  #   client = Beatbox::Client.new(base_url: "http://127.0.0.1:7300",
  #                                api_key: ENV["BEATBOX_API_KEY"])
  #   result = client.execute(Beatbox::ExecuteRequest.wasm_wat(wat, input: { "n" => 41 }))
  #   result.value # => 42
  #
  # Zero third-party dependencies: net/http + json from the standard library.
  class Client
    DEFAULT_TIMEOUT = 65
    API_KEY_HEADER = "x-beatbox-api-key"

    # @return [String] the normalized base url (trailing slashes trimmed)
    attr_reader :base_url
    # @return [Numeric] per-request timeout in seconds
    attr_reader :timeout

    # @param base_url [String] required, e.g. "http://127.0.0.1:7300"
    # @param api_key [String, nil] sent as x-beatbox-api-key on authed routes
    # @param timeout [Numeric] open/read/write timeout in seconds (default 65)
    def initialize(base_url:, api_key: nil, timeout: DEFAULT_TIMEOUT)
      raise ArgumentError, "base_url is required" if base_url.nil? || base_url.to_s.empty?

      @base_url = base_url.to_s.sub(%r{/+\z}, "")
      @api_key = api_key
      @timeout = timeout
    end

    # GET /v1/health (unauthenticated). Returns raw JSON as a Hash.
    def health
      request_json(:get, "/v1/health", auth: false)
    end

    # GET /v1/capabilities. Returns raw JSON as a Hash.
    def capabilities
      request_json(:get, "/v1/capabilities", auth: true)
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
      uri = URI.parse(@base_url + path)
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

    def build_http(uri)
      http = Net::HTTP.new(uri.host, uri.port)
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
      req[API_KEY_HEADER] = @api_key if auth && @api_key && !@api_key.empty?

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
    # 3xx is surfaced as an ApiError rather than replaying the api-key header to
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
  end
end
