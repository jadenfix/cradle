package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonValue;

/** Terminal status of a synchronous execution. */
public enum ExecutionStatus {
    OK("ok"),
    ERROR("error"),
    TIMEOUT("timeout"),
    OOM("oom"),
    KILLED("killed"),
    DENIED("denied");

    private final String wire;

    ExecutionStatus(String wire) {
        this.wire = wire;
    }

    @JsonValue
    public String wire() {
        return wire;
    }

    @JsonCreator
    public static ExecutionStatus fromWire(String value) {
        for (ExecutionStatus status : values()) {
            if (status.wire.equals(value)) {
                return status;
            }
        }
        throw new IllegalArgumentException("Unknown execution status: " + value);
    }
}
