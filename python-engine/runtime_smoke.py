"""Offline import/native-extension smoke test shared by Python and PyO3.

No model is contacted. Network access fails the test, including during imports.
Run only in a short-lived diagnostic process: the audit hook is permanent.
"""
import json
from pathlib import Path
import sys


def run(require_isolated=True):
    connections = []

    def no_network(event, args):
        if event in ("socket.connect", "socket.getaddrinfo", "socket.sendto"):
            connections.append(event)
            raise RuntimeError("Network access is forbidden during the offline runtime check")

    sys.addaudithook(no_network)
    root = Path(sys.prefix).resolve()
    if require_isolated:
        assert sys.flags.isolated, "Interpreter must use isolated configuration"
        assert (root / "python314._pth").is_file(), "Missing private interpreter path configuration"
        assert all(Path(p).resolve().is_relative_to(root) for p in sys.path), "An import path escaped the private runtime"

    import agent
    import dspy
    import langgraph.graph
    import numpy
    import pydantic_core
    import orjson
    import ssl
    import sqlite3
    import importlib.metadata as metadata
    from packaging.requirements import Requirement
    from unittest.mock import patch

    assert numpy.arange(3).sum() == 3
    assert orjson.loads(orjson.dumps({"ok": True})) == {"ok": True}
    with sqlite3.connect(":memory:") as db:
        assert db.execute("select 1").fetchone() == (1,)
    # Exercise the real decorated Python API and DSPy context, mocking only
    # inference. This is not a live model-quality or Ollama availability test.
    with patch.object(agent, "diagram_gen", return_value="@startuml\nA -> B: offline\n@enduml"):
        result = agent.run_agent("offline fixture", "sequence")
        assert result["format"] == "plantuml" and "offline" in result["content"]
    assert not connections, "An imported dependency attempted network access"
    # Validate the staged dependency closure, including requirements activated
    # by transitive extras. Ignore unrelated packages on the build interpreter.
    seen = set()
    pending = [(dist.metadata["Name"], "") for dist in metadata.distributions()]
    while pending:
        name, extra = pending.pop()
        key = (name.lower().replace("_", "-"), extra)
        if key in seen:
            continue
        seen.add(key)
        for raw in metadata.requires(name) or []:
            requirement = Requirement(raw)
            if requirement.marker and not requirement.marker.evaluate({"extra": extra}):
                continue
            version = metadata.version(requirement.name)
            assert version in requirement.specifier, f"Dependency mismatch: {name} needs {requirement}, got {version}"
            pending.extend((requirement.name, required_extra) for required_extra in requirement.extras)
    import ctypes
    dll_buffer = ctypes.create_unicode_buffer(32768)
    dll_name = ctypes.windll.kernel32.GetModuleFileNameW
    dll_name.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_ulong]
    dll_name.restype = ctypes.c_ulong
    assert dll_name(ctypes.pythonapi._handle, dll_buffer, len(dll_buffer)), "Cannot identify loaded Python DLL"
    python_dll = Path(dll_buffer.value).resolve()
    if require_isolated:
        assert python_dll.is_relative_to(root), "Python DLL came from the host installation"
    modules = {name: str(Path(module.__file__).resolve()) for name, module in {
        "agent": agent, "dspy": dspy, "numpy": numpy, "pydantic_core": pydantic_core, "orjson": orjson,
    }.items()}
    if require_isolated:
        assert all(Path(path).is_relative_to(root) for path in modules.values()), "A module came from the host installation"
    return json.dumps({"status": "ok", "python": sys.version.split()[0], "prefix": str(root),
                       "isolated": bool(sys.flags.isolated), "modules": modules, "networkAttempts": connections,
                       "pythonDll": str(python_dll), "dependencyClosure": "valid",
                       "inference": "mocked", "openssl": ssl.OPENSSL_VERSION})


if __name__ == "__main__":
    print(run())
