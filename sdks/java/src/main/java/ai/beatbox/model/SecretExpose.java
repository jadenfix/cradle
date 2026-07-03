package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonValue;

/** How a secret is exposed to the sandboxed program. */
public enum SecretExpose {
    ENV("env"),
    FILE("file");

    private final String wire;

    SecretExpose(String wire) {
        this.wire = wire;
    }

    @JsonValue
    public String wire() {
        return wire;
    }

    @JsonCreator
    public static SecretExpose fromWire(String value) {
        for (SecretExpose expose : values()) {
            if (expose.wire.equals(value)) {
                return expose;
            }
        }
        throw new IllegalArgumentException("Unknown secret expose: " + value);
    }
}
