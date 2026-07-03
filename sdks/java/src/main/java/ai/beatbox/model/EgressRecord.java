package ai.beatbox.model;

/** A record of network egress to a single domain/port during execution. */
public record EgressRecord(String domain, int port, long bytes) {
}
