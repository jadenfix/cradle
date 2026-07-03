package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonInclude;

/**
 * Partial resource limits. Any {@code null} field is omitted on the wire and merged onto the
 * daemon's defaults.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record Limits(
        Long wallMs,
        Long cpuMs,
        Long memoryBytes,
        Long diskBytes,
        Long outputBytes,
        Long fuel,
        Integer pids) {

    /** Start from an empty set of limits and set fields with the {@code with*} helpers. */
    public static Limits none() {
        return new Limits(null, null, null, null, null, null, null);
    }

    public static Limits wallMs(long wallMs) {
        return none().withWallMs(wallMs);
    }

    public Limits withWallMs(long value) {
        return new Limits(value, cpuMs, memoryBytes, diskBytes, outputBytes, fuel, pids);
    }

    public Limits withCpuMs(long value) {
        return new Limits(wallMs, value, memoryBytes, diskBytes, outputBytes, fuel, pids);
    }

    public Limits withMemoryBytes(long value) {
        return new Limits(wallMs, cpuMs, value, diskBytes, outputBytes, fuel, pids);
    }

    public Limits withDiskBytes(long value) {
        return new Limits(wallMs, cpuMs, memoryBytes, value, outputBytes, fuel, pids);
    }

    public Limits withOutputBytes(long value) {
        return new Limits(wallMs, cpuMs, memoryBytes, diskBytes, value, fuel, pids);
    }

    public Limits withFuel(long value) {
        return new Limits(wallMs, cpuMs, memoryBytes, diskBytes, outputBytes, value, pids);
    }

    public Limits withPids(int value) {
        return new Limits(wallMs, cpuMs, memoryBytes, diskBytes, outputBytes, fuel, value);
    }
}
