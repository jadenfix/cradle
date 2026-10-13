#!/usr/bin/env python3
"""Verify the real Wasm CLI, authenticated daemon, MCP and SQLite job recovery."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def main():
    subprocess.run(["cargo", "build", "--locked", "--bin", "beatbox", "--bin", "beatboxd"], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT))
    binaries = Path(metadata["target_directory"]) / "debug"
    cli = json.loads(subprocess.check_output([str(binaries / "beatbox"), "run", "examples/fib.wasm", "--input", '{"n":10}'], cwd=ROOT))
    assert cli["status"] == "ok" and cli["value"] == 55 and cli["deterministic"]
    timeout = json.loads(subprocess.check_output([str(binaries / "beatbox"), "run", "examples/spin.wasm", "--input", '{"n":0}', "--policy", "fuel=1000000"], cwd=ROOT))
    assert timeout["status"] == "timeout" and timeout["error"]["code"] == "fuel_exhausted"
    print("PASS: real CLI fib(10)=55 and fuel exhaustion")

    with tempfile.TemporaryDirectory(prefix="cradle-native-e2e-") as directory:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        key = "inert-local-e2e-token"
        log = tempfile.TemporaryFile()
        process = None

        def request(path, payload=None, authenticated=True, expected_status=200):
            headers = {"content-type": "application/json", "accept": "application/json, text/event-stream"}
            if authenticated:
                headers["authorization"] = f"Bearer {key}"
            req = urllib.request.Request(base + path, data=None if payload is None else json.dumps(payload).encode(), headers=headers)
            try:
                with urllib.request.urlopen(req, timeout=10) as response:
                    assert response.status == expected_status
                    return json.loads(response.read())
            except urllib.error.HTTPError as error:
                assert error.code == expected_status, (path, error.code, error.read().decode())
                return json.loads(error.read())

        def start():
            nonlocal process
            process = subprocess.Popen([str(binaries / "beatboxd"), "--addr", f"127.0.0.1:{port}", "--db-path", str(Path(directory) / "jobs.sqlite3")], env=dict(os.environ, BEATBOX_API_KEY=key), stdout=log, stderr=subprocess.STDOUT)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    log.seek(0)
                    raise RuntimeError(log.read().decode())
                try:
                    request("/v1/health", authenticated=False)
                    return
                except urllib.error.URLError:
                    time.sleep(0.1)
            raise RuntimeError("daemon did not become ready")

        def stop():
            if process is not None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

        try:
            start()
            request("/v1/capabilities", authenticated=False, expected_status=401)
            capabilities = request("/v1/capabilities")
            assert capabilities
            body = json.loads((ROOT / "examples/req-fib.json").read_text())
            result = request("/v1/execute", body)
            assert result["status"] == "ok" and result["value"] == 55
            body["idempotency_key"] = "native-e2e-fib"
            job_id = request("/v1/jobs", body, expected_status=202)["job_id"]
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                job = request(f"/v1/jobs/{job_id}")
                if job["status"] not in ("queued", "running"):
                    break
                time.sleep(0.05)
            assert job["status"] == "succeeded" and job["result"]["value"] == 55
            assert request("/v1/jobs", body, expected_status=202)["job_id"] == job_id
            mcp = request("/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}})
            assert "run_wasm" in {tool["name"] for tool in mcp["result"]["tools"]}
            print("PASS: authenticated REST execute, async jobs, idempotency and real MCP catalog")
            stop()
            start()
            restored = request(f"/v1/jobs/{job_id}")
            assert restored["status"] == "succeeded" and restored["result"]["value"] == 55
            assert request("/v1/jobs", body, expected_status=202)["job_id"] == job_id
            print("PASS: durable SQLite result/idempotency survive daemon restart")
        finally:
            stop()
            log.close()


if __name__ == "__main__":
    main()
