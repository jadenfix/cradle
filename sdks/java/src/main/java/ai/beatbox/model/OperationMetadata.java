package ai.beatbox.model;

/** Progress metadata for a long-running operation. */
public record OperationMetadata(
        String targetResource,
        String createTime,
        String currentStage,
        Double progressRatio) {
}
