//! Resolve trusted application code without guessing paths from the working directory.
use pyo3::prelude::*;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

fn engine_path(executable: &Path, development: Option<&Path>) -> Result<PathBuf, String> {
    let directory = executable
        .parent()
        .ok_or("Executable directory unavailable")?;
    if directory.join("python314._pth").is_file() {
        let engine = directory.join("python-engine");
        if !engine.join("agent.py").is_file() {
            return Err("Private Python engine is missing. Rebuild the portable bundle.".into());
        }
        return Ok(engine);
    }
    if let Some(engine) = development {
        if engine.join("agent.py").is_file() {
            return Ok(engine.to_owned());
        }
    }
    Err("Private Python runtime is missing. Use a complete portable bundle, not a standalone executable.".into())
}

pub fn initialize() -> Result<(), String> {
    INITIALIZED
        .get_or_init(|| {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            #[cfg(debug_assertions)]
            let development = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../python-engine"));
            #[cfg(not(debug_assertions))]
            let development: Option<PathBuf> = None;
            let engine = engine_path(&executable, development.as_deref())?;
            Python::with_gil(|py| {
                let sys = py.import("sys").map_err(|e| e.to_string())?;
                // Installed application resources may be read-only. Never create
                // bytecode caches alongside vendored source files.
                sys.setattr("dont_write_bytecode", true)
                    .map_err(|e| e.to_string())?;
                py.import("__main__")
                    .and_then(|main| {
                        main.setattr(
                            "__file__",
                            engine.join("embedded_entry.py").to_string_lossy().as_ref(),
                        )
                    })
                    .map_err(|e| e.to_string())?;
                sys.getattr("path")
                    .and_then(|path| {
                        path.call_method1("insert", (0, engine.to_string_lossy().as_ref()))
                    })
                    .map_err(|e| format!("Could not configure Python engine path: {e}"))?;
                Ok(())
            })
        })
        .clone()
}

/// Explicit diagnostic only. Never run model discovery or inference at startup.
pub fn offline_check() -> Result<String, String> {
    initialize()?;
    Python::with_gil(|py| {
        py.import("runtime_smoke")?
            .call_method1("run", (true,))?
            .extract::<String>()
    })
    .map_err(|e| {
        // This explicit diagnostic contains no project prompts; expose traceback
        // to the build smoke runner, not to normal generation/UI errors.
        Python::with_gil(|py| e.print(py));
        format!("Offline Python runtime check failed: {e}")
    })
}

pub fn bundled_frontend() -> bool {
    cfg!(feature = "custom-protocol")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_runtime_never_uses_working_directory() {
        assert!(engine_path(Path::new("missing/arch-gen.exe"), None).is_err());
    }
    #[test]
    fn development_engine_is_explicit() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../python-engine");
        assert_eq!(
            engine_path(Path::new("missing/arch-gen.exe"), Some(&root)).unwrap(),
            root
        );
    }
}
