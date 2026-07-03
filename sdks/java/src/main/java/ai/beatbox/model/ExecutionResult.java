package ai.beatbox.model;

import java.util.List;

/**
 * The result of a synchronous execution.
 *
 * <p>{@code value} is arbitrary JSON (object, number, string, {@code null}, ...); callers cast it as
 * needed. Unknown/extra fields are ignored so future server additions do not break deserialization.
 */
public record ExecutionResult(
        ExecutionStatus status,
        Object value,
        String stdout,
        boolean stdoutTruncated,
        String stderr,
        boolean stderrTruncated,
        Metrics metrics,
        Lane lane,
        boolean deterministic,
        String inputsDigest,
        String engineVersion,
        String beatboxVersion,
        EffectiveIsolation effectiveIsolation,
        List<EgressRecord> egress,
        ErrorBody error,
        Integer exitCode) {

    /** True when the execution completed successfully ({@link ExecutionStatus#OK}). */
    public boolean isOk() {
        return status == ExecutionStatus.OK;
    }
}
