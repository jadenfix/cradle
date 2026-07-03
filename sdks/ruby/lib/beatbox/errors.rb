# frozen_string_literal: true

module Beatbox
  # Base class for every error raised by the SDK.
  class Error < StandardError; end

  # Raised when the daemon returns a non-2xx HTTP response.
  #
  # Carries the HTTP +status+, the machine-readable +code+ taken from the
  # +{ "error": { "code", "message" } }+ body (when present), and a human
  # +message+. The api key is never included in the message.
  class ApiError < Error
    # @return [Integer] the HTTP status code
    attr_reader :status
    # @return [String, nil] the error code from the response body, if any
    attr_reader :code

    def initialize(status:, code: nil, message: nil)
      @status = status
      @code = code
      super(message && !message.empty? ? message : "beatbox API error (HTTP #{status})")
    end
  end

  # Raised on a transport-level failure (DNS, connect, TLS, timeout, reset).
  # The api key is never included in the message.
  class TransportError < Error; end
end
