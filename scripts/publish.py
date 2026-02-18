#!/usr/bin/env python3
"""Manual npm publish workflow for qa-memorize-mcp packages."""

import argparse
import os
import subprocess
import sys

PLATFORMS = {
    "win-x64":    "qa-memorize-mcp-win-x64",
    "linux-x64":  "qa-memorize-mcp-linux-x64",
    "osx-x86_64": "qa-memorize-mcp-darwin-x64",
    "osx-arm64":  "qa-memorize-mcp-darwin-arm64",
}


def detect_platform():
    import platform
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


def load_env():
    env_path = os.path.join(os.path.dirname(os.path.dirname(__file__)), ".env")
    env = {}
    with open(env_path) as f:
        for line in f:
            line = line.strip()
            if "=" in line and not line.startswith("#"):
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip()
    token = env.get("npm_pass_2fa") or env.get("npm_ak")
    if not token:
        print("ERROR: npm_pass_2fa/npm_ak not found in .env", file=sys.stderr)
        sys.exit(1)
    return token


def write_npmrc(pkg_dir, token):
    path = os.path.join(pkg_dir, ".npmrc")
    with open(path, "w") as f:
        f.write(f"//registry.npmjs.org/:_authToken={token}\n")
    return path


def publish_pkg(pkg_dir, dry_run):
    name = os.path.basename(pkg_dir)
    cmd = ["npm", "publish", "--access", "public"]
    if dry_run:
        cmd.append("--dry-run")
    result = subprocess.run(cmd, cwd=pkg_dir, shell=(os.name == "nt"))
    status = "DRY-RUN OK" if dry_run and result.returncode == 0 else ("OK" if result.returncode == 0 else "FAILED")
    print(f"  {name}: {status}")
    return result.returncode == 0


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", help="Target platform (auto-detected if omitted)")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()

    plat = args.platform or detect_platform()
    if plat not in PLATFORMS:
        print(f"ERROR: Unknown platform '{plat}'. Choose from: {', '.join(PLATFORMS)}", file=sys.stderr)
        sys.exit(1)

    token = load_env()
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

    if not args.skip_build:
        subprocess.run(
            [sys.executable, "scripts/pack-npm.py", "--platform", plat, "--skip-build"],
            cwd=root, check=True
        )

    active_npm = PLATFORMS[plat]
    other_platforms = [v for k, v in PLATFORMS.items() if k != plat]
    pkg_order = [active_npm, "qa-memorize-mcp"] + other_platforms

    npmrc_files = []
    print(f"\nPublishing {'(dry-run) ' if args.dry_run else ''}packages:")
    all_ok = True
    for pkg_name in pkg_order:
        pkg_dir = os.path.join(root, "npm", pkg_name)
        if not os.path.isdir(pkg_dir):
            print(f"  {pkg_name}: SKIPPED (not found)")
            continue
        npmrc_files.append(write_npmrc(pkg_dir, token))
        if not publish_pkg(pkg_dir, args.dry_run):
            all_ok = False

    for f in npmrc_files:
        try:
            os.remove(f)
        except OSError:
            pass

    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()
