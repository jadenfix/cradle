/**
 * Runs fib(10) on the wasm lane and asserts the result.
 *
 *   CRADLE_TOKEN=... node dist/examples/fib.js
 *   # or: npm run example
 *
 * Requires a running beatbox daemon (default http://127.0.0.1:7300; override
 * with CRADLE_BASE_URL).
 */

import assert from "node:assert/strict";
import {
  BeatboxApiError,
  BeatboxClient,
  BeatboxTransportError,
  ExecuteRequest,
} from "../src/index.js";

// The fib WAT from the repo's examples/fib.wat, as a single line.
const FIB_WAT =
  '(module (func $fib (param $n i64) (result i64) local.get $n i64.const 2 i64.lt_s if (result i64) local.get $n else local.get $n i64.const 1 i64.sub call $fib local.get $n i64.const 2 i64.sub call $fib i64.add end) (func (export "run") (param i64) (result i64) local.get 0 call $fib))';

async function main(): Promise<void> {
  const client = new BeatboxClient({
    baseUrl: process.env.CRADLE_BASE_URL ?? "http://127.0.0.1:7300",
    token: process.env.CRADLE_TOKEN,
  });

  const result = await client.execute(
    ExecuteRequest.wasmWat(FIB_WAT, {
      input: { n: 10 },
      policy: { limits: { wall_ms: 5000, fuel: 10_000_000 } },
    }),
  );

  console.log("status:", result.status);
  console.log("value :", result.value);
  console.log("fuel  :", result.metrics.fuel_used);

  // fib(10) === 55
  assert.equal(result.status, "ok");
  assert.equal(result.value, 55);
  console.log("OK: fib(10) === 55");
}

main().catch((err: unknown) => {
  if (err instanceof BeatboxApiError) {
    console.error(`API error ${err.status} [${err.code}]: ${err.message}`);
  } else if (err instanceof BeatboxTransportError) {
    console.error(`Transport error: ${err.message}`);
  } else {
    console.error(err);
  }
  process.exitCode = 1;
});
