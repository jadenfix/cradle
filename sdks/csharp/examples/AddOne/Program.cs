using System;
using System.Threading.Tasks;
using Beatbox;

// Runs a wasm_wat "add one" program against a local beatbox daemon and asserts
// the returned value is 42. Requires a running daemon:
//
//   BEATBOX_BASE_URL   (default http://127.0.0.1:7300)
//   BEATBOX_API_KEY    (optional)

const string Wat =
    "(module (func (export \"run\") (param i64) (result i64) local.get 0 i64.const 1 i64.add))";

var baseUrl = Environment.GetEnvironmentVariable("BEATBOX_BASE_URL") ?? "http://127.0.0.1:7300";
var apiKey = Environment.GetEnvironmentVariable("BEATBOX_API_KEY");

using var client = new BeatboxClient(baseUrl, apiKey);

try
{
    var result = await client.ExecuteAsync(
        ExecuteRequest.WasmWat(Wat, input: new { n = 41 }));

    if (result.Value is null)
    {
        Console.Error.WriteLine($"execution returned no value (status: {result.Status})");
        return 1;
    }

    long value = result.Value.Value.GetInt64();
    Console.WriteLine($"value = {value}");

    if (value != 42)
    {
        Console.Error.WriteLine($"expected 42, got {value}");
        return 1;
    }

    Console.WriteLine("ok");
    return 0;
}
catch (BeatboxApiException ex)
{
    Console.Error.WriteLine($"API error (HTTP {ex.Status}, code {ex.Code ?? "none"}): {ex.Message}");
    return 1;
}
catch (BeatboxTransportException ex)
{
    Console.Error.WriteLine($"transport error: {ex.Message}");
    return 1;
}
