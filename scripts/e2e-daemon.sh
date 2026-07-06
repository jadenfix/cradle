#!/usr/bin/env bash
set -euo pipefail

tmp="$(mktemp -d "${TMPDIR:-/tmp}/beatbox-e2e.XXXXXX")"
pid=""
proxy_pid=""

cleanup() {
  if [[ -n "$proxy_pid" ]] && kill -0 "$proxy_pid" 2>/dev/null; then
    kill "$proxy_pid" 2>/dev/null || true
    wait "$proxy_pid" 2>/dev/null || true
  fi
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

cargo build -p beatbox -p beatboxd
target_dir="${CARGO_TARGET_DIR:-target}"
beatbox_bin="$target_dir/debug/beatbox"
beatboxd_bin="$target_dir/debug/beatboxd"

env -u BEATBOX_API_KEY "$beatboxd_bin" \
  --addr 127.0.0.1:0 \
  --db-path "$tmp/noauth.sqlite3" \
  >"$tmp/beatboxd-noauth.log" 2>&1 &
noauth_pid="$!"
noauth_running=true
for _ in {1..100}; do
  if grep -q 'refusing to start beatboxd without BEATBOX_API_KEY' "$tmp/beatboxd-noauth.log"; then
    noauth_running=false
    break
  fi
  if grep -q '^beatboxd listening on ' "$tmp/beatboxd-noauth.log"; then
    break
  fi
  noauth_state="$(ps -p "$noauth_pid" -o stat= 2>/dev/null | tr -d '[:space:]' || true)"
  if [[ -z "$noauth_state" || "$noauth_state" == Z* ]]; then
    noauth_running=false
    break
  fi
  sleep 0.1
done
if [[ "$noauth_running" == true ]]; then
  kill "$noauth_pid" 2>/dev/null || true
  wait "$noauth_pid" 2>/dev/null || true
  echo "beatboxd started without BEATBOX_API_KEY" >&2
  cat "$tmp/beatboxd-noauth.log" >&2
  exit 1
fi
if wait "$noauth_pid"; then
  echo "beatboxd without BEATBOX_API_KEY exited successfully" >&2
  cat "$tmp/beatboxd-noauth.log" >&2
  exit 1
fi
grep -q 'refusing to start beatboxd without BEATBOX_API_KEY' "$tmp/beatboxd-noauth.log"

if env -u BEATBOX_API_KEY "$beatboxd_bin" \
  --addr 0.0.0.0:0 \
  --db-path "$tmp/noauth-nonloopback.sqlite3" \
  --allow-unauthenticated \
  >"$tmp/beatboxd-noauth-nonloopback.log" 2>&1; then
  echo "beatboxd accepted unauthenticated non-loopback bind" >&2
  cat "$tmp/beatboxd-noauth-nonloopback.log" >&2
  exit 1
fi
grep -q 'non-loopback address' "$tmp/beatboxd-noauth-nonloopback.log"

BEATBOX_API_KEY=e2e-secret "$beatboxd_bin" \
  --addr 127.0.0.1:0 \
  --db-path "$tmp/jobs.sqlite3" \
  >"$tmp/beatboxd.log" 2>&1 &
pid="$!"

base_url=""
for _ in {1..150}; do
  if ! kill -0 "$pid" 2>/dev/null; then
    cat "$tmp/beatboxd.log" >&2
    exit 1
  fi
  base_url="$(sed -n 's/^beatboxd listening on //p' "$tmp/beatboxd.log" | tail -n1)"
  if [[ -n "$base_url" ]] && curl -fsS "$base_url/v1/health" >"$tmp/health.json"; then
    break
  fi
  sleep 0.1
done

if [[ -z "$base_url" ]]; then
  cat "$tmp/beatboxd.log" >&2
  echo "beatboxd did not report a listening URL" >&2
  exit 1
fi

origin_status="$(curl -sS \
  -o "$tmp/origin-denied.json" \
  -w '%{http_code}' \
  -H 'origin: https://attacker.example' \
  -H 'x-beatbox-api-key: e2e-secret' \
  "$base_url/v1/capabilities")"
if [[ "$origin_status" != "403" ]]; then
  echo "cross-origin REST request returned $origin_status, expected 403" >&2
  cat "$tmp/origin-denied.json" >&2
  exit 1
fi
grep -q '"code":"forbidden"' "$tmp/origin-denied.json"
grep -q 'origin not allowed' "$tmp/origin-denied.json"

duplicate_origin_status="$(curl -sS \
  -o "$tmp/duplicate-origin-denied.json" \
  -w '%{http_code}' \
  -H 'origin: http://localhost:3000' \
  -H 'origin: https://attacker.example' \
  -H 'x-beatbox-api-key: e2e-secret' \
  "$base_url/v1/capabilities")"
if [[ "$duplicate_origin_status" != "403" ]]; then
  echo "duplicate-origin REST request returned $duplicate_origin_status, expected 403" >&2
  cat "$tmp/duplicate-origin-denied.json" >&2
  exit 1
fi
grep -q '"code":"forbidden"' "$tmp/duplicate-origin-denied.json"
grep -q 'origin not allowed' "$tmp/duplicate-origin-denied.json"

ambiguous_auth_status="$(curl -sS \
  -o "$tmp/ambiguous-auth-denied.json" \
  -w '%{http_code}' \
  -H 'x-beatbox-api-key: e2e-secret' \
  -H 'authorization: Bearer e2e-secret' \
  "$base_url/v1/capabilities")"
if [[ "$ambiguous_auth_status" != "401" ]]; then
  echo "ambiguous-auth REST request returned $ambiguous_auth_status, expected 401" >&2
  cat "$tmp/ambiguous-auth-denied.json" >&2
  exit 1
fi
grep -q '"code":"unauthorized"' "$tmp/ambiguous-auth-denied.json"

host_status="$(curl -sS \
  -o "$tmp/host-denied.json" \
  -w '%{http_code}' \
  -H 'host: attacker.example' \
  -H 'x-beatbox-api-key: e2e-secret' \
  "$base_url/v1/capabilities")"
if [[ "$host_status" != "403" ]]; then
  echo "cross-host REST request returned $host_status, expected 403" >&2
  cat "$tmp/host-denied.json" >&2
  exit 1
fi
grep -q '"code":"forbidden"' "$tmp/host-denied.json"
grep -q 'host not allowed' "$tmp/host-denied.json"

python3 - "$base_url" "$tmp/request-target-denied.http" <<'PY'
import socket
import sys
from urllib.parse import urlsplit

base_url, output_path = sys.argv[1], sys.argv[2]
url = urlsplit(base_url)
host = url.hostname
port = url.port or 80
if host is None:
    raise SystemExit(f"base URL has no host: {base_url}")

with socket.create_connection((host, port), timeout=5) as conn:
    conn.sendall(
        (
            "GET http://attacker.example/v1/capabilities HTTP/1.1\r\n"
            f"Host: {url.netloc}\r\n"
            "x-beatbox-api-key: e2e-secret\r\n"
            "Connection: close\r\n"
            "\r\n"
        ).encode("ascii")
    )
    response = b""
    while True:
        chunk = conn.recv(4096)
        if not chunk:
            break
        response += chunk

with open(output_path, "wb") as handle:
    handle.write(response)

status_line = response.split(b"\r\n", 1)[0]
if b" 403 " not in status_line:
    raise SystemExit(
        f"absolute request-target returned {status_line!r}, expected HTTP 403"
    )
if b'"code":"forbidden"' not in response or b"request target not allowed" not in response:
    raise SystemExit(response.decode("utf-8", errors="replace"))
PY

if "$beatbox_bin" run examples/fib.wat --remote "$base_url" --input '{"n":10}' \
  >"$tmp/remote-noauth.out" 2>&1; then
  echo "unauthenticated remote CLI execution unexpectedly succeeded" >&2
  cat "$tmp/remote-noauth.out" >&2
  exit 1
fi
grep -qi 'unauthorized' "$tmp/remote-noauth.out"

proxy_port_file="$tmp/proxy-port"
proxy_log="$tmp/proxy-requests.log"
python3 - "$proxy_port_file" "$proxy_log" <<'PY' &
import socket
import sys
import time

port_file, log_file = sys.argv[1], sys.argv[2]
server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", 0))
server.listen(20)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.getsockname()[1]))

deadline = time.monotonic() + 20
while time.monotonic() < deadline:
    server.settimeout(0.2)
    try:
        conn, _addr = server.accept()
    except socket.timeout:
        continue
    with conn:
        data = conn.recv(4096)
        with open(log_file, "ab") as handle:
            handle.write(data)
        conn.sendall(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n")
PY
proxy_pid="$!"
for _ in {1..100}; do
  if [[ -s "$proxy_port_file" ]]; then
    break
  fi
  if ! kill -0 "$proxy_pid" 2>/dev/null; then
    echo "test proxy exited before reporting a port" >&2
    exit 1
  fi
  sleep 0.05
done
if [[ ! -s "$proxy_port_file" ]]; then
  echo "test proxy did not report a port" >&2
  exit 1
fi
proxy_url="http://127.0.0.1:$(cat "$proxy_port_file")"

env \
  ALL_PROXY="$proxy_url" \
  HTTP_PROXY="$proxy_url" \
  HTTPS_PROXY="$proxy_url" \
  all_proxy="$proxy_url" \
  http_proxy="$proxy_url" \
  https_proxy="$proxy_url" \
  NO_PROXY= \
  no_proxy= \
  "$beatbox_bin" run examples/fib.wat \
  --remote "$base_url" \
  --api-key e2e-secret \
  --input '{"n":10}' \
  >"$tmp/remote-fib.json"
if [[ -s "$proxy_log" ]]; then
  echo "API-key-bearing client request was sent through an ambient proxy" >&2
  cat "$proxy_log" >&2
  exit 1
fi
grep '"status": "ok"' "$tmp/remote-fib.json"
grep '"value": 55' "$tmp/remote-fib.json"
grep '"deterministic": true' "$tmp/remote-fib.json"

wasm_b64="$(base64 < examples/fib.wasm | tr -d '\n')"
cat >"$tmp/job-request.json" <<JSON
{"lane":"wasm","source":{"kind":"wasm_bytes_base64","bytes":"$wasm_b64"},"input":{"n":10}}
JSON

curl -fsS \
  -H 'content-type: application/json' \
  -H 'x-beatbox-api-key: e2e-secret' \
  -d @"$tmp/job-request.json" \
  "$base_url/v1/jobs" \
  >"$tmp/job-create.json"
job_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["job_id"])' "$tmp/job-create.json")"

job_status=""
for _ in {1..100}; do
  curl -fsS \
    -H 'x-beatbox-api-key: e2e-secret' \
    "$base_url/v1/jobs/$job_id" \
    >"$tmp/job.json"
  job_status="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["status"])' "$tmp/job.json")"
  case "$job_status" in
    succeeded) break ;;
    failed|canceled)
      cat "$tmp/job.json" >&2
      exit 1
      ;;
  esac
  sleep 0.1
done

if [[ "$job_status" != "succeeded" ]]; then
  cat "$tmp/job.json" >&2
  echo "job did not succeed" >&2
  exit 1
fi
python3 - "$tmp/job.json" <<'PY'
import json
import sys

job = json.load(open(sys.argv[1]))
assert job["status"] == "succeeded", job
assert job["result"]["status"] == "ok", job
assert job["result"]["value"] == 55, job
PY

cat >"$tmp/mcp-request.json" <<JSON
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_wasm","arguments":{"wasm_base64":"$wasm_b64","input":{"n":10}}}}
JSON
curl -fsS \
  -H 'content-type: application/json' \
  -H 'authorization: Bearer e2e-secret' \
  -d @"$tmp/mcp-request.json" \
  "$base_url/mcp" \
  >"$tmp/mcp-response.json"
python3 - "$tmp/mcp-response.json" <<'PY'
import json
import sys

response = json.load(open(sys.argv[1]))
assert "error" not in response, response
text = response["result"]["content"][0]["text"]
assert not text.lstrip().startswith("{"), response
result = response["result"]["structuredContent"]
assert result["status"] == "ok", result
assert result["value"] == 55, result
assert result["effective_isolation"]["mechanisms"], result
PY

if curl -fsS "$base_url/openapi.json" >"$tmp/openapi-noauth.json" 2>"$tmp/openapi-noauth.err"; then
  echo "unauthenticated OpenAPI request unexpectedly succeeded" >&2
  cat "$tmp/openapi-noauth.json" >&2
  exit 1
fi
curl -fsS \
  -H 'x-beatbox-api-key: e2e-secret' \
  "$base_url/openapi.json" \
  >"$tmp/openapi.json"
python3 - "$tmp/openapi.json" <<'PY'
import json
import sys

spec = json.load(open(sys.argv[1]))
paths = spec["paths"]
assert "/v1/execute" in paths, paths
assert "/v1/jobs" in paths, paths
assert "/mcp" in paths, paths
PY

echo "beatbox daemon e2e passed at $base_url"
