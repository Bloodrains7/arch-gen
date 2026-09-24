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

// The only preprocessor lines this renderer accepts: PlantUML's own C4 stdlib,
// bundled inside plantuml.jar and reachable without any file or network access.
// Canonical (mixed-case) form, since PlantUML resolves stdlib names case-insensitively
// and this is what we emit to the renderer regardless of how the caller cased it —
// no user-controlled casing ever reaches the output, so case cannot be used to sneak
// a different string past the whitelist.
const C4_INCLUDES: [&str; 7] = [
    "!include <C4/C4>",
    "!include <C4/C4_Context>",
    "!include <C4/C4_Container>",
    "!include <C4/C4_Component>",
    "!include <C4/C4_Dynamic>",
    "!include <C4/C4_Deployment>",
    "!include <C4/C4_Sequence>",
];

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
    let mut body = Vec::with_capacity(lines.len().saturating_sub(2));
    for line in &lines[1..lines.len() - 1] {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        // Deliberately restricted renderer: no preprocessing, includes,
        // alternative layouts, multiple diagrams or filename directives —
        // except the exact C4 stdlib includes above (whitelist, not a prefix
        // match, so `!include <C4/../../etc/passwd>` or `!includeurl ...`
        // still fall through to the rejection below).
        if lower.starts_with('!') || lower.starts_with('@') {
            match C4_INCLUDES.iter().find(|allowed| allowed.to_lowercase() == lower) {
                Some(canonical) => body.push((*canonical).to_string()),
                None => return Err(format!(
                    "Local rendering does not support preprocessor/include directives or multiple diagrams, except these C4 stdlib includes: {}.",
                    C4_INCLUDES.join(", ")
                )),
            }
        } else {
            body.push((*line).to_string());
        }
    }
    Ok(format!(
        "@startuml\n!pragma layout smetana\n{}\n@enduml\n",
        body.join("\n")
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
    #[test]
    fn c4_stdlib_includes_are_whitelisted_exactly() {
        for include in C4_INCLUDES {
            // Accepted verbatim, with surrounding whitespace, and in any case —
            // PlantUML itself resolves stdlib names case-insensitively, and we
            // always emit the canonical spelling regardless of the input's case.
            for variant in [
                include.to_string(),
                format!("  {include}  "),
                include.to_uppercase(),
                include.to_lowercase(),
            ] {
                let source = format!("@startuml\n{variant}\nPerson(a, \"A\", \"desc\")\n@enduml");
                let rendered =
                    validate(&source).unwrap_or_else(|e| panic!("{variant:?} rejected: {e}"));
                assert!(
                    rendered.contains(include),
                    "{variant:?} -> {rendered} missing canonical {include:?}"
                );
            }
        }
    }
    #[test]
    fn only_the_c4_stdlib_includes_are_allowed() {
        for source in [
            "@startuml\n!include <C4/C4_Context>.puml\n@enduml",
            "@startuml\n!include <C4/../../etc/passwd>\n@enduml",
            "@startuml\n!include <C4/C4_Context>\n!include <C4/../../etc/passwd>\n@enduml",
            "@startuml\n!includeurl https://example.com/C4_Context.puml\n@enduml",
            "@startuml\n!include https://raw.githubusercontent.com/plantuml-stdlib/C4-PlantUML/master/C4_Context.puml\n@enduml",
            "@startuml\n!include C4_Context.puml\n@enduml",
            "@startuml\n!include ../secret.puml\n@enduml",
            "@startuml\n!import <C4/C4_Context>\n@enduml",
            "@startuml\n!define FOO bar\n@enduml",
            "@startuml\n!pragma layout dot\n@enduml",
            "@startuml\n!theme amiga\n@enduml",
            "@startuml\n!include <awslib/AWSCommon>\n@enduml",
            // Valid include followed by a second diagram — still rejected as a whole.
            "@startuml\n!include <C4/C4_Context>\n@enduml\n@startuml\n@enduml",
        ] {
            assert!(validate(source).is_err(), "{source}");
        }
    }
    #[test]
    fn generated_input_wraps_the_c4_include_between_pragma_and_body() {
        let rendered =
            validate("@startuml\n!include <C4/C4_Context>\nPerson(a, \"A\", \"desc\")\n@enduml")
                .unwrap();
        assert!(rendered.starts_with("@startuml\n"));
        assert!(rendered.trim_end().ends_with("@enduml"));
        let pragma_at = rendered.find("!pragma layout smetana").unwrap();
        let include_at = rendered.find("!include <C4/C4_Context>").unwrap();
        let body_at = rendered.find("Person(a").unwrap();
        assert!(pragma_at < include_at && include_at < body_at);
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
            "@startuml\n!include <C4/C4_Context>\nPerson(user, \"User\", \"A user\")\nSystem(sys, \"My System\", \"Does something\")\nSystem_Ext(ext, \"External\", \"A dependency\")\nRel(user, sys, \"Uses\")\nRel(sys, ext, \"Calls\")\nSHOW_LEGEND()\n@enduml",
            "@startuml\n!include <C4/C4_Container>\nPerson(user, \"User\", \"A user\")\nSystem_Boundary(c1, \"My System\") {\n  Container(web, \"Web App\", \"Svelte\", \"UI\")\n  Container(api, \"API\", \"Rust\", \"Logic\")\n}\nSystem_Ext(ext, \"External\", \"A dependency\")\nRel(user, web, \"Uses\", \"HTTPS\")\nRel(web, api, \"Calls\", \"IPC\")\nRel(api, ext, \"Calls\", \"HTTPS\")\nLAYOUT_WITH_LEGEND()\n@enduml",
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
