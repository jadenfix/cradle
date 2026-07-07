"""Run an add-one wasm module and assert the result.

Usage::

    CRADLE_TOKEN=... python examples/add_one.py [base_url]

Defaults to base_url http://127.0.0.1:7300. Requires a running beatbox daemon.
"""

import os
import sys

from beatbox import (
    BeatboxApiError,
    BeatboxTransportError,
    Client,
    ExecuteRequest,
)

ADD_ONE_WAT = (
    '(module (func (export "run") (param i64) (result i64)'
    " local.get 0 i64.const 1 i64.add))"
)


def main() -> int:
    base_url = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:7300"
    client = Client(base_url, token=os.environ.get("CRADLE_TOKEN"))

    try:
        result = client.execute(
            ExecuteRequest.wasm_wat(ADD_ONE_WAT, input={"n": 41})
        )
    except BeatboxApiError as exc:
        print(f"API error (HTTP {exc.status}, code={exc.code}): {exc.message}")
        return 1
    except BeatboxTransportError as exc:
        print(f"transport error: {exc.message}")
        return 1

    print(f"status = {result.status}")
    print(f"value  = {result.value}")
    print(f"fuel   = {result.metrics.fuel_used}")
    assert result.value == 42, f"expected 42, got {result.value!r}"
    print("OK: result.value == 42")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
