package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonValue;

/** Execution lane. Serializes to its snake_case wire value. */
public enum Lane {
    WASM("wasm"),
    PYTHON_WASI("python_wasi"),
    PYTHON_NATIVE("python_native"),
    JS_WASM("js_wasm"),
    JS_NATIVE("js_native"),
    EXEC("exec");

    private final String wire;

    Lane(String wire) {
        this.wire = wire;
    }

    @JsonValue
    public String wire() {
        return wire;
    }

    @JsonCreator
    public static Lane fromWire(String value) {
        for (Lane lane : values()) {
            if (lane.wire.equals(value)) {
                return lane;
            }
        }
        throw new IllegalArgumentException("Unknown lane: " + value);
    }
}
