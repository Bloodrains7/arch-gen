"""Build a private Windows x64 CPython runtime from hash-locked artifacts.

This is a build tool, not an installer for the developer's Python environment.
Requires a Windows x64 CPython 3.14 build interpreter with pip. --offline reuses
the verified archive/wheel cache; missing artifacts fail without network access.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".runtime-cache"
TARGET = ROOT / "src-tauri/resources/python"
PYTHON_URL = "https://www.python.org/ftp/python/3.14.7/python-3.14.7-embed-amd64.zip"
# Digest from the official python.org release's Sigstore bundle (TLS retrieved).
# We verify SHA-256 here, not the Sigstore signature/identity chain.
PYTHON_SHA = "d297e5ff019966817ad8502465176139f2d3d840fa4ed84b13bed399a6ab1f15"


def checksum(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def unpack(archive, destination):
    with zipfile.ZipFile(archive) as bundle:
        for entry in bundle.infolist():
            if not (destination / entry.filename).resolve().is_relative_to(destination.resolve()):
                raise ValueError("Archive entry escapes the staging directory")
        bundle.extractall(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--destination", type=Path, default=TARGET,
                        help="New staging destination; only the default or a child of .runtime-cache is allowed")
    args = parser.parse_args()
    if sys.platform != "win32" or struct.calcsize("P") != 8 or sys.version_info[:2] != (3, 14):
        raise RuntimeError("Use Windows x64 CPython 3.14 as the build interpreter")
    target = args.destination.resolve()
    if target != TARGET.resolve() and (target == CACHE.resolve() or not target.is_relative_to(CACHE.resolve())):
        raise ValueError("Destination must be the default runtime or a child of .runtime-cache")
    if target.exists():
        raise FileExistsError(f"Refusing to overwrite {target}")
    CACHE.mkdir(exist_ok=True)
    archive = CACHE / "python-3.14.7-embed-amd64.zip"
    if not archive.exists():
        if args.offline:
            raise FileNotFoundError("Offline Python archive is missing")
        urllib.request.urlretrieve(PYTHON_URL, archive)
    if checksum(archive) != PYTHON_SHA:
        raise ValueError("Python archive SHA-256 mismatch; no archive content executed")
    wheels = CACHE / "wheels-cp314-win-x64"
    wheels.mkdir(exist_ok=True)
    lock = ROOT / "python-engine/requirements-win-x64.lock"
    common = [sys.executable, "-m", "pip", "--isolated"]
    if not args.offline:
        subprocess.run(common + ["download", "--index-url", "https://pypi.org/simple", "--require-hashes",
                       "--only-binary=:all:", "--dest", str(wheels), "-r", str(lock)], check=True)
    stage = Path(tempfile.mkdtemp(prefix="python-stage-", dir=CACHE)).resolve()
    print(f"Staging at {stage}; retained on failure", flush=True)
    unpack(archive, stage)
    subprocess.run(common + ["install", "--no-index", "--find-links", str(wheels), "--require-hashes",
                   "--only-binary=:all:", "--no-compile", "--no-warn-conflicts", "--target", str(stage / "Lib/site-packages"),
                   "-r", str(lock)], check=True)
    # DLL-adjacent _pth applies both to python.exe and to embedded PyO3.
    # No import site: neither user site, registry paths nor .pth executable code.
    (stage / "python314._pth").write_text("python314.zip\n.\nLib/site-packages\npython-engine\n", encoding="ascii")
    engine = stage / "python-engine"
    engine.mkdir()
    for name in ("agent.py", "ai_provider.py", "runtime_smoke.py", "embedded_entry.py"):
        shutil.copy2(ROOT / "python-engine" / name, engine / name)
    shutil.copy2(lock, stage / lock.name)
    packages = []
    for metadata_file in sorted((stage / "Lib/site-packages").glob("*.dist-info/METADATA")):
        from email.parser import Parser
        meta = Parser().parsestr(metadata_file.read_text(encoding="utf-8"))
        packages.append({"name": meta["Name"], "version": meta["Version"],
                         "licenseExpression": meta["License-Expression"], "license": meta["License"]})
    (stage / "runtime-manifest.json").write_text(json.dumps({"pythonUrl": PYTHON_URL, "pythonSha256": PYTHON_SHA,
        "lockSha256": checksum(lock), "packages": packages, "licenseReviewComplete": False}, indent=2), encoding="utf-8")
    # Use a short-lived process with no inherited Python/Java/provider settings.
    env = {key: os.environ[key] for key in ("SystemRoot", "TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA") if key in os.environ}
    env["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32")
    env["DSPY_CACHEDIR"] = str(CACHE / "smoke-dspy")
    env["LITELLM_LOCAL_MODEL_COST_MAP"] = "True"
    completed = subprocess.run([str(stage / "python.exe"), "-B", str(engine / "runtime_smoke.py")],
                               cwd=CACHE, env=env, capture_output=True, text=True, timeout=90)
    print(completed.stdout)
    if completed.returncode:
        print(completed.stderr, file=sys.stderr)
        raise RuntimeError("Private Python offline smoke failed; stage retained")
    # Both final absolute paths must remain in this workspace before promotion.
    if not stage.is_relative_to(CACHE.resolve()) or not target.is_relative_to(ROOT):
        raise ValueError("Invalid staging destination")
    if target.exists():
        raise FileExistsError("Destination appeared while staging; refusing overwrite")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(stage), str(target))
    print(f"Private Python runtime ready: {target}")


if __name__ == "__main__":
    main()
