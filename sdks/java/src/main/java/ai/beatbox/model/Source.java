package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonSubTypes;
import com.fasterxml.jackson.annotation.JsonTypeInfo;

/**
 * The program to run, a tagged union on the {@code kind} discriminator.
 *
 * <p>Use the static factory methods for the common cases, e.g.
 * {@code Source.wasmWat("(module ...)")}. The remote API only accepts {@code wasm_wat} and
 * {@code wasm_bytes_base64} for the {@code wasm} lane.
 */
@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.PROPERTY, property = "kind")
@JsonSubTypes({
        @JsonSubTypes.Type(value = Source.Inline.class, name = "inline"),
        @JsonSubTypes.Type(value = Source.WasmFile.class, name = "wasm_file"),
        @JsonSubTypes.Type(value = Source.WasmWat.class, name = "wasm_wat"),
        @JsonSubTypes.Type(value = Source.WasmBytesBase64.class, name = "wasm_bytes_base64"),
        @JsonSubTypes.Type(value = Source.ModuleRef.class, name = "module_ref"),
})
public sealed interface Source
        permits Source.Inline, Source.WasmFile, Source.WasmWat, Source.WasmBytesBase64, Source.ModuleRef {

    /** Inline source code interpreted by the selected lane. */
    record Inline(String code) implements Source {
    }

    /** A wasm module already present on the daemon host at {@code path}. */
    record WasmFile(String path) implements Source {
    }

    /** WebAssembly text format, compiled by the daemon. */
    record WasmWat(String text) implements Source {
    }

    /** A base64-encoded wasm binary. */
    record WasmBytesBase64(String bytes) implements Source {
    }

    /** A reference to a previously uploaded module by its sha256 digest. */
    record ModuleRef(String sha256) implements Source {
    }

    static Source inline(String code) {
        return new Inline(code);
    }

    static Source wasmFile(String path) {
        return new WasmFile(path);
    }

    static Source wasmWat(String text) {
        return new WasmWat(text);
    }

    static Source wasmBytesBase64(String bytes) {
        return new WasmBytesBase64(bytes);
    }

    static Source moduleRef(String sha256) {
        return new ModuleRef(sha256);
    }
}
