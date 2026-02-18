#!/usr/bin/env python3
"""Functional tests for memorize_mcp hook system.
Tests: /api/recall endpoint, memorize-hook.mjs (Claude Code + Gemini CLI), opencode-plugin.mjs
Requires: built memorize_mcp binary, embedding model files, ONNX Runtime, Node.js
"""
import json, os, signal, subprocess, sys, time, urllib.request, urllib.error, urllib.parse

HOOK_PORT = 19533
HTTP_PORT = 19532
PROJECT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BINARY = os.path.join(PROJECT_DIR, "target", "debug", "memorize_mcp.exe" if sys.platform == "win32" else "memorize_mcp")
MODEL_DIR = os.path.join(PROJECT_DIR, "embedding_model")
HOOK_SCRIPT = os.path.join(PROJECT_DIR, "hooks", "memorize-hook.mjs")
PLUGIN_FILE = os.path.join(PROJECT_DIR, "hooks", "opencode-plugin.mjs")

passed = 0
failed = 0
server_proc = None


def log(msg):
    print(f"  {msg}")


def test(name, condition, detail=""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  PASS: {name}")
    else:
        failed += 1
        print(f"  FAIL: {name}" + (f" - {detail}" if detail else ""))


def recall(q, **kwargs):
    params = {"q": q}
    params.update(kwargs)
    qs = urllib.parse.urlencode(params)
    url = f"http://localhost:{HOOK_PORT}/api/recall?{qs}"
    req = urllib.request.Request(url)
    with urllib.request.urlopen(req, timeout=5) as resp:
        return resp.status, json.loads(resp.read().decode())


def recall_status(q="", **kwargs):
    params = {"q": q} if q else {}
    params.update(kwargs)
    qs = urllib.parse.urlencode(params)
    url = f"http://localhost:{HOOK_PORT}/api/recall?{qs}"
    try:
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status
    except urllib.error.HTTPError as e:
        return e.code



def run_hook(stdin_data):
    env = os.environ.copy()
    env["MEMORIZE_HOOK_PORT"] = str(HOOK_PORT)
    proc = subprocess.run(
        ["node", HOOK_SCRIPT],
        input=json.dumps(stdin_data),
        capture_output=True, text=True, timeout=10, env=env,
    )
    return proc.returncode, proc.stdout.strip(), proc.stderr.strip()


def seed_data(db_dir):
    """Write seed QA data as JSON snapshot so server imports it on startup."""
    snapshot = {
        "version": 1,
        "exported_at": "2025-01-01T00:00:00Z",
        "topics": [{"topic_name": "Rust Programming"}],
        "qa_records": [
            {
                "question": "What is Rust ownership?",
                "answer": "Rust uses ownership to manage memory without GC",
                "topic": "Rust Programming",
                "merged": False,
            },
            {
                "question": "How does Rust handle concurrency?",
                "answer": "Rust prevents data races at compile time via the borrow checker",
                "topic": "Rust Programming",
                "merged": False,
            },
        ],
        "knowledge": [],
    }
    path = os.path.join(db_dir, "memorize_data.json")
    with open(path, "w") as f:
        json.dump(snapshot, f)


def start_server():
    global server_proc
    import tempfile
    tmp = tempfile.mkdtemp(prefix="memorize_test_")
    seed_data(tmp)
    env = os.environ.copy()
    # Find ORT DLL
    try:
        ort_path = subprocess.check_output(
            [sys.executable, "-c",
             "import onnxruntime, os; print(os.path.join(os.path.dirname(onnxruntime.__file__), 'capi', "
             f"'onnxruntime.dll' if '{sys.platform}' == 'win32' else 'libonnxruntime.so'))"],
            text=True
        ).strip()
        env["ORT_DYLIB_PATH"] = ort_path
    except Exception:
        pass

    server_proc = subprocess.Popen(
        [BINARY, "--transport", "http", "--port", str(HTTP_PORT),
         "--hook-port", str(HOOK_PORT), "--db-path", tmp,
         "--model-dir", MODEL_DIR, "--debug"],
        env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    # Wait for server to be ready
    for _ in range(30):
        time.sleep(1)
        try:
            urllib.request.urlopen(f"http://localhost:{HOOK_PORT}/api/recall?q=ping", timeout=2)
            return True
        except Exception:
            pass
    return False


def stop_server():
    global server_proc
    if server_proc:
        if sys.platform == "win32":
            server_proc.terminate()
        else:
            server_proc.send_signal(signal.SIGTERM)
        try:
            server_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server_proc.kill()
        server_proc = None


# ── Test Groups ──

def test_recall_endpoint():
    print("\n[1] /api/recall endpoint")

    status = recall_status()
    test("Missing q param returns 400", status == 400, f"got {status}")

    status, data = recall("Rust ownership memory management")
    test("Recall finds seeded QA", status == 200 and len(data) > 0, f"got {len(data)} results")

    if data:
        first = data[0]
        test("Result has type=qa", first.get("type") == "qa", f"got {first.get('type')}")
        test("Result has question field", "question" in first)
        test("Result has answer field", "answer" in first)
        test("Result has score field", "score" in first)

    status, data = recall("ownership", context="Rust Programming")
    test("Recall with context finds QA", status == 200 and len(data) > 0, f"got {len(data)} results")

    status, data = recall("Rust", limit=1)
    test("Recall respects limit=1", status == 200 and len(data) <= 1, f"got {len(data)} results")

    try:
        url = f"http://localhost:{HTTP_PORT}/api/recall?q=Rust"
        with urllib.request.urlopen(url, timeout=5) as resp:
            test("Recall works on HTTP port too", resp.status == 200)
    except Exception as e:
        test("Recall works on HTTP port too", False, str(e))


def test_hook_claude_code():
    print("\n[2] memorize-hook.mjs — Claude Code (UserPromptSubmit)")

    # With results
    code, stdout, stderr = run_hook({
        "session_id": "test-session",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "What is Rust ownership?"
    })
    test("Exit code 0", code == 0)
    test("Stdout contains [Memory Recall]", "[Memory Recall]" in stdout, f"got: {stdout[:100]}")
    test("Stdout is plain text (not JSON)", not stdout.startswith("{"))

    # With no matching results
    code, stdout, _ = run_hook({
        "session_id": "test",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "xyzzy completely unrelated gibberish 12345"
    })
    # May or may not have results depending on DB state, but should not crash
    test("No crash on unrelated query", code == 0)


def test_hook_gemini_cli():
    print("\n[3] memorize-hook.mjs — Gemini CLI (BeforeAgent)")

    code, stdout, stderr = run_hook({
        "session_id": "test-session",
        "hook_event_name": "BeforeAgent",
        "prompt": "What is Rust ownership?",
        "cwd": "/tmp",
        "timestamp": "2025-01-01T00:00:00Z",
        "transcript_path": ""
    })
    test("Exit code 0", code == 0)

    if stdout:
        try:
            parsed = json.loads(stdout)
            test("Output is valid JSON", True)
            hso = parsed.get("hookSpecificOutput", {})
            test("Has hookSpecificOutput", bool(hso))
            test("hookEventName is BeforeAgent", hso.get("hookEventName") == "BeforeAgent",
                 f"got {hso.get('hookEventName')}")
            test("Has additionalContext", "additionalContext" in hso)
        except json.JSONDecodeError:
            test("Output is valid JSON", False, f"got: {stdout[:100]}")
    else:
        test("Gemini output present", False, "empty stdout")


def test_hook_server_down():
    print("\n[4] memorize-hook.mjs — Server unreachable")

    env = os.environ.copy()
    env["MEMORIZE_HOOK_PORT"] = "19999"  # Nothing running here
    proc = subprocess.run(
        ["node", HOOK_SCRIPT],
        input=json.dumps({
            "session_id": "test",
            "hook_event_name": "UserPromptSubmit",
            "prompt": "test"
        }),
        capture_output=True, text=True, timeout=10, env=env,
    )
    test("Exit code 0 when server down", proc.returncode == 0)
    test("No stdout when server down", proc.stdout.strip() == "")


def test_hook_bad_input():
    print("\n[5] memorize-hook.mjs — Bad input handling")

    # Invalid JSON
    env = os.environ.copy()
    env["MEMORIZE_HOOK_PORT"] = str(HOOK_PORT)
    proc = subprocess.run(
        ["node", HOOK_SCRIPT],
        input="not json at all",
        capture_output=True, text=True, timeout=10, env=env,
    )
    test("Invalid JSON → exit 0", proc.returncode == 0)
    test("Invalid JSON → no stdout", proc.stdout.strip() == "")

    # Empty prompt
    code, stdout, _ = run_hook({
        "session_id": "test",
        "hook_event_name": "UserPromptSubmit",
        "prompt": ""
    })
    test("Empty prompt → exit 0", code == 0)
    test("Empty prompt → no stdout", stdout == "")

    # Unknown hook event
    code, stdout, _ = run_hook({
        "session_id": "test",
        "hook_event_name": "UnknownEvent",
        "prompt": "test"
    })
    test("Unknown event → exit 0", code == 0)
    test("Unknown event → no stdout", stdout == "")


def test_opencode_plugin():
    print("\n[6] opencode-plugin.mjs — Module structure")

    # Check file exists
    test("Plugin file exists", os.path.exists(PLUGIN_FILE))

    # Check it's valid ESM that Node can parse
    result = subprocess.run(
        ["node", "--input-type=module", "-e",
         f"import p from './{os.path.relpath(PLUGIN_FILE, PROJECT_DIR).replace(os.sep, '/')}'; "
         "const h = p({}); "
         "console.log(typeof h['chat.params'])"],
        capture_output=True, text=True, timeout=10, cwd=PROJECT_DIR,
    )
    if result.returncode == 0:
        test("Plugin exports function with chat.params", result.stdout.strip() == "function")
    else:
        # Node might not be available
        if "node" in result.stderr.lower() or result.returncode == 127:
            log("Node.js not available, skipping plugin tests")
        else:
            test("Plugin loads in Node", False, result.stderr[:200])


# ── Main ──

def main():
    global passed, failed

    if not os.path.exists(BINARY):
        print(f"Binary not found: {BINARY}")
        print("Run 'cargo build' first")
        sys.exit(1)

    print("Starting memorize_mcp server...")
    if not start_server():
        print("Failed to start server within 30s")
        stop_server()
        sys.exit(1)
    print("Server ready.\n")

    try:
        test_recall_endpoint()
        test_hook_claude_code()
        test_hook_gemini_cli()
        test_hook_server_down()
        test_hook_bad_input()
        test_opencode_plugin()
    finally:
        print("\nStopping server...")
        stop_server()

    print(f"\n{'='*40}")
    print(f"Results: {passed} passed, {failed} failed")
    print(f"{'='*40}")
    sys.exit(1 if failed > 0 else 0)


if __name__ == "__main__":
    main()
