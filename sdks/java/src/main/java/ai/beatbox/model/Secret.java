package ai.beatbox.model;

/** A secret to expose to the program, resolved by reference (never inlined here). */
public record Secret(String name, String valueRef, SecretExpose expose) {
}
