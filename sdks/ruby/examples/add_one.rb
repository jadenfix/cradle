# frozen_string_literal: true

# Runs a tiny wasm_wat "add one" module against a live beatbox daemon and
# asserts the returned value is 42.
#
#   CRADLE_TOKEN=... ruby -Ilib examples/add_one.rb
#
# Set CRADLE_BASE_URL to override the default local daemon address.

require "beatbox"

base_url = ENV.fetch("CRADLE_BASE_URL", "http://127.0.0.1:7300")
token = ENV["CRADLE_TOKEN"]

client = Beatbox::Client.new(base_url: base_url, token: token)

wat = <<~WAT
  (module
    (func (export "run") (param i64) (result i64)
      local.get 0
      i64.const 1
      i64.add))
WAT

request = Beatbox::ExecuteRequest.wasm_wat(wat, entrypoint: "run", input: { "n" => 41 })

begin
  result = client.execute(request)
rescue Beatbox::ApiError => e
  warn "API error (status #{e.status}, code #{e.code}): #{e.message}"
  exit 1
rescue Beatbox::TransportError => e
  warn "Transport error: #{e.message}"
  exit 1
end

puts "status: #{result.status}"
puts "value:  #{result.value.inspect}"
puts "wall_time_ms: #{result.metrics&.wall_time_ms}"

raise "expected 42, got #{result.value.inspect}" unless result.value == 42

puts "OK: value == 42"
