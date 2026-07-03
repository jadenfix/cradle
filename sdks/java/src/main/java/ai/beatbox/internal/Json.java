package ai.beatbox.internal;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.PropertyNamingStrategies;

/**
 * Central factory for the {@link ObjectMapper} used across the SDK.
 *
 * <p>The wire format is snake_case ({@code wall_ms}, {@code cpu_time_ms}); models are written
 * idiomatically in camelCase and mapped by the {@code SNAKE_CASE} naming strategy. Null fields are
 * omitted on serialization (partial policies/limits merge onto server defaults), and unknown fields
 * are ignored on deserialization so new server fields never break older clients.
 */
public final class Json {

    private Json() {
    }

    public static ObjectMapper newMapper() {
        ObjectMapper mapper = new ObjectMapper();
        mapper.configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);
        mapper.setPropertyNamingStrategy(PropertyNamingStrategies.SNAKE_CASE);
        mapper.setSerializationInclusion(JsonInclude.Include.NON_NULL);
        return mapper;
    }
}
