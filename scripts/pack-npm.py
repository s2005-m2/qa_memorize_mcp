#!/usr/bin/env python3
"""Assemble npm platform package from existing build artifacts."""

import argparse
import os
import platform
import shutil
import subprocess
import sys

PLATFORMS = {
    "win-x64":    {"binary": "memorize_mcp.exe", "ort": "onnxruntime.dll",        "npm": "qa-memorize-mcp-win-x64"},
    "linux-x64":  {"binary": "memorize_mcp",     "ort": "libonnxruntime.so",       "npm": "qa-memorize-mcp-linux-x64"},
    "osx-x86_64": {"binary": "memorize_mcp",     "ort": "libonnxruntime.dylib",    "npm": "qa-memorize-mcp-darwin-x64"},
    "osx-arm64":  {"binary": "memorize_mcp",     "ort": "libonnxruntime.dylib",    "npm": "qa-memorize-mcp-darwin-arm64"},
}


def detect_platform():
    system = platform.system().lower()
    machine = platform.machine().lower()
    if system == "windows":
        return "win-x64"
    elif system == "linux":
        return "linux-x64"
    elif system == "darwin":
        return "osx-arm64" if machine == "arm64" else "osx-x86_64"
    print(f"ERROR: Cannot auto-detect platform for {system}/{machine}", file=sys.stderr)
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", help="Target platform (auto-detected if omitted)")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()

    plat = args.platform or detect_platform()
    if plat not in PLATFORMS:
        print(f"ERROR: Unknown platform '{plat}'. Choose from: {', '.join(PLATFORMS)}", file=sys.stderr)
        sys.exit(1)

    info = PLATFORMS[plat]
    dist_dir = "dist"
    bin_src = os.path.join(dist_dir, info["binary"])
    ort_src = os.path.join(dist_dir, info["ort"])

    if not args.skip_build:
        subprocess.run([sys.executable, "scripts/package.py", "--platform", plat, "--build"], check=True)
    else:
        if not os.path.exists(bin_src):
            print(f"ERROR: Binary not found: {bin_src}", file=sys.stderr)
            sys.exit(1)

    npm_bin = os.path.join("npm", info["npm"], "bin")
    os.makedirs(npm_bin, exist_ok=True)

    shutil.copy2(bin_src, os.path.join(npm_bin, info["binary"]))
    shutil.copy2(ort_src, os.path.join(npm_bin, info["ort"]))

    model_out = os.path.join(npm_bin, "embedding_model")
    subprocess.run(
        ["node", "scripts/compress-model.mjs", "--input-dir", "embedding_model", "--output-dir", model_out],
        check=True,
    )

    print(f"\nPackaged to {npm_bin}:")
    for entry in sorted(os.listdir(npm_bin)):
        path = os.path.join(npm_bin, entry)
        if os.path.isdir(path):
            for sub in sorted(os.listdir(path)):
                size = os.path.getsize(os.path.join(path, sub))
                print(f"  {entry}/{sub}  ({size:,} bytes)")
        else:
            print(f"  {entry}  ({os.path.getsize(path):,} bytes)")


if __name__ == "__main__":
    main()
