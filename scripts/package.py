#!/usr/bin/env python3
"""Package memorize-mcp with ONNX Runtime for a target platform."""

import argparse
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

ORT_VERSION = "1.23.0"

PLATFORMS = {
    "win-x64": {
        "url": "https://github.com/microsoft/onnxruntime/releases/download/v{ver}/onnxruntime-win-x64-{ver}.zip",
        "lib": "lib/onnxruntime.dll",
        "archive": "zip",
        "binary": "memorize_mcp.exe",
        "prefix": "onnxruntime-win-x64-{ver}",
    },
    "linux-x64": {
        "url": "https://github.com/microsoft/onnxruntime/releases/download/v{ver}/onnxruntime-linux-x64-{ver}.tgz",
        "lib": "lib/libonnxruntime.so",
        "archive": "tgz",
        "binary": "memorize_mcp",
        "prefix": "onnxruntime-linux-x64-{ver}",
    },
    "osx-x86_64": {
        "url": "https://github.com/microsoft/onnxruntime/releases/download/v{ver}/onnxruntime-osx-x86_64-{ver}.tgz",
        "lib": "lib/libonnxruntime.dylib",
        "archive": "tgz",
        "binary": "memorize_mcp",
        "prefix": "onnxruntime-osx-x86_64-{ver}",
    },
    "osx-arm64": {
        "url": "https://github.com/microsoft/onnxruntime/releases/download/v{ver}/onnxruntime-osx-arm64-{ver}.tgz",
        "lib": "lib/libonnxruntime.dylib",
        "archive": "tgz",
        "binary": "memorize_mcp",
        "prefix": "onnxruntime-osx-arm64-{ver}",
    },
}


def detect_platform():
    system = platform.system().lower()
    machine = platform.machine().lower()

    if system == "windows":
        return "win-x64"
    elif system == "linux":
        return "linux-x64"
    elif system == "darwin":
        if machine == "arm64":
            return "osx-arm64"
        return "osx-x86_64"

    print(f"ERROR: Cannot auto-detect platform for {system}/{machine}", file=sys.stderr)
    sys.exit(1)


def download_ort(plat, version, cache_dir):
    info = PLATFORMS[plat]
    url = info["url"].format(ver=version)
    ext = "zip" if info["archive"] == "zip" else "tgz"
    filename = f"onnxruntime-{plat}-{version}.{ext}"
    cached = os.path.join(cache_dir, filename)

    if os.path.exists(cached):
        print(f"Using cached: {cached}")
        return cached

    os.makedirs(cache_dir, exist_ok=True)
    print(f"Downloading {url} ...")
    urllib.request.urlretrieve(url, cached)
    print(f"Saved to {cached}")
    return cached


def extract_lib(archive_path, plat, version, dest_dir):
    info = PLATFORMS[plat]
    prefix = info["prefix"].format(ver=version)
    lib_rel = f"{prefix}/{info['lib']}"

    with tempfile.TemporaryDirectory() as tmp:
        if info["archive"] == "zip":
            with zipfile.ZipFile(archive_path, "r") as zf:
                zf.extract(lib_rel, tmp)
        else:
            with tarfile.open(archive_path, "r:gz") as tf:
                member = tf.getmember(lib_rel)
                tf.extract(member, tmp)

        extracted = os.path.join(tmp, lib_rel)
        dest = os.path.join(dest_dir, os.path.basename(info["lib"]))
        shutil.copy2(extracted, dest)
        return dest


def package(plat, version, build, output_dir):
    info = PLATFORMS[plat]
    project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    if build:
        print("Running cargo build --release ...")
        subprocess.run(["cargo", "build", "--release"], cwd=project_root, check=True)

    binary_src = os.path.join(project_root, "target", "release", info["binary"])
    if not os.path.exists(binary_src):
        print(f"ERROR: Binary not found at {binary_src}", file=sys.stderr)
        print("Run with --build or build manually first.", file=sys.stderr)
        sys.exit(1)

    cache_dir = os.path.join(project_root, ".ort_cache")
    archive = download_ort(plat, version, cache_dir)

    os.makedirs(output_dir, exist_ok=True)

    print(f"Copying binary: {info['binary']}")
    shutil.copy2(binary_src, os.path.join(output_dir, info["binary"]))

    lib_path = extract_lib(archive, plat, version, output_dir)
    print(f"Copying library: {os.path.basename(lib_path)}")

    model_dir_src = os.path.join(project_root, "embedding_model")
    model_dir_dst = os.path.join(output_dir, "embedding_model")
    if os.path.isdir(model_dir_src):
        if os.path.exists(model_dir_dst):
            shutil.rmtree(model_dir_dst)
        os.makedirs(model_dir_dst)
        for fname in ("model_ort.onnx", "tokenizer.json"):
            src = os.path.join(model_dir_src, fname)
            if os.path.exists(src):
                print(f"Copying model file: {fname}")
                shutil.copy2(src, os.path.join(model_dir_dst, fname))
            else:
                print(f"WARNING: {src} not found, skipping", file=sys.stderr)

    print(f"\nPackaged to {output_dir}/")
    for root, dirs, files in os.walk(output_dir):
        level = root.replace(output_dir, "").count(os.sep)
        indent = "  " * level
        print(f"{indent}{os.path.basename(root)}/")
        for f in files:
            print(f"{indent}  {f}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Package memorize-mcp with ONNX Runtime")
    parser.add_argument(
        "--platform",
        choices=list(PLATFORMS.keys()),
        help="Target platform (auto-detected if omitted)",
    )
    parser.add_argument(
        "--ort-version",
        default=ORT_VERSION,
        help=f"ONNX Runtime version (default: {ORT_VERSION})",
    )
    parser.add_argument("--build", action="store_true", help="Run cargo build --release first")
    parser.add_argument("--output", default="dist", help="Output directory (default: dist)")
    args = parser.parse_args()

    plat = args.platform or detect_platform()
    print(f"Platform: {plat}")
    print(f"ORT version: {args.ort_version}")
    package(plat, args.ort_version, args.build, args.output)
