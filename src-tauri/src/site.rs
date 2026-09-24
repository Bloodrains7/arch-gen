//! Exports a whole project as a docs-as-code site: a plain folder of Markdown, `toc.yml`
//! and `mkdocs.yml` pages plus rendered diagram SVGs, built on the frontend (site-export.ts)
//! and handed here only to be validated and written. This module never builds the content —
//! only where it is allowed to land.
use crate::project;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::ErrorKind, path::Path};

type Result<T> = std::result::Result<T, String>;

const MAX_FILES: usize = 2000;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Path segments, e.g. `images/<doc>/<section>-<n>.svg` is 3.
const MAX_DEPTH: usize = 4;
const ALLOWED_EXTENSIONS: [&str; 3] = ["md", "yml", "svg"];

#[derive(Debug, Clone, Deserialize)]
pub struct SiteFile {
    path: String,
    content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedSite {
    path: String,
    files: usize,
    bytes: usize,
}

/// `relative` must be a `/`-separated path with no leading `/`, no `..`, an allowed
/// extension and every component safe on every filesystem (see `project::safe_name`).
/// Parsed by hand rather than with `std::path::Path`: the content came from the frontend,
/// not the local OS, so its meaning must not depend on which OS is running this check.
fn validate_relative_path(relative: &str) -> Result<()> {
    let parts: Vec<&str> = relative.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return Err(format!("{relative}: path is nested too deep (max {MAX_DEPTH} segments)."));
    }
    for part in &parts {
        project::safe_name(part).map_err(|_| format!("Unsafe file path in the exported site: {relative:?}"))?;
    }
    let extension = parts
        .last()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    if !extension.is_some_and(|ext| ALLOWED_EXTENSIONS.contains(&ext.as_str())) {
        return Err(format!("{relative}: only .md, .yml and .svg files are written by a site export."));
    }
    Ok(())
}

fn export(root: &Path, files: &[SiteFile]) -> Result<ExportedSite> {
    if files.is_empty() {
        return Err("Nothing to export: the project has no documents.".into());
    }
    if files.len() > MAX_FILES {
        return Err(format!("The site has {} files, more than the {MAX_FILES} limit.", files.len()));
    }
    let bytes: usize = files.iter().map(|f| f.content.len()).sum();
    if bytes > MAX_TOTAL_BYTES {
        return Err(format!(
            "The exported site is {bytes} bytes, over the {MAX_TOTAL_BYTES} (64 MiB) limit."
        ));
    }
    // Validate every file before writing any of them; a case-insensitive path is also a
    // collision (Windows and macOS by default), even though the two strings differ.
    let mut seen = HashSet::new();
    for file in files {
        validate_relative_path(&file.path)?;
        if !seen.insert(file.path.to_ascii_lowercase()) {
            return Err(format!(
                "Two exported files collide once compared case-insensitively: {}",
                file.path
            ));
        }
    }
    // Never overwrite or merge into existing content: the folder must be new or empty.
    if root.is_file() {
        return Err(format!("{}: a file already exists at this path. Choose a folder.", root.display()));
    }
    match std::fs::read_dir(root) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err("The chosen folder is not empty. Site export never overwrites or merges into existing content — choose an empty or new folder.".into());
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", root.display())),
    }
    for file in files {
        project::write_atomic(&root.join(&file.path), &file.content)?;
    }
    Ok(ExportedSite { path: root.display().to_string(), files: files.len(), bytes })
}

#[tauri::command]
pub async fn export_site(path: String, files: Vec<SiteFile>) -> Result<ExportedSite> {
    tokio::task::spawn_blocking(move || export(Path::new(&path), &files))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "archgen-site-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn file(path: &str, content: &str) -> SiteFile {
        SiteFile { path: path.into(), content: content.into() }
    }

    #[test]
    fn writes_every_file_into_a_new_or_empty_folder() {
        let dir = TempDir::new();
        let files = vec![
            file("index.md", "# Project\n"),
            file("billing.md", "# Billing\n"),
            file("toc.yml", "- name: Overview\n  href: index.md\n"),
            file("mkdocs.yml", "site_name: Project\nnav:\n  - Overview: index.md\n"),
            file("images/billing/context-1.svg", "<svg></svg>"),
        ];
        let result = export(&dir.0, &files).unwrap();
        assert_eq!(result.files, 5);
        assert_eq!(result.bytes, files.iter().map(|f| f.content.len()).sum::<usize>());
        assert_eq!(std::fs::read_to_string(dir.0.join("index.md")).unwrap(), "# Project\n");
        assert_eq!(std::fs::read_to_string(dir.0.join("images/billing/context-1.svg")).unwrap(), "<svg></svg>");

        // An existing, non-empty folder is refused, and nothing about it changes.
        let error = export(&dir.0, &[file("index.md", "# Changed\n")]).unwrap_err();
        assert!(error.contains("not empty"), "{error}");
        assert_eq!(std::fs::read_to_string(dir.0.join("index.md")).unwrap(), "# Project\n");
    }

    #[test]
    fn an_empty_existing_folder_is_accepted() {
        let dir = TempDir::new();
        std::fs::create_dir_all(&dir.0).unwrap();
        export(&dir.0, &[file("index.md", "# Project\n")]).unwrap();
    }

    #[test]
    fn a_file_where_the_folder_should_be_is_refused() {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.0.parent().unwrap()).unwrap();
        std::fs::write(&dir.0, "not a folder").unwrap();
        let error = export(&dir.0, &[file("index.md", "x")]).unwrap_err();
        assert!(error.contains("a file already exists"), "{error}");
    }

    #[test]
    fn path_traversal_and_absolute_paths_are_rejected_and_nothing_is_written() {
        let dir = TempDir::new();
        for bad in ["../escape.md", "/etc/passwd.md", "a/../../b.md", "a\\b.md", "a:b.md", "images/../x.md"] {
            let error = export(&dir.0, &[file(bad, "x")]).unwrap_err();
            assert!(error.contains("Unsafe file path") || error.contains("too deep"), "{bad}: {error}");
        }
        assert!(!dir.0.exists());
    }

    #[test]
    fn only_md_yml_and_svg_are_accepted() {
        let dir = TempDir::new();
        let error = export(&dir.0, &[file("script.js", "x")]).unwrap_err();
        assert!(error.contains("only .md, .yml and .svg"), "{error}");
        assert!(!dir.0.exists());
    }

    #[test]
    fn depth_is_limited() {
        let dir = TempDir::new();
        let error = export(&dir.0, &[file("a/b/c/d/e.md", "x")]).unwrap_err();
        assert!(error.contains("too deep"), "{error}");
    }

    #[test]
    fn case_insensitive_collisions_are_refused() {
        let dir = TempDir::new();
        let error = export(&dir.0, &[file("Billing.md", "a"), file("billing.md", "b")]).unwrap_err();
        assert!(error.contains("collide"), "{error}");
        assert!(!dir.0.exists());
    }

    #[test]
    fn file_count_and_total_size_are_limited() {
        let dir = TempDir::new();
        let many: Vec<SiteFile> = (0..MAX_FILES + 1).map(|i| file(&format!("images/d/{i}-1.svg"), "x")).collect();
        assert!(export(&dir.0, &many).unwrap_err().contains("more than"));

        let big = vec![file("index.md", &"x".repeat(MAX_TOTAL_BYTES + 1))];
        assert!(export(&dir.0, &big).unwrap_err().contains("64 MiB"));
        assert!(!dir.0.exists());
    }

    #[test]
    fn an_empty_file_list_is_refused() {
        let dir = TempDir::new();
        assert!(export(&dir.0, &[]).unwrap_err().contains("Nothing to export"));
    }
}
