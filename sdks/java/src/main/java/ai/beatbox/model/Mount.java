package ai.beatbox.model;

/** A host directory mounted into the sandbox at {@code guest}. */
public record Mount(String host, String guest, MountMode mode) {
}
