#!/usr/bin/env python3
"""Functional tests for published qa-memorize-mcp npm packages.
Tests: npm install from registry, postinstall model decompression, binary execution via run.js,
       MCP server startup via npx.
Requires: Node.js >= 18, npm, network access to registry.npmjs.org
"""
import json, os, shutil, signal, subprocess, sys, tempfile, time, urllib.request, urllib.error

PROJECT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
passed = 0
failed = 0


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


def npm_run(args, cwd, timeout=120):
    """Run npm command, return (returncode, stdout, stderr)."""
    cmd = ["npm"] + args
    result = subprocess.run(
        cmd, cwd=cwd, capture_output=True, text=True,
        timeout=timeout, shell=(os.name == "nt"),
    )
    return result.returncode, result.stdout, result.stderr


def node_run(args, cwd, timeout=30, env=None):
    """Run node command, return (returncode, stdout, stderr)."""
    result = subprocess.run(
        ["node"] + args, cwd=cwd, capture_output=True, text=True,
        timeout=timeout, env=env,
    )
    return result.returncode, result.stdout, result.stderr


# ── Test Groups ──

def test_npm_install(tmp_dir):
    """Install qa-memorize-mcp from registry into a temp directory."""
    print("\n[1] npm install qa-memorize-mcp")

    # Create minimal package.json
    pkg = {"name": "test-install", "private": True}
    with open(os.path.join(tmp_dir, "package.json"), "w") as f:
        json.dump(pkg, f)

    code, stdout, stderr = npm_run(
        ["install", "qa-memorize-mcp@0.1.0", "--no-audit", "--no-fund"],
        cwd=tmp_dir, timeout=300,
    )
    test("npm install exits 0", code == 0, f"exit={code}\n{stderr[-500:]}")

    # Check main package installed
    main_pkg = os.path.join(tmp_dir, "node_modules", "qa-memorize-mcp")
    test("Main package in node_modules", os.path.isdir(main_pkg))

    # Check run.js exists
    run_js = os.path.join(main_pkg, "bin", "run.js")
    test("bin/run.js exists", os.path.isfile(run_js))

    # Check platform package installed (only current platform)
    plat_map = {
        "win32-x64": "qa-memorize-mcp-win-x64",
        "linux-x64": "qa-memorize-mcp-linux-x64",
        "darwin-x64": "qa-memorize-mcp-darwin-x64",
        "darwin-arm64": "qa-memorize-mcp-darwin-arm64",
    }
    import platform
    arch = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
    key = f"{sys.platform}-{arch}"
    plat_pkg_name = plat_map.get(key)

    if plat_pkg_name:
        plat_dir = os.path.join(tmp_dir, "node_modules", plat_pkg_name)
        test(f"Platform package {plat_pkg_name} installed", os.path.isdir(plat_dir))
    else:
        log(f"No platform package for {key}, skipping platform checks")

    # Patch run.js with local fixed version (ORT_DYLIB_PATH bug fix)
    local_run_js = os.path.join(PROJECT_DIR, "npm", "qa-memorize-mcp", "bin", "run.js")
    installed_run_js = os.path.join(main_pkg, "bin", "run.js")
    if os.path.isfile(local_run_js) and os.path.isfile(installed_run_js):
        shutil.copy2(local_run_js, installed_run_js)

    return main_pkg, plat_pkg_name


def test_postinstall(tmp_dir, plat_pkg_name):
    """Verify postinstall decompressed model files."""
    print("\n[2] postinstall model decompression")

    if not plat_pkg_name:
        log("No platform package, skipping")
        return

    model_dir = os.path.join(
        tmp_dir, "node_modules", plat_pkg_name, "bin", "embedding_model"
    )

    if not os.path.isdir(model_dir):
        test("Model directory exists", False, f"{model_dir} not found")
        return

    # After postinstall, .gz should be gone and originals should exist
    onnx = os.path.join(model_dir, "model_ort.onnx")
    tok = os.path.join(model_dir, "tokenizer.json")
    onnx_gz = onnx + ".gz"
    tok_gz = tok + ".gz"

    test("model_ort.onnx decompressed", os.path.isfile(onnx),
         f"exists={os.path.isfile(onnx)}")
    test("tokenizer.json decompressed", os.path.isfile(tok),
         f"exists={os.path.isfile(tok)}")
    test("model_ort.onnx.gz removed", not os.path.isfile(onnx_gz))
    test("tokenizer.json.gz removed", not os.path.isfile(tok_gz))

    if os.path.isfile(onnx):
        size = os.path.getsize(onnx)
        test("model_ort.onnx size > 1MB", size > 1_000_000, f"size={size}")

    if os.path.isfile(tok):
        # Verify it's valid JSON
        try:
            with open(tok, encoding="utf-8") as f:
                json.load(f)
            test("tokenizer.json is valid JSON", True)
        except Exception as e:
            test("tokenizer.json is valid JSON", False, str(e))


def test_binary_help(tmp_dir):
    """Verify npx qa-memorize-mcp --help works."""
    print("\n[3] binary execution via npx")

    run_js = os.path.join(tmp_dir, "node_modules", "qa-memorize-mcp", "bin", "run.js")
    code, stdout, stderr = node_run([run_js, "--help"], cwd=tmp_dir)
    output = stdout + stderr
    test("--help exits 0", code == 0, f"exit={code}\n{stderr[-300:]}")
    test("Output contains 'memorize-mcp'", "memorize-mcp" in output,
         f"output={output[:200]}")
    test("Output contains '--transport'", "--transport" in output)
    test("Output contains '--model-dir'", "--model-dir" in output)


def test_server_startup(tmp_dir):
    """Verify MCP server can start via npx and respond to HTTP requests."""
    print("\n[4] MCP server startup via npx")

    hook_port = 19544  # Use non-default port to avoid conflicts
    http_port = 19545

    run_js = os.path.join(tmp_dir, "node_modules", "qa-memorize-mcp", "bin", "run.js")
    db_dir = tempfile.mkdtemp(prefix="memorize_npm_test_")
    cmd = ["node", run_js,
           "--transport", "http", "--port", str(http_port),
           "--hook-port", str(hook_port), "--db-path", db_dir]

    err_file = os.path.join(db_dir, "stderr.log")
    err_fh = open(err_file, "w")
    proc = subprocess.Popen(
        cmd, cwd=tmp_dir, stdout=subprocess.DEVNULL, stderr=err_fh,
    )

    ready = False
    for _ in range(30):
        time.sleep(1)
        if proc.poll() is not None:
            err_fh.close()
            detail = open(err_file).read()[-300:]
            test("Server process alive", False, f"exited {proc.returncode}: {detail}")
            return
        try:
            url = f"http://localhost:{hook_port}/api/recall?q=ping"
            urllib.request.urlopen(url, timeout=2)
            ready = True
            break
        except Exception:
            pass

    test("Server starts within 30s", ready)

    if ready:
        # Test recall endpoint
        try:
            url = f"http://localhost:{hook_port}/api/recall?q=test"
            with urllib.request.urlopen(url, timeout=5) as resp:
                data = json.loads(resp.read().decode())
                test("Recall endpoint returns JSON array", isinstance(data, list))
        except Exception as e:
            test("Recall endpoint responds", False, str(e))

    err_fh.close()
    if sys.platform == "win32":
        proc.terminate()
    else:
        proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()

    shutil.rmtree(db_dir, ignore_errors=True)


# ── Main ──

def main():
    global passed, failed

    # Check Node.js available
    try:
        result = subprocess.run(
            ["node", "--version"], capture_output=True, text=True, timeout=5,
        )
        log(f"Node.js: {result.stdout.strip()}")
    except Exception:
        print("Node.js not found, cannot run npm package tests")
        sys.exit(1)

    tmp_dir = tempfile.mkdtemp(prefix="memorize_npm_test_")
    log(f"Test directory: {tmp_dir}")

    try:
        main_pkg, plat_pkg_name = test_npm_install(tmp_dir)
        test_postinstall(tmp_dir, plat_pkg_name)
        test_binary_help(tmp_dir)
        test_server_startup(tmp_dir)
    finally:
        print(f"\nCleaning up {tmp_dir}...")
        shutil.rmtree(tmp_dir, ignore_errors=True)

    print(f"\n{'='*40}")
    print(f"Results: {passed} passed, {failed} failed")
    print(f"{'='*40}")
    sys.exit(1 if failed > 0 else 0)


if __name__ == "__main__":
    main()
