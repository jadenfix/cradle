package ai.beatbox;

import static org.junit.jupiter.api.Assertions.assertDoesNotThrow;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetSocketAddress;
import java.net.Proxy;
import java.net.ProxySelector;
import java.net.SocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;

class BaseUrlPolicyTest {

    @Test
    void acceptsSecureAndLoopbackLiteralBaseUrls() {
        assertEquals("https://daemon.example", client("https://daemon.example").baseUrl());
        assertEquals("https://daemon.example/proxy/beatbox", client("https://daemon.example/proxy/beatbox/").baseUrl());
        assertEquals("http://127.0.0.1:7300", client("http://127.0.0.1:7300").baseUrl());
        assertEquals("http://[::1]:7300", client("http://[::1]:7300").baseUrl());
    }

    @Test
    void rejectsBaseUrlsThatCouldLeakApiKeys() {
        for (String baseUrl : new String[] {
                " http://127.0.0.1:7300",
                "http://127.0.0.1:7300 ",
                "http://localhost:7300",
                "http://127.1:7300",
                "http://10.0.0.1:7300",
                "http://192.168.1.10:7300",
                "http://example.com",
                "ftp://127.0.0.1:7300",
                "https://user@example.com",
                "https://user:pass@example.com",
                "https://example.com?api=v1",
                "https://example.com#fragment",
                "/relative",
        }) {
            assertThrows(IllegalArgumentException.class, () -> client(baseUrl), baseUrl);
        }
    }

    @Test
    void rejectsRetargetingPathPrefixes() {
        for (String baseUrl : new String[] {
                "https://example.com/base/../admin",
                "https://example.com/base/%2e%2e/admin",
                "https://example.com/base/%2E/admin",
                "https://example.com/base/%2Fadmin",
                "https://example.com/base/%5Cadmin",
                "https://example.com/base\\admin",
                "https://example.com/base/%",
        }) {
            assertThrows(IllegalArgumentException.class, () -> client(baseUrl), baseUrl);
        }
    }

    @Test
    void preservesValidatedPathPrefixes() {
        BeatboxClient client = client("https://daemon.example/proxy/beatbox/");

        assertEquals("https://daemon.example/proxy/beatbox/v1/jobs/job-1", client.jobUri("job-1").toString());
    }

    @Test
    void preservesEscapedBasePrefixWhileEscapingJobId() {
        BeatboxClient client = client("https://daemon.example/proxy/a%20b");

        assertEquals(
                "https://daemon.example/proxy/a%20b/v1/jobs/..%2Fexecute",
                client.jobUri("../execute").toString());
    }

    @Test
    void acceptsCustomHttpClientOnlyWhenRedirectsAreDisabled() {
        assertDoesNotThrow(() -> BeatboxClient.builder()
                .baseUrl("https://daemon.example")
                .httpClient(HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NEVER).build())
                .build());

        assertThrows(IllegalArgumentException.class, () -> BeatboxClient.builder()
                .baseUrl("https://daemon.example")
                .httpClient(HttpClient.newBuilder().followRedirects(HttpClient.Redirect.ALWAYS).build())
                .build());
    }

    @Test
    void rejectsCustomHttpClientWithProxy() {
        HttpClient proxied = HttpClient.newBuilder()
                .followRedirects(HttpClient.Redirect.NEVER)
                .proxy(ProxySelector.of(InetSocketAddress.createUnresolved("proxy.example", 8080)))
                .build();

        assertThrows(IllegalArgumentException.class, () -> BeatboxClient.builder()
                .baseUrl("http://127.0.0.1:7300")
                .httpClient(proxied)
                .build());
    }

    @Test
    void rejectsCustomHttpClientForPlaintextLoopback() {
        HttpClient custom = HttpClient.newBuilder()
                .followRedirects(HttpClient.Redirect.NEVER)
                .build();

        assertThrows(IllegalArgumentException.class, () -> BeatboxClient.builder()
                .baseUrl("http://127.0.0.1:7300")
                .httpClient(custom)
                .build());
    }

    @Test
    void defaultClientBypassesJvmProxySelectorForPlaintextLoopback() throws Exception {
        ProxySelector original = ProxySelector.getDefault();
        AtomicInteger proxySelections = new AtomicInteger();
        AtomicReference<String> authorization = new AtomicReference<>();
        AtomicReference<String> apiKey = new AtomicReference<>();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        try {
            server.createContext("/v1/capabilities", exchange -> {
                authorization.set(exchange.getRequestHeaders().getFirst("Authorization"));
                apiKey.set(exchange.getRequestHeaders().getFirst("x-beatbox-api-key"));
                byte[] body = "{}".getBytes();
                exchange.sendResponseHeaders(200, body.length);
                exchange.getResponseBody().write(body);
                exchange.close();
            });
            server.start();
            ProxySelector.setDefault(new ProxySelector() {
                @Override
                public List<Proxy> select(URI uri) {
                    proxySelections.incrementAndGet();
                    return List.of(new Proxy(
                            Proxy.Type.HTTP,
                            InetSocketAddress.createUnresolved("proxy.example", 8080)));
                }

                @Override
                public void connectFailed(URI uri, SocketAddress sa, IOException ioe) {
                }
            });

            BeatboxClient client = BeatboxClient.builder()
                    .baseUrl("http://127.0.0.1:" + server.getAddress().getPort())
                    .token("secret-token")
                    .build();

            client.capabilities();

            assertEquals(0, proxySelections.get());
            assertEquals("Bearer secret-token", authorization.get());
            assertEquals(null, apiKey.get());
        } finally {
            server.stop(0);
            ProxySelector.setDefault(original);
        }
    }

    private static BeatboxClient client(String baseUrl) {
        return BeatboxClient.builder().baseUrl(baseUrl).build();
    }
}
