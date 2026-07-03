package ai.beatbox.examples;

import ai.beatbox.BeatboxApiException;
import ai.beatbox.BeatboxClient;
import ai.beatbox.BeatboxTransportException;
import ai.beatbox.model.ExecuteRequest;
import ai.beatbox.model.ExecutionResult;
import java.util.Map;

/**
 * Runs a one-instruction "add one" wasm module against a live daemon and asserts the value is 42.
 *
 * <p>Run with a daemon on {@code http://127.0.0.1:7300}:
 * <pre>{@code
 *   BEATBOX_API_KEY=... mvn -q exec:java -Dexec.mainClass=ai.beatbox.examples.AddOneExample
 * }</pre>
 */
public final class AddOneExample {

    private static final String ADD_ONE_WAT =
            "(module (func (export \"run\") (param i64) (result i64) "
                    + "local.get 0 i64.const 1 i64.add))";

    public static void main(String[] args) {
        String baseUrl = System.getenv().getOrDefault("BEATBOX_BASE_URL", "http://127.0.0.1:7300");
        String apiKey = System.getenv("BEATBOX_API_KEY");

        BeatboxClient client = BeatboxClient.builder()
                .baseUrl(baseUrl)
                .apiKey(apiKey)
                .build();

        try {
            ExecutionResult result = client.execute(
                    ExecuteRequest.wasmWat(ADD_ONE_WAT, Map.of("n", 41)));

            System.out.println("status = " + result.status());
            System.out.println("value  = " + result.value());

            long value = ((Number) result.value()).longValue();
            if (value != 42) {
                throw new AssertionError("expected 42 but got " + value);
            }
            System.out.println("OK: 41 + 1 == 42");
        } catch (BeatboxApiException e) {
            System.err.println("API error " + e.status() + " (" + e.code() + "): " + e.getMessage());
            System.exit(1);
        } catch (BeatboxTransportException e) {
            System.err.println("Transport error: " + e.getMessage());
            System.exit(1);
        }
    }

    private AddOneExample() {
    }
}
