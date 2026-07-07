# frozen_string_literal: true

require "minitest/autorun"
require "json"
require "socket"
require "beatbox"

class JobIdEncodingTest < Minitest::Test
  def test_path_traversal_slash_is_percent_encoded
    # "/" must never open a new path segment.
    assert_equal "..%2Fexecute", Beatbox::Util.encode_path_segment("../execute")
  end

  def test_query_delimiters_are_encoded
    assert_equal "x%3Fk%3Dv", Beatbox::Util.encode_path_segment("x?k=v")
  end

  def test_uuid_is_left_intact
    uuid = "3f8c9b2a-1d4e-4c6f-9a0b-1122334455ff"
    assert_equal uuid, Beatbox::Util.encode_path_segment(uuid)
  end

  def test_unreserved_characters_pass_through
    assert_equal "a-b_c.d~e", Beatbox::Util.encode_path_segment("a-b_c.d~e")
  end

  def test_space_and_unicode_are_encoded
    assert_equal "a%20b", Beatbox::Util.encode_path_segment("a b")
    assert_equal "%C3%A9", Beatbox::Util.encode_path_segment("é")
  end

  def test_empty_id_is_rejected
    assert_raises(ArgumentError) { Beatbox::Util.encode_path_segment("") }
  end

  def test_dot_id_is_rejected
    assert_raises(ArgumentError) { Beatbox::Util.encode_path_segment(".") }
  end

  def test_dotdot_id_is_rejected
    assert_raises(ArgumentError) { Beatbox::Util.encode_path_segment("..") }
  end

  def test_nil_id_is_rejected
    assert_raises(ArgumentError) { Beatbox::Util.encode_path_segment(nil) }
  end
end

class RequestSerializationTest < Minitest::Test
  def test_wasm_wat_request_round_trips_through_json
    req = Beatbox::ExecuteRequest.wasm_wat(
      "(module)",
      entrypoint: "run",
      input: { "n" => 41 },
      policy: Beatbox::Policy.new(limits: Beatbox::Limits.new(wall_ms: 5000)),
      idempotency_key: "step-1"
    )

    wire = JSON.parse(JSON.generate(req.to_h))

    expected = {
      "lane" => "wasm",
      "source" => { "kind" => "wasm_wat", "text" => "(module)" },
      "entrypoint" => "run",
      "input" => { "n" => 41 },
      "policy" => { "limits" => { "wall_ms" => 5000 } },
      "idempotency_key" => "step-1"
    }
    assert_equal expected, wire

    # Parse back into a model and re-serialize: stable round-trip.
    reparsed = Beatbox::ExecuteRequest.from_h(wire)
    assert_equal wire, reparsed.to_h
    assert_equal "wasm", reparsed.lane
    assert_equal "wasm_wat", reparsed.source.kind
    assert_equal 5000, reparsed.policy.limits[:wall_ms]
  end

  def test_limits_only_serializes_provided_keys
    limits = Beatbox::Limits.new(wall_ms: 1000, memory_bytes: 2048)
    assert_equal({ "wall_ms" => 1000, "memory_bytes" => 2048 }, limits.to_h)
  end

  def test_optional_fields_are_omitted_when_absent
    req = Beatbox::ExecuteRequest.wasm_wat("(module)")
    assert_equal(
      { "lane" => "wasm", "source" => { "kind" => "wasm_wat", "text" => "(module)" } },
      req.to_h
    )
  end

  def test_explicit_null_input_is_sent
    req = Beatbox::ExecuteRequest.wasm_wat("(module)", input: nil)
    assert req.to_h.key?("input")
    assert_nil req.to_h["input"]
  end

  def test_all_source_constructors
    assert_equal({ "kind" => "inline", "code" => "x" }, Beatbox::Source.inline("x").to_h)
    assert_equal({ "kind" => "wasm_file", "path" => "/m" }, Beatbox::Source.wasm_file("/m").to_h)
    assert_equal({ "kind" => "wasm_wat", "text" => "t" }, Beatbox::Source.wasm_wat("t").to_h)
    assert_equal({ "kind" => "wasm_bytes_base64", "bytes" => "b" },
                 Beatbox::Source.wasm_bytes_base64("b").to_h)
    assert_equal({ "kind" => "module_ref", "sha256" => "s" },
                 Beatbox::Source.module_ref("s").to_h)
  end
end

class ResultDeserializationTest < Minitest::Test
  def fixture
    {
      "status" => "ok",
      "value" => 42,
      "stdout" => "",
      "stdout_truncated" => false,
      "stderr" => "",
      "stderr_truncated" => false,
      "metrics" => {
        "wall_time_ms" => 3,
        "cpu_time_ms" => nil,
        "fuel_used" => 1234,
        "peak_memory_bytes" => nil
      },
      "lane" => "wasm",
      "deterministic" => true,
      "inputs_digest" => "sha256:abc",
      "engine_version" => "w0-1",
      "beatbox_version" => "0.1.0",
      "effective_isolation" => { "os" => "linux", "mechanisms" => [], "downgrades" => [] },
      "egress" => [],
      "error" => nil,
      "exit_code" => nil,
      "future_unknown_field" => { "anything" => true }
    }
  end

  def test_parses_typed_fields
    result = Beatbox::ExecutionResult.from_h(fixture)

    assert result.ok?
    assert_equal 42, result.value
    assert_equal "wasm", result.lane
    assert_equal 3, result.metrics.wall_time_ms
    assert_nil result.metrics.cpu_time_ms
    assert_equal 1234, result.metrics.fuel_used
    assert_nil result.metrics.peak_memory_bytes
    assert_nil result.error
  end

  def test_unknown_fields_survive_round_trip
    result = Beatbox::ExecutionResult.from_h(fixture)
    round_tripped = JSON.parse(JSON.generate(result.to_h))
    assert_equal fixture, round_tripped
    assert_equal({ "anything" => true }, round_tripped["future_unknown_field"])
  end

  def test_error_body_is_parsed_when_present
    body = fixture.merge(
      "status" => "error",
      "error" => { "code" => "trap", "message" => "unreachable" }
    )
    result = Beatbox::ExecutionResult.from_h(body)
    refute result.ok?
    assert_equal "trap", result.error.code
    assert_equal "unreachable", result.error.message
  end

  def test_job_record_parses_nested_request_and_result
    job = Beatbox::JobRecord.from_h(
      "job_id" => "abc",
      "status" => "succeeded",
      "request" => {
        "lane" => "wasm",
        "source" => { "kind" => "wasm_wat", "text" => "(module)" }
      },
      "result" => fixture,
      "created_at" => "2026-07-03T00:00:00Z",
      "updated_at" => "2026-07-03T00:00:01Z"
    )

    assert_equal "abc", job.job_id
    assert_equal "succeeded", job.status
    assert_equal "wasm", job.request.lane
    assert_equal 42, job.result.value
  end
end

class ErrorModelTest < Minitest::Test
  def test_api_error_exposes_status_code_message
    err = Beatbox::ApiError.new(status: 422, code: "bad_source", message: "nope")
    assert_equal 422, err.status
    assert_equal "bad_source", err.code
    assert_equal "nope", err.message
    assert_kind_of Beatbox::Error, err
  end

  def test_api_error_default_message_hides_nothing_sensitive
    err = Beatbox::ApiError.new(status: 500)
    assert_match(/HTTP 500/, err.message)
  end

  def test_client_requires_base_url
    assert_raises(ArgumentError) { Beatbox::Client.new(base_url: "") }
  end

  def test_client_trims_trailing_slashes_from_base_url
    client = Beatbox::Client.new(base_url: "https://api.example.test///")
    assert_equal "https://api.example.test", client.base_url
  end
end

class BaseUrlPolicyTest < Minitest::Test
  def test_accepts_https_base_url
    client = Beatbox::Client.new(base_url: "https://api.example.test/beatbox")
    assert_equal "https://api.example.test/beatbox", client.base_url
  end

  def test_accepts_plaintext_loopback_literals
    assert_equal "http://127.0.0.1:7300",
                 Beatbox::Client.new(base_url: "http://127.0.0.1:7300").base_url
    assert_equal "http://[::1]:7300",
                 Beatbox::Client.new(base_url: "http://[::1]:7300").base_url
  end

  def test_rejects_unsafe_base_urls
    rejected = [
      " http://127.0.0.1:7300",
      "https://api.example.test ",
      "api.example.test",
      "ftp://api.example.test",
      "https://@api.example.test",
      "https://user:pass@api.example.test",
      "https://api.example.test/path?token=one",
      "https://api.example.test/path#frag",
      "http://localhost:7300",
      "http://api.example.test",
      "https://api.example.test/./v1",
      "https://api.example.test/../v1",
      "https://api.example.test/%2e/v1",
      "https://api.example.test/%2e%2e/v1",
      "https://api.example.test/api%2Fv1",
      "https://api.example.test/api%5Cv1",
      "https://api.example.test/api\\v1"
    ]

    rejected.each do |base_url|
      assert_raises(ArgumentError, "expected #{base_url.inspect} to be rejected") do
        Beatbox::Client.new(base_url: base_url)
      end
    end
  end

  def test_http_client_disables_environment_proxies
    with_env("http_proxy" => "http://127.0.0.1:9", "https_proxy" => "http://127.0.0.1:9",
             "no_proxy" => nil, "NO_PROXY" => nil) do
      client = Beatbox::Client.new(base_url: "https://api.example.test")
      http = client.send(:build_http, URI.parse("https://api.example.test/v1/capabilities"))

      refute http.proxy?
    end
  end

  def test_public_job_request_preserves_valid_escaped_base_prefix_and_job_segment
    body = JSON.generate(
      "job_id" => "a/b",
      "status" => "queued",
      "request" => { "lane" => "wasm", "source" => { "kind" => "wasm_wat", "text" => "(module)" } },
      "created_at" => "2026-07-03T00:00:00Z",
      "updated_at" => "2026-07-03T00:00:01Z"
    )
    server = OneShotHttpServer.new(
      status: 200,
      headers: { "Content-Type" => "application/json" },
      body: body
    )

    client = Beatbox::Client.new(base_url: "http://127.0.0.1:#{server.port}/root%7E", api_key: "test-key")
    job = client.get_job("a/b")

    assert_equal "a/b", job.job_id
    assert_includes server.request, "GET /root%7E/v1/jobs/a%2Fb HTTP/1.1"
    assert server.request.lines.any? { |line| line.chomp.casecmp("X-Beatbox-Api-Key: test-key").zero? }
  ensure
    server&.close
  end

  def test_redirect_response_is_not_followed
    server = OneShotHttpServer.new(status: 302, headers: { "Location" => "/leaked" }, body: "")

    client = Beatbox::Client.new(base_url: "http://127.0.0.1:#{server.port}", api_key: "test-key")
    error = assert_raises(Beatbox::ApiError) { client.capabilities }

    assert_equal 302, error.status
    assert_includes server.request, "GET /v1/capabilities HTTP/1.1"
  ensure
    server&.close
  end

  private

  def with_env(overrides)
    old_values = overrides.each_key.to_h { |key| [key, ENV[key]] }
    overrides.each do |key, value|
      value.nil? ? ENV.delete(key) : ENV[key] = value
    end
    yield
  ensure
    old_values.each do |key, value|
      value.nil? ? ENV.delete(key) : ENV[key] = value
    end
  end

  class OneShotHttpServer
    attr_reader :port, :request

    def initialize(status:, headers:, body:)
      @server = TCPServer.new("127.0.0.1", 0)
      @port = @server.addr[1]
      @request = ""
      @thread = Thread.new { serve(status, headers, body) }
    end

    def close
      @server.close unless @server.closed?
      @thread.join
    end

    private

    def serve(status, headers, body)
      socket = @server.accept
      @request = read_headers(socket)
      response_headers = headers.merge(
        "Content-Length" => body.bytesize.to_s,
        "Connection" => "close"
      )
      socket.write("HTTP/1.1 #{status} #{reason(status)}\r\n")
      response_headers.each { |name, value| socket.write("#{name}: #{value}\r\n") }
      socket.write("\r\n")
      socket.write(body)
    rescue IOError
      nil
    ensure
      socket&.close
    end

    def read_headers(socket)
      buffer = +""
      buffer << socket.readpartial(4096) until buffer.include?("\r\n\r\n")
      buffer
    end

    def reason(status)
      status == 200 ? "OK" : "Found"
    end
  end
end
