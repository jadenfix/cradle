# frozen_string_literal: true

module Beatbox
  # Small internal helpers. Public only so the encoding rules can be unit tested.
  module Util
    module_function

    # RFC 3986 unreserved bytes: ALPHA / DIGIT / "-" / "." / "_" / "~".
    def unreserved?(byte)
      (byte >= 0x41 && byte <= 0x5A) || # A-Z
        (byte >= 0x61 && byte <= 0x7A) || # a-z
        (byte >= 0x30 && byte <= 0x39) || # 0-9
        byte == 0x2D || byte == 0x2E || byte == 0x5F || byte == 0x7E # - . _ ~
    end

    # Percent-encode +id+ as a single URI path segment.
    #
    # Every byte outside the RFC 3986 unreserved set is escaped, so a "/" in
    # the id becomes "%2F" and can never open a new path segment. An empty id,
    # or "." / ".." (which could retarget the request), is rejected.
    #
    # @raise [ArgumentError] if the id is empty, ".", or ".."
    def encode_path_segment(id)
      s = id.nil? ? "" : id.to_s
      raise ArgumentError, "job_id must not be empty" if s.empty?
      raise ArgumentError, "job_id must not be '.' or '..'" if s == "." || s == ".."

      s.b.bytes.map { |byte| unreserved?(byte) ? byte.chr : format("%%%02X", byte) }.join
    end
  end
end
