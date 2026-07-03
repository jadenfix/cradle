package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonValue;

/** Filesystem mount access mode. */
public enum MountMode {
    RO("ro"),
    RW("rw");

    private final String wire;

    MountMode(String wire) {
        this.wire = wire;
    }

    @JsonValue
    public String wire() {
        return wire;
    }

    @JsonCreator
    public static MountMode fromWire(String value) {
        for (MountMode mode : values()) {
            if (mode.wire.equals(value)) {
                return mode;
            }
        }
        throw new IllegalArgumentException("Unknown mount mode: " + value);
    }
}
