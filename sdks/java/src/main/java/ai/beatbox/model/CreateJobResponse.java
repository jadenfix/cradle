package ai.beatbox.model;

/** Response to {@code createJob}: the id of the newly created asynchronous job. */
public record CreateJobResponse(String jobId) {
}
