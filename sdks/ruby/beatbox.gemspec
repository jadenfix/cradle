# frozen_string_literal: true

require_relative "lib/beatbox/version"

Gem::Specification.new do |spec|
  spec.name = "beatbox"
  spec.version = Beatbox::VERSION
  spec.authors = ["Beatbox"]
  spec.summary = "Ruby SDK for the beatbox sandbox REST API"
  spec.description = "Zero-dependency, idiomatic Ruby client for the beatbox " \
                     "sandbox daemon: execute wasm/python/js/exec workloads, " \
                     "manage async jobs, and read capabilities."
  spec.homepage = "https://github.com/jadenfix/beatbox"
  spec.license = "Apache-2.0"
  spec.required_ruby_version = ">= 3.0"

  spec.metadata = {
    "homepage_uri" => spec.homepage,
    "source_code_uri" => "#{spec.homepage}/tree/main/sdks/ruby",
    "rubygems_mfa_required" => "true"
  }

  spec.files = Dir[
    "lib/**/*.rb",
    "README.md",
    "LICENSE*",
    "beatbox.gemspec"
  ]
  spec.require_paths = ["lib"]

  # Zero runtime dependencies: standard library only (net/http, uri, json).
end
