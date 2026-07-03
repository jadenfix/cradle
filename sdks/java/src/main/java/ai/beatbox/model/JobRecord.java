package ai.beatbox.model;

/** The full record of an asynchronous job, including its request and (once finished) its result. */
public record JobRecord(
        String jobId,
        JobStatus status,
        ExecuteRequest request,
        ExecutionResult result,
        ErrorBody error,
        String createdAt,
        String updatedAt) {
}
