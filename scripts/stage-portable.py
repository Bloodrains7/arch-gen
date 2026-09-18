"""Stage a standalone Windows folder and test the real executable/PyO3 bridge.

Example: python scripts/stage-portable.py --executable src-tauri/target/release/arch-gen.exe
Does not build an installer or certify a clean Windows machine. Never overwrites.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    args = parser.parse_args()
    executable = args.executable.resolve()
    runtime = ROOT / "src-tauri/resources/python"
    renderer = ROOT / "src-tauri/resources/renderer"
    if not executable.is_file() or not (runtime / "python314._pth").is_file() or not (renderer / "plantuml.jar").is_file():
        raise FileNotFoundError("Build the executable and run both runtime setup scripts first")
    runtime_manifest = json.loads((runtime / "runtime-manifest.json").read_text(encoding="utf-8"))
    with (ROOT / "python-engine/requirements-win-x64.lock").open("rb") as lock:
        if hashlib.file_digest(lock, "sha256").hexdigest() != runtime_manifest["lockSha256"]:
            raise RuntimeError("Private runtime has an older dependency lock. Stage the matching runtime before packaging.")
    builds = ROOT / "portable-builds"
    builds.mkdir(exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="archgen-win-x64-", dir=builds))
    shutil.copytree(runtime, output, dirs_exist_ok=True)
    shutil.copy2(executable, output / "arch-gen.exe")
    # Engine source must match the checkout being packaged, not an older setup.
    for name in ("agent.py", "ai_provider.py", "runtime_smoke.py", "embedded_entry.py"):
        shutil.copy2(ROOT / "python-engine" / name, output / "python-engine" / name)
    shutil.copytree(renderer, output / "resources/renderer")
    (output / "docs").mkdir()
    shutil.copy2(ROOT / "docs/RUNTIME-DISTRIBUTION.md", output / "docs/RUNTIME-DISTRIBUTION.md")
    with tempfile.TemporaryDirectory(prefix="archgen-check-", dir=ROOT / ".runtime-cache") as work:
        env = {key: os.environ[key] for key in ("SystemRoot", "TEMP", "TMP", "USERPROFILE", "LOCALAPPDATA", "APPDATA") if key in os.environ}
        env["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32")
        # Poison Python variables deliberately. DLL-adjacent _pth must ignore them.
        env["PYTHONHOME"] = str(Path(work) / "missing-host-python")
        env["PYTHONPATH"] = str(Path(work) / "untrusted")
        env["DSPY_CACHEDIR"] = str(Path(work) / "dspy-cache")
        started = time.monotonic()
        completed = subprocess.run([str(output / "arch-gen.exe"), "--offline-runtime-check"],
            cwd=work, env=env, capture_output=True, text=True, timeout=90)
        if completed.returncode:
            print(completed.stdout)
            print(completed.stderr)
            raise RuntimeError(f"Real PyO3 smoke failed ({completed.returncode}); staged folder retained: {output}")
        report = json.loads(completed.stdout)
        if report.get("status") != "ok" or Path(report["prefix"]).resolve() != output.resolve():
            raise RuntimeError("Executable did not use its private Python runtime")
        if report.get("debugBuild") or not report.get("bundledFrontend"):
            raise RuntimeError("Portable builds require --release --features custom-protocol (embedded frontend, no Vite server)")
        report["elapsedSeconds"] = round(time.monotonic() - started, 2)
        report["cleanWindowsVerified"] = False
    # Generate a content manifest for this exact assembled folder, including DLLs.
    files = []
    for file in sorted(output.rglob("*")):
        if file.is_file():
            with file.open("rb") as handle:
                files.append({"path": file.relative_to(output).as_posix(), "bytes": file.stat().st_size,
                              "sha256": hashlib.file_digest(handle, "sha256").hexdigest()})
    (output / "portable-manifest.json").write_text(json.dumps({"smoke": report, "files": files}, indent=2), encoding="utf-8")
    print(json.dumps({"output": str(output), "smoke": report, "files": len(files), "bytes": sum(f["bytes"] for f in files)}, indent=2))


if __name__ == "__main__":
    main()
