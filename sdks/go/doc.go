// Package beatbox is a zero-dependency Go SDK for the beatbox sandbox REST API.
//
// It talks to a running beatbox daemon (default base URL
// http://127.0.0.1:7300) using only the standard library (net/http and
// encoding/json). Construct a [Client] with [New] and functional options,
// then call the context-first methods that mirror the daemon's v1 surface:
//
//	c := beatbox.New("http://127.0.0.1:7300", beatbox.WithToken(token))
//	res, err := c.Execute(ctx, beatbox.WasmWatRequest(
//	    `(module (func (export "run") (param i64) (result i64)
//	        local.get 0 i64.const 1 i64.add))`,
//	    map[string]any{"n": 41}))
//	// res.Value decodes to 42
//
// All non-2xx responses surface as a typed [*APIError]; transport failures are
// wrapped errors. The token is never included in any URL or error message.
package beatbox
