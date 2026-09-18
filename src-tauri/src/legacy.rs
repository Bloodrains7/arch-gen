//! Read-only import of the former single-file `.archgen` SQLite format.
//! Projects now live as plain files in a folder (see `project.rs`); this module
//! only reads the latest checkpoint so old work can be saved into a folder.
use crate::project::Project;
use rusqlite::{Connection, OpenFlags};
use std::{path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4147434e;
const MAX_PAYLOAD: usize = 32 * 1024 * 1024;

fn check_header(connection: &Connection) -> Result<(), String> {
    let app: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if app != APPLICATION_ID || version != 1 {
        return Err("Not a supported ArchGen project file. The file was not changed.".into());
    }
    Ok(())
}

fn read_latest(connection: &Connection) -> Result<Project, String> {
    let revision: u64 = connection
        .query_row(
            "SELECT revision FROM revisions ORDER BY revision DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let size: usize = connection
        .query_row(
            "SELECT length(CAST(payload AS BLOB)) FROM revisions WHERE revision = ?1",
            [revision],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if size > MAX_PAYLOAD {
        return Err("Project exceeds the 32 MiB content limit.".into());
    }
    let (payload, project_id): (String, String) = connection
        .query_row(
            "SELECT payload, project_id FROM revisions WHERE revision = ?1",
            [revision],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let project: Project = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    project.validate()?;
    if project.revision != revision || project.id != project_id {
        return Err("Project history is inconsistent. The file was not changed.".into());
    }
    Ok(project)
}

fn load(path: &Path) -> Result<Project, String> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    check_header(&tx)?;
    read_latest(&tx)
}

#[tauri::command]
pub async fn import_legacy_project(path: String) -> Result<Project, String> {
    tokio::task::spawn_blocking(move || load(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn temp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "archgen-legacy-{}-{}.archgen",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn write_legacy(path: &Path, revisions: &[(u64, &str)]) {
        let connection = Connection::open(path).unwrap();
        connection.pragma_update(None, "application_id", APPLICATION_ID).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection.execute_batch("CREATE TABLE revisions (revision INTEGER PRIMARY KEY, project_id TEXT NOT NULL, payload TEXT NOT NULL, saved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);").unwrap();
        for (revision, name) in revisions {
            let payload = serde_json::json!({"schemaVersion":1,"id":"project","name":name,"revision":revision,"activeDocumentId":"doc","documents":[{"id":"doc","name":"Dokument","template":"arc42","language":"sk","revision":0,"sections":[]}]});
            connection
                .execute(
                    "INSERT INTO revisions (revision, project_id, payload) VALUES (?1, ?2, ?3)",
                    params![revision, "project", payload.to_string()],
                )
                .unwrap();
        }
    }

    #[test]
    fn imports_the_latest_checkpoint_without_modifying_the_file() {
        let path = temp_path();
        write_legacy(&path, &[(0, "Old"), (4, "Latest")]);
        let before = std::fs::read(&path).unwrap();
        assert_eq!(load(&path).unwrap().name, "Latest");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_foreign_and_tampered_databases() {
        let path = temp_path();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE valuable_data (id INTEGER);").unwrap();
        drop(connection);
        assert!(load(&path).is_err());
        std::fs::remove_file(&path).unwrap();

        write_legacy(&path, &[(0, "Project")]);
        let connection = Connection::open(&path).unwrap();
        connection.execute("UPDATE revisions SET revision = 7", []).unwrap();
        drop(connection);
        assert!(load(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
