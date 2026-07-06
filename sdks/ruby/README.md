# beatbox (Ruby SDK)

Idiomatic, **zero-dependency** Ruby client for the [beatbox](https://github.com/jadenfix/beatbox)
sandbox REST API. Standard library only (`net/http`, `uri`, `json`); Ruby 3.0+.

Part of the 7-language beatbox SDK fleet — same method names, config, and error
model across TypeScript, Python, Go, Java, Ruby, PHP, and C#.

## Install

From RubyGems (once published):

```ruby
# Gemfile
gem "beatbox", "~> 0.1"
```

```sh
gem install beatbox
```

Or build/install from this checkout:

```sh
cd sdks/ruby
gem build beatbox.gemspec
gem install ./beatbox-0.1.0.gem
```

You can also use it straight from source with `ruby -Ilib ...`.

## Quickstart

Run a `wasm_wat` "add one" module and check the value is 42:

```ruby
require "beatbox"

client = Beatbox::Client.new(
  base_url: "http://127.0.0.1:7300",
  api_key: ENV["BEATBOX_API_KEY"]
)

wat = <<~WAT
  (module
    (func (export "run") (param i64) (result i64)
      local.get 0
      i64.const 1
      i64.add))
WAT

result = client.execute(
  Beatbox::ExecuteRequest.wasm_wat(wat, entrypoint: "run", input: { "n" => 41 })
)

puts result.value          # => 42
raise "unexpected" unless result.value == 42
```

A runnable version is in [`examples/add_one.rb`](examples/add_one.rb):

```sh
BEATBOX_API_KEY=... ruby -Ilib examples/add_one.rb
```

## Configuration

```ruby
Beatbox::Client.new(
  base_url: "http://127.0.0.1:7300", # required; trailing slashes trimmed
  api_key: ENV["BEATBOX_API_KEY"],   # optional
  timeout: 65                        # optional, seconds (default 65)
)
```

When `api_key` is set it is sent as the `x-beatbox-api-key` header on every
request **except** `health` and `openapi`, which are unauthenticated. Redirects
are never followed, so the api key cannot leak to another origin.

## Methods

| Method | HTTP | Auth | Returns |
| --- | --- | --- | --- |
| `client.health` | `GET /v1/health` | no | `Hash` (raw JSON) |
| `client.capabilities` | `GET /v1/capabilities` | yes | `Hash` (raw JSON) |
| `client.browser_profiles` | `GET /v1/browser/profiles` | yes | `Hash` (raw JSON) |
| `client.browser_admit(request)` | `POST /v1/browser/admit` | yes | `Hash` (raw JSON) |
| `client.browser_adapter_contract` | `GET /v1/browser/adapter/contract` | yes | `Hash` (raw JSON) |
| `client.browser_adapter_capability(request)` | `POST /v1/browser/adapter/capability` | yes | `Hash` (raw JSON) |
| `client.browser_adapter_register(request)` | `POST /v1/browser/adapter/register` | yes | `Hash` (raw JSON) |
| `client.validate_browser_adapter(request)` | `POST /v1/browser/adapter/validate` | yes | `Hash` (raw JSON) |
| `client.execute(request)` | `POST /v1/execute` | yes | `Beatbox::ExecutionResult` |
| `client.create_job(request)` | `POST /v1/jobs` | yes | `Beatbox::CreateJobResponse` |
| `client.get_job(job_id)` | `GET /v1/jobs/{id}` | yes | `Beatbox::JobRecord` |
| `client.cancel_job(job_id)` | `DELETE /v1/jobs/{id}` | yes | `nil` |
| `client.openapi` | `GET /openapi.json` | no | `Hash` (raw JSON) |

`job_id` is percent-encoded as a single path segment; an empty id, `.`, or `..`
raises `ArgumentError` before any request is sent.

## Building requests

```ruby
# Ergonomic one-liners
Beatbox::ExecuteRequest.wasm_wat("(module ...)", input: { "n" => 41 })
Beatbox::ExecuteRequest.wasm_bytes_base64("AGFzbQ...", entrypoint: "run")

# Full control
Beatbox::ExecuteRequest.new(
  lane: Beatbox::Lane::WASM,
  source: Beatbox::Source.wasm_wat("(module ...)"),
  entrypoint: "run",
  input: { "n" => 41 },
  stdin: "",
  policy: Beatbox::Policy.new(limits: Beatbox::Limits.new(wall_ms: 5000)),
  idempotency_key: "step-1"
)
```

`Source` variants: `inline(code)`, `wasm_file(path)`, `wasm_wat(text)`,
`wasm_bytes_base64(bytes)`, `module_ref(sha256)`.

`Limits`/`Policy` are **partial** — only the keys you set are serialized and
merged onto the daemon defaults. Known `Limits` keys: `wall_ms`, `cpu_ms`,
`memory_bytes`, `disk_bytes`, `output_bytes`, `pids`, `fuel`.

## Results

`ExecutionResult` exposes `status`, `value`, `stdout`/`stderr` (+ `_truncated`),
`error`, `metrics`, `deterministic`, `inputs_digest`, `engine_version`,
`beatbox_version`, `effective_isolation`, `egress`, `exit_code`, plus `ok?`.
`Metrics#cpu_time_ms`, `#fuel_used`, and `#peak_memory_bytes` may be `nil`.
Unknown fields are preserved (`#raw` / `#to_h`) for forward-compatibility.

## Error handling

```ruby
begin
  client.execute(request)
rescue Beatbox::ApiError => e
  # non-2xx response
  e.status  # HTTP status (Integer)
  e.code    # error code from the body, e.g. "bad_source" (may be nil)
  e.message # human message
rescue Beatbox::TransportError => e
  # DNS/connect/TLS/timeout failure
  e.message
end
```

Both derive from `Beatbox::Error`. The api key is never included in any error
message.

## Development

```sh
ruby -Ilib -e "require 'beatbox'"          # smoke-load
ruby -Ilib -Itest test/beatbox_test.rb      # run unit tests
rake test                                   # same, via Rake
```

The unit tests (`test/beatbox_test.rb`) cover job-id encoding and request/result
JSON round-trips and need no live daemon.

## License

Apache-2.0.
