package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonSubTypes;
import com.fasterxml.jackson.annotation.JsonTypeInfo;

/** Determinism policy, a tagged union on {@code kind}. */
@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.PROPERTY, property = "kind")
@JsonSubTypes({
        @JsonSubTypes.Type(value = Determinism.Off.class, name = "off"),
        @JsonSubTypes.Type(value = Determinism.Seeded.class, name = "seeded"),
})
public sealed interface Determinism permits Determinism.Off, Determinism.Seeded {

    /** Determinism disabled. */
    record Off() implements Determinism {
    }

    /** Deterministic execution seeded with {@code seed} and a fixed clock epoch. */
    record Seeded(long seed, long epochMs) implements Determinism {
    }

    static Determinism off() {
        return new Off();
    }

    static Determinism seeded(long seed, long epochMs) {
        return new Seeded(seed, epochMs);
    }
}
