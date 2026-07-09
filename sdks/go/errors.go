package beatbox

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
)

// APIError is returned when the daemon responds with a non-2xx status. It
// carries the HTTP status and, when the response body follows the standard
// {"error": {...}} envelope, the code and message from that body.
type APIError struct {
	// Status is the HTTP status code.
	Status int
	// Code is the machine-readable error code from the response body, if any.
	Code string
	// Message is the human-readable error message.
	Message string
}

// Error implements the error interface. It never includes auth material.
func (e *APIError) Error() string {
	if e.Code != "" {
		return fmt.Sprintf("beatbox: api error %d (%s): %s", e.Status, e.Code, e.Message)
	}
	return fmt.Sprintf("beatbox: api error %d: %s", e.Status, e.Message)
}

// parseAPIError builds an *APIError from a non-2xx response body, tolerating
// bodies that are the standard envelope, a bare ErrorBody, or opaque text.
func parseAPIError(status int, body []byte) *APIError {
	e := &APIError{Status: status}

	var env errorResponse
	if err := json.Unmarshal(body, &env); err == nil && (env.Error.Code != "" || env.Error.Message != "") {
		e.Code = env.Error.Code
		e.Message = env.Error.Message
	} else {
		var eb ErrorBody
		if err := json.Unmarshal(body, &eb); err == nil && (eb.Code != "" || eb.Message != "") {
			e.Code = eb.Code
			e.Message = eb.Message
		} else if txt := strings.TrimSpace(string(body)); txt != "" {
			e.Message = txt
		}
	}

	if e.Message == "" {
		e.Message = http.StatusText(status)
	}
	return e
}
