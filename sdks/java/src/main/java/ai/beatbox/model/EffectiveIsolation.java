package ai.beatbox.model;

import java.util.List;

/** The isolation mechanisms actually applied by the host, and any downgrades from the request. */
public record EffectiveIsolation(
        String os,
        List<String> mechanisms,
        List<String> downgrades,
        Integer landlockAbi) {
}
