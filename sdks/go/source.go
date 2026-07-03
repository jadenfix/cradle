package beatbox

import "encoding/base64"

// SourceInline builds an "inline" source from guest source code.
func SourceInline(code string) Source {
	return Source{Kind: SourceKindInline, Code: code}
}

// SourceWasmFile builds a "wasm_file" source referencing a path on the daemon host.
func SourceWasmFile(path string) Source {
	return Source{Kind: SourceKindWasmFile, Path: path}
}

// SourceWasmWat builds a "wasm_wat" source from WebAssembly text format.
func SourceWasmWat(text string) Source {
	return Source{Kind: SourceKindWasmWat, Text: text}
}

// SourceWasmBytesBase64 builds a "wasm_bytes_base64" source from raw wasm bytes,
// encoding them as standard base64 for the wire.
func SourceWasmBytesBase64(module []byte) Source {
	return Source{Kind: SourceKindWasmBytesBase64, Bytes: base64.StdEncoding.EncodeToString(module)}
}

// SourceModuleRef builds a "module_ref" source referencing a cached module by
// its sha256 digest.
func SourceModuleRef(sha256 string) Source {
	return Source{Kind: SourceKindModuleRef, SHA256: sha256}
}

// WasmWatRequest builds an ExecuteRequest for the wasm lane from WAT text and an
// optional input value. Pass input nil to omit it. This covers the common
// one-line case:
//
//	res, err := c.Execute(ctx, beatbox.WasmWatRequest(wat, map[string]any{"n": 41}))
func WasmWatRequest(text string, input any) ExecuteRequest {
	return ExecuteRequest{
		Lane:   LaneWasm,
		Source: SourceWasmWat(text),
		Input:  input,
	}
}
