"""Exercise a real MCP subprocess and archive round trip in isolated temp dirs."""
import argparse
import json
import os
import queue
import subprocess
import tempfile
import threading
import time
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--server", required=True)
args = parser.parse_args()
server = str(Path(args.server).resolve(strict=True))
root = Path(__file__).resolve().parents[2]

with tempfile.TemporaryDirectory(prefix="unzip-mcp-smoke-") as directory:
    temporary = Path(directory)
    source = temporary / "source"
    source.mkdir()
    (source / "hello.txt").write_text("unzip-rs MCP round trip", encoding="utf-8")
    data = temporary / "data"
    data.mkdir()
    (data / "config.json").write_text(json.dumps({"passwords": ["private-library-secret"]}), encoding="utf-8")
    environment = os.environ.copy()
    environment.pop("UNZIP_CONFIG_PATH", None)
    environment.pop("UNZIP_CATALOG_PATH", None)
    environment.pop("UNZIP_BUNDLED_DIR", None)
    process = subprocess.Popen([server, "--data-dir", str(data)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, encoding="utf-8", env=environment)
    responses = queue.Queue()
    errors = []
    def read_stdout():
        try:
            for line in process.stdout:
                responses.put(json.loads(line))
        except Exception as error:
            responses.put(error)
    def read_stderr():
        errors.extend(process.stderr.read().splitlines())
    threading.Thread(target=read_stdout, daemon=True).start()
    threading.Thread(target=read_stderr, daemon=True).start()
    sequence = 0
    def request(method, params=None, allow_error=False):
        global sequence
        sequence += 1
        identifier = sequence
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params or {}}) + "\n")
        process.stdin.flush()
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            try:
                item = responses.get(timeout=max(0.1, deadline-time.monotonic()))
            except queue.Empty:
                raise AssertionError(f"Timeout waiting for {method}; stderr={errors}")
            if isinstance(item, Exception):
                raise item
            if item.get("id") == identifier:
                if allow_error and "error" in item:
                    return {"isError": True, "protocolError": item["error"]}
                assert "error" not in item, item
                return item["result"]
        raise AssertionError(f"No response to {method}")
    def call(name, arguments=None, expect_error=False):
        result = request("tools/call", {"name": name, "arguments": arguments or {}}, allow_error=expect_error)
        assert bool(result.get("isError", False)) == expect_error, result
        if expect_error:
            return result
        if "structuredContent" in result and result["structuredContent"] is not None:
            return result["structuredContent"]
        return json.loads("".join(block.get("text", "") for block in result["content"]))
    def wait_job(identifier):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            status = call("archive_job_status", {"job_id": identifier})
            if status["status"] in ("completed", "failed", "cancelled"):
                assert status["status"] == "completed", status
                return status
            time.sleep(0.1)
        raise AssertionError("Archive job did not finish")
    try:
        initialized = request("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "unzip-rs-smoke", "version": "1.0"}})
        assert initialized["serverInfo"]["name"], initialized
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        process.stdin.flush()
        tools = request("tools/list")["tools"]
        expected = {"archive_info", "archive_scan", "archive_preview_pack", "archive_start_pack", "archive_start_extract",
                    "archive_job_status", "archive_cancel_job", "archive_catalog_query", "archive_catalog_update_metadata"}
        assert expected == {tool["name"] for tool in tools}, tools
        info = call("archive_info")
        assert "private-library-secret" not in json.dumps(info), info
        bundle_path = Path(info["bundled_dir"])
        assert (bundle_path / "7z.exe").is_file(), info
        assert (bundle_path / "UnRAR.exe").is_file(), info
        call("archive_scan", {"paths": ["relative/archive.zip"]}, expect_error=True)
        pack_out = temporary / "packs"
        preview = call("archive_preview_pack", {"sources": [str(source)], "output_dir": str(pack_out),
            "prefix": "MCP", "prefix_enabled": True, "number_start": "001", "step": 2})
        assert "MCP001" in json.dumps(preview), preview
        assert not pack_out.exists(), "Preview created output directory"
        call("archive_start_pack", {"sources": [str(source)], "output_dir": str(source / "nested")}, expect_error=True)
        assert not (source / "nested").exists(), "Invalid request wrote files"
        job = call("archive_start_pack", {"sources": [str(source)], "output_dir": str(pack_out),
            "prefix": "MCP", "prefix_enabled": True, "number_start": "001", "format": "zip",
            "password_mode": "manual", "password": "roundtrip-secret", "mix_txt": False})
        packed = wait_job(job["job_id"])
        assert "roundtrip-secret" not in json.dumps(packed), packed
        visible = call("archive_job_status", {"job_id": job["job_id"], "include_passwords": True})
        assert "roundtrip-secret" in json.dumps(visible), visible
        archive = next(pack_out.glob("MCP001*.zip"))
        assert archive.exists()
        scanned = call("archive_scan", {"paths": [str(archive)]})
        assert "MCP001" in json.dumps(scanned), scanned
        output = temporary / "extracted"
        unpack = call("archive_start_extract", {"paths": [str(archive)], "output_dir": str(output),
            "passwords": ["roundtrip-secret"]})
        wait_job(unpack["job_id"])
        files = list(output.rglob("hello.txt"))
        assert len(files) == 1 and files[0].read_text(encoding="utf-8") == "unzip-rs MCP round trip", files
        assert archive.exists(), "Extraction removed source archive"
        saved_config = json.loads((data / "config.json").read_text(encoding="utf-8"))
        assert "roundtrip-secret" not in saved_config.get("passwords", []), "Runtime password persisted"
        # A saved failure directory must not be used unless explicitly enabled.
        saved_failure_dir = temporary / "saved-failed"
        (data / "config.json").write_text(json.dumps({"failed_dir": str(saved_failure_dir),
            "failed_dir_enabled": True, "hash_match": False}), encoding="utf-8")
        wrong = call("archive_start_extract", {"paths": [str(archive)],
            "output_dir": str(temporary / "wrong-password"), "passwords": ["intentionally-wrong"]})
        wrong_id = wrong["job_id"]
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            failed = call("archive_job_status", {"job_id": wrong_id})
            if failed["status"] in ("completed", "failed", "cancelled"):
                break
            time.sleep(0.1)
        else:
            raise AssertionError("Wrong-password task did not finish")
        assert archive.exists() and not saved_failure_dir.exists(), "Disabled failure relocation moved source"
        catalog = call("archive_catalog_query", {"page": 1, "page_size": 20})
        assert "roundtrip-secret" not in json.dumps(catalog), catalog
        revealed = call("archive_catalog_query", {"page": 1, "page_size": 20, "include_passwords": True})
        assert "roundtrip-secret" in json.dumps(revealed), revealed
        item = revealed["items"][0]
        call("archive_catalog_update_metadata", {"id": item["record_id"], "metadata": {"notes": "MCP checked", "custom_fields": {"state": "verified"}}})
        updated = call("archive_catalog_query", {"page": 1, "page_size": 20, "search": "MCP001"})
        assert updated["items"][0]["metadata"]["notes"] == "MCP checked", updated
        next_name = call("archive_preview_pack", {"sources": [str(source)], "output_dir": str(pack_out),
            "prefix": "MCP", "prefix_enabled": True, "number_start": "001", "step": 1, "auto_number": True})
        assert "MCP002" in json.dumps(next_name), next_name
        call("archive_cancel_job", {"job_id": "unknown-job"}, expect_error=True)
        call("archive_start_extract", {"paths": [str(archive)], "output_dir": str(temporary / "bad"),
             "failed_dir_enabled": True}, expect_error=True)
        print("PASS: real stdio handshake, 9 tools, validation, preview, encrypted pack/extract, catalog update, auto numbering, password redaction")
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=10)
