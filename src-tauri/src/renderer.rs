//! Local rendering only. Never fall back to PATH Java or a remote service.
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
#[cfg(not(debug_assertions))]
use tauri::Manager;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Semaphore,
};

const MAX_SOURCE: usize = 256 * 1024;
const MAX_SVG: usize = 4 * 1024 * 1024;
static RENDERS: Semaphore = Semaphore::const_new(2);

fn validate(source: &str) -> Result<String, String> {
    if source.len() > MAX_SOURCE {
        return Err("Diagram exceeds the 256 KiB render limit.".into());
    }
    let lines: Vec<_> = source.trim().lines().collect();
    if lines.first().map(|l| l.trim()) != Some("@startuml")
        || lines.last().map(|l| l.trim()) != Some("@enduml")
    {
        return Err("Local rendering requires one @startuml / @enduml diagram.".into());
    }
    for line in &lines[1..lines.len() - 1] {
        let lower = line.trim().to_lowercase();
        // Deliberately restricted first renderer: no preprocessing, includes,
        // alternative layouts, multiple diagrams or filename directives.
        if lower.starts_with('!') || lower.starts_with('@') {
            return Err("Local rendering does not support preprocessor/include directives or multiple diagrams yet.".into());
        }
    }
    Ok(format!(
        "@startuml\n!pragma layout smetana\n{}\n@enduml\n",
        lines[1..lines.len() - 1].join("\n")
    ))
}

fn paths(root: &Path) -> Result<(PathBuf, PathBuf), String> {
    let java = root.join("jre/bin/java.exe");
    let jar = root.join("plantuml.jar");
    if !java.is_file() || !jar.is_file() {
        return Err("Local renderer is not installed. Run scripts/setup-renderer.ps1 in the development checkout; packaged builds need the private renderer resources. No data was sent remotely.".into());
    }
    Ok((java, jar))
}

async fn limited(mut input: impl AsyncRead + Unpin, max: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    (&mut input)
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Could not read renderer output.")?;
    if bytes.len() > max {
        return Err("Renderer output limit exceeded.".into());
    }
    Ok(bytes)
}

async fn render_at(root: &Path, source: &str) -> Result<String, String> {
    let source = validate(source)?;
    let (java, jar) = paths(root)?;
    let _permit = RENDERS
        .try_acquire()
        .map_err(|_| "Two diagrams are already rendering. Try again shortly.")?;
    let mut command = Command::new(java);
    command
        .args([
            "-Xmx256m",
            "-Djava.awt.headless=true",
            "-DPLANTUML_SECURITY_PROFILE=SANDBOX",
            "-DPLANTUML_LIMIT_SIZE=4096",
            "-jar",
        ])
        .arg(jar)
        .args([
            "-tsvg",
            "-pipe",
            "-charset",
            "UTF-8",
            "-nometadata",
            "-failfast2",
        ])
        .env_clear()
        .env("PLANTUML_SECURITY_PROFILE", "SANDBOX")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Windows JVM needs the system directory; no inherited Java options or PATH.
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start the private Java renderer.")?;
    let mut stdin = child.stdin.take().ok_or("Renderer stdin unavailable.")?;
    let stdout = child.stdout.take().ok_or("Renderer stdout unavailable.")?;
    let stderr = child.stderr.take().ok_or("Renderer stderr unavailable.")?;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let write = async {
            stdin
                .write_all(source.as_bytes())
                .await
                .map_err(|_| "Could not send diagram to renderer.".to_string())?;
            drop(stdin);
            Ok::<_, String>(())
        };
        let wait = async {
            child
                .wait()
                .await
                .map_err(|_| "Could not wait for renderer.".to_string())
        };
        let (_, svg, _, status) = tokio::try_join!(
            write,
            limited(stdout, MAX_SVG),
            limited(stderr, 64 * 1024),
            wait
        )?;
        if !status.success() {
            return Err(
                "PlantUML could not render this diagram. Check its syntax and supported features."
                    .into(),
            );
        }
        let svg = String::from_utf8(svg).map_err(|_| "Renderer returned invalid UTF-8.")?;
        if !svg.contains("<svg") || !svg.contains("</svg>") {
            return Err("Renderer did not return an SVG diagram.".into());
        }
        Ok(svg)
    })
    .await;
    match result {
        Ok(Ok(svg)) => Ok(svg),
        other => {
            let _ = child.kill().await;
            match other {
                Ok(Err(error)) => Err(error),
                _ => Err("Local rendering exceeded the 15 second timeout.".into()),
            }
        }
    }
}

#[tauri::command]
pub async fn render_local_diagram(
    app: tauri::AppHandle,
    content: String,
) -> Result<String, String> {
    #[cfg(debug_assertions)]
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/renderer");
    #[cfg(not(debug_assertions))]
    let root = app
        .path()
        .resource_dir()
        .map_err(|_| "Application resource directory unavailable.")?
        .join("resources/renderer");
    let _ = app;
    render_at(&root, &content).await
}

/// Smoke the staged resources, not the development checkout's renderer.
pub async fn offline_check() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = executable
        .parent()
        .ok_or("Executable directory unavailable")?
        .join("resources/renderer");
    render_at(&root, "@startuml\nA -> B: Private runtime\n@enduml").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_and_directives_are_rejected() {
        for source in [
            "",
            "@startuml file\n@enduml",
            "@startuml\n!include secret\n@enduml",
            "@startuml\n  !Pragma layout dot\n@enduml",
            "@startuml\n@enduml\n@startuml\n@enduml",
        ] {
            assert!(validate(source).is_err(), "{source}");
        }
        assert!(validate(&"x".repeat(MAX_SOURCE + 1)).is_err());
        assert!(validate("@startuml\nA -> B: hello\n@enduml")
            .unwrap()
            .contains("!pragma layout smetana"));
    }
    #[tokio::test]
    async fn output_is_bounded() {
        assert!(limited(&b"12345"[..], 4).await.is_err());
        assert_eq!(limited(&b"1234"[..], 4).await.unwrap(), b"1234");
    }
    #[tokio::test]
    #[ignore = "Requires scripts/setup-renderer.ps1 private runtime"]
    async fn private_renderer_smoke() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/renderer");
        for source in [
            "@startuml\nA -> B: Offline\n@enduml",
            "@startuml\nclass Customer\nclass Order\nCustomer --> Order\n@enduml",
        ] {
            let svg = render_at(&root, source).await.unwrap();
            assert!(svg.contains("<svg"));
        }
        assert!(
            render_at(&root, "@startuml\nthis is not valid ???\n@enduml")
                .await
                .is_err()
        );
    }
}
