package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonSubTypes;
import com.fasterxml.jackson.annotation.JsonTypeInfo;
import java.util.List;

/** Network policy, a tagged union on {@code kind}. */
@JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.PROPERTY, property = "kind")
@JsonSubTypes({
        @JsonSubTypes.Type(value = NetPolicy.Deny.class, name = "deny"),
        @JsonSubTypes.Type(value = NetPolicy.Proxy.class, name = "proxy"),
})
public sealed interface NetPolicy permits NetPolicy.Deny, NetPolicy.Proxy {

    /** All network egress denied. */
    record Deny() implements NetPolicy {
    }

    /** Egress allowed to the listed domains and ports through the daemon proxy. */
    @JsonInclude(JsonInclude.Include.NON_NULL)
    record Proxy(List<String> allowDomains, List<Integer> allowPorts) implements NetPolicy {
    }

    static NetPolicy deny() {
        return new Deny();
    }

    static NetPolicy proxy(List<String> allowDomains, List<Integer> allowPorts) {
        return new Proxy(allowDomains, allowPorts);
    }
}
