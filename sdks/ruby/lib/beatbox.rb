# frozen_string_literal: true

# Beatbox — zero-dependency Ruby SDK for the beatbox sandbox REST API.
#
# Requires only the standard library (net/http, uri, json). See {Beatbox::Client}.
module Beatbox
end

require_relative "beatbox/version"
require_relative "beatbox/errors"
require_relative "beatbox/util"
require_relative "beatbox/models"
require_relative "beatbox/client"
