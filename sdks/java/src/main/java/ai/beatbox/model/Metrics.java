package ai.beatbox.model;

/**
 * Execution metrics.
 *
 * <p>{@code cpuTimeMs}, {@code fuelUsed} and {@code peakMemoryBytes} are nullable: the W0 wasm lane
 * does not measure CPU time separately from wall time, so use {@code fuelUsed} as the deterministic
 * compute signal there rather than treating wall time as CPU.
 */
public record Metrics(
        long wallTimeMs,
        Long cpuTimeMs,
        Long fuelUsed,
        Long peakMemoryBytes) {
}
