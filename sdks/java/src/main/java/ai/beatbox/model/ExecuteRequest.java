package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonInclude;

/**
 * A request to run a program, used by both {@code execute} and {@code createJob}.
 *
 * <p>Only {@code lane} and {@code source} are required. Use the {@code wasmWat} factories for the
 * common one-liner, or {@link #builder(Lane, Source)} for full control. Null fields are omitted.
 */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record ExecuteRequest(
        Lane lane,
        Source source,
        String entrypoint,
        Object input,
        String stdin,
        Policy policy,
        String idempotencyKey) {

    /** Run WAT source on the wasm lane. */
    public static ExecuteRequest wasmWat(String wat) {
        return new ExecuteRequest(Lane.WASM, Source.wasmWat(wat), null, null, null, null, null);
    }

    /** Run WAT source on the wasm lane with the given JSON input. */
    public static ExecuteRequest wasmWat(String wat, Object input) {
        return new ExecuteRequest(Lane.WASM, Source.wasmWat(wat), null, input, null, null, null);
    }

    public static Builder builder(Lane lane, Source source) {
        return new Builder(lane, source);
    }

    /** Fluent builder for requests that set more than the required fields. */
    public static final class Builder {
        private final Lane lane;
        private final Source source;
        private String entrypoint;
        private Object input;
        private String stdin;
        private Policy policy;
        private String idempotencyKey;

        private Builder(Lane lane, Source source) {
            this.lane = lane;
            this.source = source;
        }

        public Builder entrypoint(String entrypoint) {
            this.entrypoint = entrypoint;
            return this;
        }

        public Builder input(Object input) {
            this.input = input;
            return this;
        }

        public Builder stdin(String stdin) {
            this.stdin = stdin;
            return this;
        }

        public Builder policy(Policy policy) {
            this.policy = policy;
            return this;
        }

        public Builder idempotencyKey(String idempotencyKey) {
            this.idempotencyKey = idempotencyKey;
            return this;
        }

        public ExecuteRequest build() {
            return new ExecuteRequest(lane, source, entrypoint, input, stdin, policy, idempotencyKey);
        }
    }
}
