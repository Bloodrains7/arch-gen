"""Read-only distribution spike. Run with the Python intended for PyO3.

No DSPy import, model discovery or network access. Emits evidence, not a lockfile.
"""
import importlib.metadata as metadata
import json
from pathlib import Path
import platform
import struct
import sys
import sysconfig

REQUIRED = ("dspy", "langgraph", "pydantic", "ollama")


def probe():
    packages = []
    for distribution in metadata.distributions():
        name = distribution.metadata.get("Name")
        if name:
            packages.append({"name": name, "version": distribution.version})
    installed = {p["name"].lower().replace("_", "-") for p in packages}
    missing = [name for name in REQUIRED if name not in installed]
    dll = Path(sys.base_prefix) / f"python{sys.version_info.major}{sys.version_info.minor}.dll"
    return {
        "executable": sys.executable,
        "version": platform.python_version(),
        "architectureBits": struct.calcsize("P") * 8,
        "platform": sysconfig.get_platform(),
        "pythonDll": str(dll),
        "pythonDllExists": dll.is_file(),
        "missingDirectDependencies": missing,
        "installedDistributions": sorted(packages, key=lambda p: p["name"].lower()),
        "portableBundleVerified": False,
        "releaseBlockers": [
            "PyO3 executable links the Python DLL before application startup; ship a matching DLL and stdlib.",
            "Build a dedicated dependency closure with pinned versions and hashes; this inventory is not a lockfile.",
            "Validate native extension DLLs, licenses and offline imports using a staged interpreter.",
            "Run the application on clean Windows without system Python or Java; then test explicit Ollama inference.",
        ],
    }


if __name__ == "__main__":
    result = probe()
    print(json.dumps(result, indent=2))
    raise SystemExit(1 if result["missingDirectDependencies"] or result["architectureBits"] != 64 or not result["pythonDllExists"] else 0)
