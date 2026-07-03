# frozen_string_literal: true

module Beatbox
  # Sentinel distinguishing "argument not provided" from an explicit +nil+
  # (JSON null), so a caller can send `input: nil` on the wire when they mean it.
  UNSET = Object.new
  def UNSET.inspect = "Beatbox::UNSET"
  UNSET.freeze

  # Execution lane names (see openapi Lane enum).
  module Lane
    WASM = "wasm"
    PYTHON_WASI = "python_wasi"
    PYTHON_NATIVE = "python_native"
    JS_WASM = "js_wasm"
    JS_NATIVE = "js_native"
    EXEC = "exec"
    ALL = [WASM, PYTHON_WASI, PYTHON_NATIVE, JS_WASM, JS_NATIVE, EXEC].freeze
  end

  # Terminal status of a synchronous execution (see ExecutionStatus enum).
  module ExecutionStatus
    OK = "ok"
    ERROR = "error"
    TIMEOUT = "timeout"
    OOM = "oom"
    KILLED = "killed"
    DENIED = "denied"
  end

  # Lifecycle status of an asynchronous job (see JobStatus enum).
  module JobStatus
    QUEUED = "queued"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELED = "canceled"
  end

  # A program source. Tagged union on +kind+; use the named constructors.
  #
  #   Beatbox::Source.wasm_wat("(module ...)")
  #   Beatbox::Source.wasm_bytes_base64("AGFzbQ...")
  #   Beatbox::Source.inline("print('hi')")
  #   Beatbox::Source.wasm_file("/path/to/mod.wasm")
  #   Beatbox::Source.module_ref("sha256:...")
  class Source
    # @return [String] the union discriminant, e.g. "wasm_wat"
    attr_reader :kind
    # @return [Hash{String=>Object}] the variant payload fields (without :kind)
    attr_reader :fields

    def initialize(kind, fields = {})
      @kind = kind
      @fields = fields
    end

    def self.inline(code) = new("inline", { "code" => code })
    def self.wasm_file(path) = new("wasm_file", { "path" => path })
    def self.wasm_wat(text) = new("wasm_wat", { "text" => text })
    def self.wasm_bytes_base64(bytes) = new("wasm_bytes_base64", { "bytes" => bytes })
    def self.module_ref(sha256) = new("module_ref", { "sha256" => sha256 })

    def to_h
      { "kind" => kind }.merge(fields)
    end

    # Tolerates unknown extra fields (forward-compat).
    def self.from_h(hash)
      return nil if hash.nil?

      k = hash["kind"]
      rest = hash.reject { |key, _| key == "kind" }
      new(k, rest)
    end
  end

  # Resource limits. Partial: only the keys you set are serialized and merged
  # onto the daemon defaults. Known keys: wall_ms, cpu_ms, memory_bytes,
  # disk_bytes, output_bytes, pids, fuel.
  class Limits
    def initialize(**fields)
      @values = {}
      fields.each { |k, v| @values[k.to_s] = v unless v.nil? }
    end

    # @return [Object, nil] value for a known key (String or Symbol)
    def [](key) = @values[key.to_s]

    def to_h = @values.dup

    def self.from_h(hash)
      return nil if hash.nil?

      obj = new
      hash.each { |k, v| obj[k] = v }
      obj
    end

    def []=(key, value)
      @values[key.to_s] = value
    end
  end

  # Execution policy. Partial: only provided keys are serialized. +limits+ is
  # modelled explicitly; other policy sections (determinism, net, fs, env,
  # secrets, double_jail) pass through as-is for forward-compat.
  class Policy
    # @return [Limits, nil]
    attr_reader :limits

    def initialize(limits: nil, **extra)
      @limits = limits.is_a?(Hash) ? Limits.from_h(limits) : limits
      @extra = {}
      extra.each { |k, v| @extra[k.to_s] = v unless v.nil? }
    end

    def to_h
      h = {}
      h["limits"] = @limits.to_h if @limits
      @extra.each { |k, v| h[k] = v }
      h
    end

    def self.from_h(hash)
      return nil if hash.nil?

      limits = hash.key?("limits") ? Limits.from_h(hash["limits"]) : nil
      extra = hash.reject { |k, _| k == "limits" }
      new(limits: limits, **extra.transform_keys(&:to_sym))
    end
  end

  # Body of an execute / job request.
  class ExecuteRequest
    # @return [String] lane name (see {Lane})
    attr_accessor :lane
    # @return [Source]
    attr_accessor :source
    # @return [String, nil]
    attr_accessor :entrypoint
    # @return [Object] arbitrary JSON input, or {Beatbox::UNSET} when omitted
    attr_accessor :input
    # @return [String, nil]
    attr_accessor :stdin
    # @return [Policy, nil]
    attr_accessor :policy
    # @return [String, nil]
    attr_accessor :idempotency_key

    def initialize(lane:, source:, entrypoint: nil, input: UNSET, stdin: nil,
                   policy: nil, idempotency_key: nil)
      @lane = lane
      @source = source
      @entrypoint = entrypoint
      @input = input
      @stdin = stdin
      @policy = policy.is_a?(Hash) ? Policy.from_h(policy) : policy
      @idempotency_key = idempotency_key
    end

    # Ergonomic one-liner for the common wasm-text case.
    #
    #   Beatbox::ExecuteRequest.wasm_wat("(module ...)", input: { "n" => 41 })
    def self.wasm_wat(text, lane: Lane::WASM, **opts)
      new(lane: lane, source: Source.wasm_wat(text), **opts)
    end

    def self.wasm_bytes_base64(bytes, lane: Lane::WASM, **opts)
      new(lane: lane, source: Source.wasm_bytes_base64(bytes), **opts)
    end

    def self.inline(code, lane:, **opts)
      new(lane: lane, source: Source.inline(code), **opts)
    end

    def to_h
      h = { "lane" => lane, "source" => source_to_h }
      h["entrypoint"] = entrypoint unless entrypoint.nil?
      h["input"] = input unless input.equal?(UNSET)
      h["stdin"] = stdin unless stdin.nil?
      h["policy"] = @policy.to_h unless @policy.nil?
      h["idempotency_key"] = idempotency_key unless idempotency_key.nil?
      h
    end

    def self.from_h(hash)
      return nil if hash.nil?

      new(
        lane: hash["lane"],
        source: Source.from_h(hash["source"]),
        entrypoint: hash["entrypoint"],
        input: hash.key?("input") ? hash["input"] : UNSET,
        stdin: hash["stdin"],
        policy: Policy.from_h(hash["policy"]),
        idempotency_key: hash["idempotency_key"]
      )
    end

    private

    def source_to_h
      source.respond_to?(:to_h) ? source.to_h : source
    end
  end

  # A {code, message} error object as it appears inside result/job bodies.
  class ErrorBody
    attr_reader :code, :message

    def initialize(code:, message:)
      @code = code
      @message = message
    end

    def to_h = { "code" => code, "message" => message }

    def self.from_h(hash)
      return nil if hash.nil?

      new(code: hash["code"], message: hash["message"])
    end
  end

  # Execution metrics. +cpu_time_ms+, +fuel_used+, and +peak_memory_bytes+ may
  # be nil (e.g. the W0 wasm lane reports no separate CPU time).
  class Metrics
    attr_reader :wall_time_ms, :cpu_time_ms, :fuel_used, :peak_memory_bytes

    def initialize(wall_time_ms:, cpu_time_ms: nil, fuel_used: nil, peak_memory_bytes: nil)
      @wall_time_ms = wall_time_ms
      @cpu_time_ms = cpu_time_ms
      @fuel_used = fuel_used
      @peak_memory_bytes = peak_memory_bytes
    end

    def to_h
      {
        "wall_time_ms" => wall_time_ms,
        "cpu_time_ms" => cpu_time_ms,
        "fuel_used" => fuel_used,
        "peak_memory_bytes" => peak_memory_bytes
      }
    end

    def self.from_h(hash)
      return nil if hash.nil?

      new(
        wall_time_ms: hash["wall_time_ms"],
        cpu_time_ms: hash["cpu_time_ms"],
        fuel_used: hash["fuel_used"],
        peak_memory_bytes: hash["peak_memory_bytes"]
      )
    end
  end

  # Result of a synchronous execution. Typed accessors cover the documented
  # fields; +raw+ preserves the full body so unknown fields survive a
  # round-trip (forward-compat).
  class ExecutionResult
    attr_reader :status, :value, :stdout, :stdout_truncated, :stderr,
                :stderr_truncated, :metrics, :lane, :deterministic,
                :inputs_digest, :engine_version, :beatbox_version,
                :effective_isolation, :egress, :error, :exit_code, :raw

    def initialize(fields)
      @raw = fields
      @status = fields["status"]
      @value = fields["value"]
      @stdout = fields["stdout"]
      @stdout_truncated = fields["stdout_truncated"]
      @stderr = fields["stderr"]
      @stderr_truncated = fields["stderr_truncated"]
      @metrics = Metrics.from_h(fields["metrics"])
      @lane = fields["lane"]
      @deterministic = fields["deterministic"]
      @inputs_digest = fields["inputs_digest"]
      @engine_version = fields["engine_version"]
      @beatbox_version = fields["beatbox_version"]
      @effective_isolation = fields["effective_isolation"]
      @egress = fields["egress"]
      @error = ErrorBody.from_h(fields["error"])
      @exit_code = fields["exit_code"]
    end

    def ok? = status == ExecutionStatus::OK

    # Lossless: returns the original body, so unknown fields are preserved.
    def to_h = @raw.dup

    def self.from_h(hash)
      return nil if hash.nil?

      new(hash)
    end
  end

  # Response to POST /v1/jobs (202).
  class CreateJobResponse
    attr_reader :job_id, :raw

    def initialize(fields)
      @raw = fields
      @job_id = fields["job_id"]
    end

    def to_h = @raw.dup

    def self.from_h(hash)
      return nil if hash.nil?

      new(hash)
    end
  end

  # An asynchronous job record.
  class JobRecord
    attr_reader :job_id, :status, :request, :result, :error,
                :created_at, :updated_at, :raw

    def initialize(fields)
      @raw = fields
      @job_id = fields["job_id"]
      @status = fields["status"]
      @request = ExecuteRequest.from_h(fields["request"])
      @result = ExecutionResult.from_h(fields["result"])
      @error = ErrorBody.from_h(fields["error"])
      @created_at = fields["created_at"]
      @updated_at = fields["updated_at"]
    end

    def to_h = @raw.dup

    def self.from_h(hash)
      return nil if hash.nil?

      new(hash)
    end
  end
end
