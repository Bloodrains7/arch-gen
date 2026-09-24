//! Release-note templates shared through the project folder:
//! `<project>/templates/release-notes/<name>.md`, versioned in Git with the documents.
//!
//! The files sit outside `documents/`, so a project save never owns, rewrites or
//! deletes them, and they are not part of the project fingerprint.
use crate::project;
use serde::Serialize;
use std::{io::ErrorKind, path::Path};

const DIRECTORY: &str = "templates/release-notes";
const MAX_TEMPLATE: usize = 256 * 1024;
const MAX_TEMPLATES: usize = 100;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateFile {
    file: String,
    content: String,
    /// Why the file could not be used; the others still load.
    error: Option<String>,
}

fn list(root: &Path) -> Result<Vec<TemplateFile>> {
    let directory = root.join(DIRECTORY);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{DIRECTORY}: {e}")),
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.to_ascii_lowercase().ends_with(".md") && project::safe_name(name).is_ok())
        .collect();
    names.sort();
    names.truncate(MAX_TEMPLATES);
    Ok(names
        .into_iter()
        .map(|file| {
            let path = directory.join(&file);
            let read = std::fs::metadata(&path)
                .map_err(|e| e.to_string())
                .and_then(|meta| {
                    if meta.len() > MAX_TEMPLATE as u64 {
                        Err("larger than 256 KiB".to_string())
                    } else {
                        std::fs::read(&path).map_err(|e| e.to_string())
                    }
                })
                .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_string()));
            match read {
                Ok(text) => TemplateFile {
                    file,
                    content: text.strip_prefix('\u{feff}').unwrap_or(&text).replace("\r\n", "\n"),
                    error: None,
                },
                Err(error) => TemplateFile { file, content: String::new(), error: Some(error) },
            }
        })
        .collect())
}

fn save(root: &Path, name: &str, content: &str, overwrite: bool) -> Result<String> {
    if content.len() > MAX_TEMPLATE {
        return Err("A release-note template may have at most 256 KiB.".into());
    }
    if name.trim().is_empty() {
        return Err("Give the template a name.".into());
    }
    let file = format!("{}.md", project::slug(name, "release-notes"));
    let path = root.join(DIRECTORY).join(&file);
    if path.exists() && !overwrite {
        return Err(format!("{DIRECTORY}/{file} already exists. Choose another name or overwrite it."));
    }
    project::write_atomic(&path, &format!("{}\n", content.replace("\r\n", "\n").trim_end()))?;
    Ok(file)
}

#[tauri::command]
pub async fn list_release_templates(path: String, project_id: String) -> Result<Vec<TemplateFile>> {
    tokio::task::spawn_blocking(move || {
        let root = Path::new(&path);
        project::ensure_project(root, &project_id)?;
        list(root)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Returns the file name inside `templates/release-notes/`.
#[tauri::command]
pub async fn save_release_template(
    path: String,
    project_id: String,
    name: String,
    content: String,
    overwrite: bool,
) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        let root = Path::new(&path);
        project::ensure_project(root, &project_id)?;
        save(root, &name, &content, overwrite)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_round_trip_without_overwriting_or_escaping_the_folder() {
        let root = std::env::temp_dir().join(format!("archgen-release-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(list(&root).unwrap().is_empty());
        assert_eq!(save(&root, "Zákaznícke poznámky", "## {{version}}\r\n", false).unwrap(), "zakaznicke-poznamky.md");
        assert!(save(&root, "Zákaznícke poznámky", "x", false).unwrap_err().contains("already exists"));
        assert_eq!(save(&root, "../../escape", "x", true).unwrap(), "escape.md");
        assert!(save(&root, "  ", "x", false).is_err());
        std::fs::write(root.join(DIRECTORY).join("broken.md"), [0xff, 0xfe]).unwrap();
        std::fs::write(root.join(DIRECTORY).join("notes.txt"), "ignored").unwrap();
        let listed = list(&root).unwrap();
        let summary: Vec<(&str, &str, bool)> =
            listed.iter().map(|t| (t.file.as_str(), t.content.as_str(), t.error.is_some())).collect();
        assert_eq!(summary, vec![("broken.md", "", true), ("escape.md", "x\n", false), ("zakaznicke-poznamky.md", "## {{version}}\n", false)]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
