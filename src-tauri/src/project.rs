use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4147434e;
const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
const MAX_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub(crate) schema_version: u32,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) revision: u64,
    pub(crate) active_document_id: String,
    pub(crate) documents: Vec<Document>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Document {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) language: String,
    pub(crate) revision: u64,
    pub(crate) sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Section {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) content: String,
    pub(crate) diagrams: Vec<Diagram>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Diagram {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) diagram_type: String,
    pub(crate) format: String,
}

impl Project {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.revision > MAX_REVISION {
            return Err("Unsupported project version or invalid revision.".into());
        }
        if !self
            .documents
            .iter()
            .any(|d| d.id == self.active_document_id)
        {
            return Err("Project must have an active document.".into());
        }
        let mut ids = HashSet::new();
        let mut add = |id: &str| {
            if id.is_empty() || !ids.insert(id.to_owned()) {
                Err("Missing or duplicate project ID.".to_string())
            } else {
                Ok(())
            }
        };
        add(&self.id)?;
        for doc in &self.documents {
            add(&doc.id)?;
            if doc.revision > self.revision {
                return Err("Invalid document revision.".into());
            }
            for section in &doc.sections {
                add(&section.id)?;
                for diagram in &section.diagrams {
                    add(&diagram.id)?;
                }
            }
        }
        Ok(())
    }
}

fn check_header(connection: &Connection) -> Result<(), String> {
    let app: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if app != APPLICATION_ID || version != 1 {
        return Err("Not a supported ArchGen project. The file was not changed.".into());
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
    read_revision(connection, revision)
}

fn read_revision(connection: &Connection, revision: u64) -> Result<Project, String> {
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

fn save(path: &Path, project: &Project, expected_revision: Option<u64>) -> Result<(), String> {
    project.validate()?;
    let payload = serde_json::to_string(project).map_err(|e| e.to_string())?;
    if payload.len() > MAX_PAYLOAD {
        return Err("Project exceeds the 32 MiB content limit.".into());
    }
    // Save As only creates a new file; it must never overwrite an unrelated database.
    let is_new = expected_revision.is_none();
    if is_new {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("Choose a new project filename: {e}"))?;
    }
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if is_new {
        tx.pragma_update(None, "application_id", APPLICATION_ID)
            .map_err(|e| e.to_string())?;
        tx.pragma_update(None, "user_version", 1)
            .map_err(|e| e.to_string())?;
        tx.execute_batch("CREATE TABLE revisions (revision INTEGER PRIMARY KEY, project_id TEXT NOT NULL, payload TEXT NOT NULL, saved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP);").map_err(|e| e.to_string())?;
    } else {
        check_header(&tx)?;
        let current = read_latest(&tx)?;
        if current.id != project.id || Some(current.revision) != expected_revision {
            return Err("Project changed on disk. Open the latest file or use Save As to keep your version.".into());
        }
        if project.revision <= current.revision {
            if project.revision == current.revision
                && serde_json::to_string(&current).map_err(|e| e.to_string())? == payload
            {
                return Ok(());
            }
            return Err("Project revision must increase before saving changes.".into());
        }
    }
    tx.execute(
        "INSERT INTO revisions (revision, project_id, payload) VALUES (?1, ?2, ?3)",
        params![project.revision, project.id, payload],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn load_project(path: String) -> Result<Project, String> {
    tokio::task::spawn_blocking(move || load(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_project(
    path: String,
    project: Project,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || save(Path::new(&path), &project, expected_revision))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    revision: u64,
    saved_at: String,
}

fn ensure_project(connection: &Connection, project_id: &str) -> Result<(), String> {
    check_header(connection)?;
    let id: String = connection
        .query_row(
            "SELECT project_id FROM revisions ORDER BY revision DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if id != project_id {
        return Err("This history belongs to another project.".into());
    }
    Ok(())
}

fn history(
    path: &Path,
    project_id: &str,
    before_revision: Option<u64>,
) -> Result<Vec<HistoryEntry>, String> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    ensure_project(&tx, project_id)?;
    let mut statement = tx.prepare("SELECT revision, saved_at FROM revisions WHERE project_id = ?1 AND revision < ?2 ORDER BY revision DESC LIMIT 50").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(
            params![project_id, before_revision.unwrap_or(MAX_REVISION + 1)],
            |r| {
                Ok(HistoryEntry {
                    revision: r.get(0)?,
                    saved_at: r.get(1)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn historical_project(path: &Path, project_id: &str, revision: u64) -> Result<Project, String> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    ensure_project(&tx, project_id)?;
    let project = read_revision(&tx, revision)?;
    if project.id != project_id {
        return Err("This revision belongs to another project.".into());
    }
    Ok(project)
}

#[tauri::command]
pub async fn list_project_history(
    path: String,
    project_id: String,
    before_revision: Option<u64>,
) -> Result<Vec<HistoryEntry>, String> {
    tokio::task::spawn_blocking(move || history(Path::new(&path), &project_id, before_revision))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn load_project_revision(
    path: String,
    project_id: String,
    revision: u64,
) -> Result<Project, String> {
    tokio::task::spawn_blocking(move || historical_project(Path::new(&path), &project_id, revision))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Project {
        serde_json::from_value(serde_json::json!({"schemaVersion":1,"id":"project","name":"Projekt","revision":0,"activeDocumentId":"doc","documents":[{"id":"doc","name":"Dokument","template":"arc42","language":"sk","revision":0,"sections":[{"id":"section","title":"Schéma","content":"Žiadne stratené zmeny","diagrams":[{"id":"diagram","content":"@startuml\n@enduml","diagram_type":"class","format":"plantuml"}]}]}]})).unwrap()
    }
    fn temp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "archgen-test-{}-{}.archgen",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn roundtrip_history_and_stale_writer() {
        let path = temp_path();
        let mut project = fixture();
        save(&path, &project, None).unwrap();
        assert_eq!(
            serde_json::to_value(load(&path).unwrap()).unwrap(),
            serde_json::to_value(&project).unwrap()
        );
        assert!(save(&path, &project, None).is_err());
        project.revision = 1;
        project.documents[0].revision = 1;
        project.documents[0].sections[0].content = "Updated".into();
        save(&path, &project, Some(0)).unwrap();
        assert!(save(&path, &project, Some(0)).is_err());
        save(&path, &project, Some(1)).unwrap();
        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM revisions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_foreign_database_without_modifying_it() {
        let path = temp_path();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE valuable_data (id INTEGER);")
            .unwrap();
        drop(connection);
        let before = std::fs::read(&path).unwrap();
        assert!(load(&path).is_err());
        assert!(save(&path, &fixture(), Some(0)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_payload_never_creates_file() {
        let path = temp_path();
        let mut project = fixture();
        project.documents[0].sections[0].id = "doc".into();
        assert!(save(&path, &project, None).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn history_survives_reopen_and_reads_immutable_snapshots() {
        let path = temp_path();
        let mut project = fixture();
        save(&path, &project, None).unwrap();
        project.revision = 4;
        project.name = "New name".into();
        save(&path, &project, Some(0)).unwrap();
        let entries = history(&path, "project", None).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.revision).collect::<Vec<_>>(),
            vec![4, 0]
        );
        assert!(!entries[0].saved_at.is_empty());
        assert_eq!(history(&path, "project", Some(4)).unwrap().len(), 1);
        assert_eq!(
            historical_project(&path, "project", 0).unwrap().name,
            "Projekt"
        );
        assert_eq!(load(&path).unwrap().name, "New name");
        assert!(historical_project(&path, "other", 0).is_err());
        assert!(historical_project(&path, "project", 2).is_err());
        assert!(history(&path, "other", None).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_tampered_history_identity() {
        let path = temp_path();
        save(&path, &fixture(), None).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute("UPDATE revisions SET revision = 7", [])
            .unwrap();
        drop(connection);
        assert!(load(&path).is_err());
        assert!(historical_project(&path, "project", 7).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
