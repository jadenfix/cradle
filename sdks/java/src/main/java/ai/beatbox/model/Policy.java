package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.List;
import java.util.Map;

/**
 * Sandbox policy. Every field is optional; {@code null} fields are omitted and the daemon applies
 * its defaults. A partial {@link Limits} merges field-by-field onto the defaults.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record Policy(
        Limits limits,
        Map<String, String> env,
        Determinism determinism,
        NetPolicy net,
        FsPolicy fs,
        List<Secret> secrets,
        Boolean doubleJail) {

    /** An empty policy (all defaults). */
    public static Policy defaults() {
        return new Policy(null, null, null, null, null, null, null);
    }

    /** Convenience: a policy that only constrains limits. */
    public static Policy withLimits(Limits limits) {
        return new Policy(limits, null, null, null, null, null, null);
    }
}
