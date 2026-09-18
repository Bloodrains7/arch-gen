//! Durable local editing authority. Project files remain explicit user saves.
use crate::{
    commands,
    project::{Diagram, Document, Project, Section},
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};
use tauri::State;

const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
const MAX_REVISION: u64 = 9_007_199_254_740_991;
pub struct Runtime(Mutex<Connection>);
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    let payload = serde_json::to_string(value).map_err(err)?;
    if payload.len() > MAX_PAYLOAD {
        return Err("Content exceeds the 32 MiB limit.".into());
    }
    Ok(payload)
}
fn decode<T: serde::de::DeserializeOwned>(payload: String) -> Result<T> {
    if payload.len() > MAX_PAYLOAD {
        return Err("Content exceeds the 32 MiB limit.".into());
    }
    serde_json::from_str(&payload).map_err(err)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GenerationRequest {
    Documentation {
        description: String,
        template: String,
        language: String,
    },
    Diagram {
        description: String,
        #[serde(rename = "diagramType")]
        diagram_type: String,
        language: String,
        #[serde(rename = "sectionId")]
        section_id: Option<String>,
        #[serde(rename = "diagramId")]
        diagram_id: Option<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    id: String,
    project_id: String,
    document_id: String,
    base_document: Document,
    request: GenerationRequest,
    status: String,
    sections: Option<Vec<Section>>,
    error: Option<String>,
    created_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditSession {
    session_id: String,
    project: Project,
}

impl Runtime {
    pub fn open(path: &Path) -> Result<Self> {
        Self::initialize(Connection::open(path).map_err(err)?)
    }
    fn initialize(mut connection: Connection) -> Result<Self> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS jobs_project ON jobs(project_id);").map_err(err)?;
        let tx = connection.transaction().map_err(err)?;
        let jobs: Vec<Job> = {
            let mut query = tx.prepare("SELECT payload FROM jobs").map_err(err)?;
            let rows = query
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(err)?;
            rows.map(|r| decode(r.map_err(err)?))
                .collect::<Result<_>>()?
        };
        for mut job in jobs {
            if matches!(job.status.as_str(), "queued" | "running") {
                job.status = "interrupted".into();
                job.error = Some("Application stopped before generation completed. Create a new request to retry.".into());
                put_job(&tx, &job)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(Self(Mutex::new(connection)))
    }
    fn transaction<T>(&self, action: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut connection = self.0.lock().map_err(err)?;
        let tx = connection.transaction().map_err(err)?;
        let result = action(&tx)?;
        tx.commit().map_err(err)?;
        Ok(result)
    }
}
fn project_at(db: &Connection, session: &str, expected: Option<u64>) -> Result<Project> {
    let project: Project = decode(
        db.query_row("SELECT payload FROM sessions WHERE id=?1", [session], |r| {
            r.get(0)
        })
        .map_err(err)?,
    )?;
    if expected.is_some_and(|revision| revision != project.revision) {
        return Err("Project changed. Refresh the current edit session before retrying.".into());
    }
    Ok(project)
}
fn put_project(db: &Connection, session: &str, project: &Project) -> Result<()> {
    project.validate()?;
    db.execute(
        "UPDATE sessions SET payload=?2 WHERE id=?1",
        params![session, encode(project)?],
    )
    .map_err(err)?;
    Ok(())
}
fn job_at(db: &Connection, job: &str) -> Result<Job> {
    decode(
        db.query_row("SELECT payload FROM jobs WHERE id=?1", [job], |r| r.get(0))
            .map_err(err)?,
    )
}
fn put_job(db: &Connection, job: &Job) -> Result<()> {
    db.execute("INSERT INTO jobs(id,project_id,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload", params![job.id,job.project_id,encode(job)?]).map_err(err)?;
    Ok(())
}
fn revise(current: &Project, mut proposed: Project, invalidate: bool) -> Result<Project> {
    if current.id != proposed.id {
        return Err("An edit cannot change project identity.".into());
    }
    proposed.revision = current.revision;
    for doc in &mut proposed.documents {
        doc.revision = current
            .documents
            .iter()
            .find(|d| d.id == doc.id)
            .map_or(0, |d| d.revision);
    }
    proposed.validate()?;
    if proposed == *current && !invalidate {
        return Ok(current.clone());
    }
    let revision = current
        .revision
        .checked_add(1)
        .filter(|v| *v <= MAX_REVISION)
        .ok_or("Project revision limit reached.")?;
    for doc in &mut proposed.documents {
        let old = current.documents.iter().find(|d| d.id == doc.id);
        if invalidate {
            doc.revision = revision;
        } else if old != Some(doc) {
            doc.revision = old.map_or(0, |d| d.revision + 1);
        }
    }
    proposed.revision = revision;
    proposed.validate()?;
    encode(&proposed)?;
    Ok(proposed)
}

#[tauri::command]
pub fn open_edit_session(state: State<'_, Runtime>, project: Project) -> Result<EditSession> {
    open_session(&state, project)
}
fn open_session(state: &Runtime, project: Project) -> Result<EditSession> {
    project.validate()?;
    state.transaction(|db| {
        let session_id = id();
        db.execute(
            "INSERT INTO sessions(id,payload) VALUES(?1,?2)",
            params![session_id, encode(&project)?],
        )
        .map_err(err)?;
        Ok(EditSession {
            session_id,
            project,
        })
    })
}
#[tauri::command]
pub fn apply_project_edit(
    state: State<'_, Runtime>,
    session_id: String,
    expected_revision: u64,
    proposed: Project,
    invalidate_documents: bool,
) -> Result<Project> {
    apply_edit(
        &state,
        &session_id,
        expected_revision,
        proposed,
        invalidate_documents,
    )
}
fn apply_edit(
    state: &Runtime,
    session: &str,
    expected: u64,
    proposed: Project,
    invalidate: bool,
) -> Result<Project> {
    state.transaction(|db| {
        let current = project_at(db, session, Some(expected))?;
        let updated = revise(&current, proposed, invalidate)?;
        put_project(db, session, &updated)?;
        Ok(updated)
    })
}
fn validate_request(doc: &Document, request: &GenerationRequest) -> Result<()> {
    let (description, language) = match request {
        GenerationRequest::Documentation {
            description,
            template,
            language,
        } => {
            if template.trim().is_empty() {
                return Err("Choose a documentation template.".into());
            }
            (description, language)
        }
        GenerationRequest::Diagram {
            description,
            diagram_type,
            language,
            section_id,
            diagram_id,
        } => {
            if diagram_type.trim().is_empty() {
                return Err("Choose a diagram type.".into());
            }
            if let Some(section_id) = section_id {
                if !doc.sections.iter().any(|s| &s.id == section_id) {
                    return Err("Target section no longer exists.".into());
                }
            }
            if let Some(diagram_id) = diagram_id {
                let found = doc
                    .sections
                    .iter()
                    .filter(|s| section_id.as_ref().is_none_or(|id| id == &s.id))
                    .flat_map(|s| &s.diagrams)
                    .find(|d| &d.id == diagram_id);
                if found.is_none() {
                    return Err("Target diagram does not belong to the selected section.".into());
                }
                if found.unwrap().diagram_type != *diagram_type {
                    return Err("Updating a diagram must preserve its type.".into());
                }
            }
            (description, language)
        }
    };
    if description.trim().is_empty() || language.trim().is_empty() {
        return Err("Description and language are required.".into());
    }
    encode(request)?;
    Ok(())
}
#[tauri::command]
pub fn create_generation_job(
    state: State<'_, Runtime>,
    session_id: String,
    document_id: String,
    expected_document_revision: u64,
    request: GenerationRequest,
) -> Result<Job> {
    create_job(
        &state,
        &session_id,
        &document_id,
        expected_document_revision,
        request,
    )
}
fn create_job(
    state: &Runtime,
    session: &str,
    document: &str,
    expected: u64,
    request: GenerationRequest,
) -> Result<Job> {
    state.transaction(|db| {
        let project = project_at(db, session, None)?;
        let doc = project
            .documents
            .iter()
            .find(|d| d.id == document)
            .ok_or("Document no longer exists.")?;
        if doc.revision != expected {
            return Err("Document changed before generation started.".into());
        }
        validate_request(doc, &request)?;
        let job = Job {
            id: id(),
            project_id: project.id.clone(),
            document_id: document.into(),
            base_document: doc.clone(),
            request,
            status: "queued".into(),
            sections: None,
            error: None,
            created_at: db
                .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                    r.get(0)
                })
                .map_err(err)?,
        };
        put_job(db, &job)?;
        Ok(job)
    })
}
#[tauri::command]
pub fn list_generation_jobs(state: State<'_, Runtime>, project_id: String) -> Result<Vec<Job>> {
    state.transaction(|db| {
        let mut stmt = db
            .prepare("SELECT payload FROM jobs WHERE project_id=?1 ORDER BY rowid DESC")
            .map_err(err)?;
        let rows = stmt
            .query_map([project_id], |r| r.get::<_, String>(0))
            .map_err(err)?;
        let mut terminal = 0;
        let mut jobs = Vec::new();
        for row in rows {
            let job: Job = decode(row.map_err(err)?)?;
            if matches!(
                job.status.as_str(),
                "accepted" | "recovered" | "discarded" | "cancelled"
            ) {
                terminal += 1;
                if terminal > 50 {
                    continue;
                }
            }
            jobs.push(job);
        }
        Ok(jobs)
    })
}
fn transition(state: &Runtime, job_id: &str, from: &[&str], status: &str) -> Result<Job> {
    state.transaction(|db| {
        let mut job = job_at(db, job_id)?;
        if !from.contains(&job.status.as_str()) {
            return Err(format!("Cannot change a {} job to {status}.", job.status));
        }
        job.status = status.into();
        put_job(db, &job)?;
        Ok(job)
    })
}
#[tauri::command]
pub fn cancel_generation_job(state: State<'_, Runtime>, job_id: String) -> Result<Job> {
    transition(&state, &job_id, &["queued", "running"], "cancelled")
}
#[tauri::command]
pub fn discard_generation_job(state: State<'_, Runtime>, job_id: String) -> Result<Job> {
    transition(
        &state,
        &job_id,
        &["ready", "failed", "interrupted"],
        "discarded",
    )
}

fn finish(state: &Runtime, job_id: &str, output: Result<Vec<Section>>) -> Result<Job> {
    state.transaction(|db| {
        let mut job = job_at(db, job_id)?;
        if job.status != "running" {
            return Ok(job);
        }
        let output = output.and_then(|sections| {
            encode(&sections)?;
            let mut doc = job.base_document.clone();
            doc.sections = sections.clone();
            let project = Project {
                schema_version: 1,
                id: job.project_id.clone(),
                name: String::new(),
                revision: doc.revision,
                active_document_id: doc.id.clone(),
                documents: vec![doc],
            };
            project.validate()?;
            Ok(sections)
        });
        match output {
            Ok(sections) => {
                job.sections = Some(sections);
                job.status = "ready".into();
            }
            Err(error) => {
                job.error = Some(error.chars().take(16000).collect());
                job.status = "failed".into();
            }
        }
        if encode(&job).is_err() {
            job.sections = None;
            job.status = "failed".into();
            job.error = Some("Generation exceeds the 32 MiB job content limit.".into());
        }
        put_job(db, &job)?;
        Ok(job)
    })
}
#[tauri::command]
pub async fn run_generation_job(state: State<'_, Runtime>, job_id: String) -> Result<Job> {
    let job = transition(&state, &job_id, &["queued"], "running")?;
    let output = match &job.request {
        GenerationRequest::Documentation {
            description,
            template,
            language,
        } => commands::generate_documentation(
            description.clone(),
            template.clone(),
            language.clone(),
        )
        .await
        .map(|doc| {
            doc.sections
                .into_iter()
                .map(|s| Section {
                    id: id(),
                    title: s.title,
                    content: s.content,
                    diagrams: s.diagrams.into_iter().map(new_diagram).collect(),
                })
                .collect()
        }),
        GenerationRequest::Diagram {
            description,
            diagram_type,
            language,
            section_id,
            diagram_id,
        } => {
            let (output_format, existing) =
                diagram_generation_input(&job.base_document, diagram_id.as_deref());
            commands::generate_diagram(
                description.clone(),
                diagram_type.clone(),
                output_format,
                language.clone(),
                existing,
            )
            .await
            .map(|result| {
                merge_diagram(
                    &job.base_document,
                    section_id.as_deref(),
                    diagram_id.as_deref(),
                    result,
                )
            })
        }
    };
    finish(&state, &job_id, output)
}
fn new_diagram(d: commands::DiagramResult) -> Diagram {
    Diagram {
        id: id(),
        content: d.content,
        diagram_type: d.diagram_type,
        format: d.format,
    }
}
fn diagram_generation_input(base: &Document, diagram_id: Option<&str>) -> (String, Option<String>) {
    match base
        .sections
        .iter()
        .flat_map(|s| &s.diagrams)
        .find(|d| Some(d.id.as_str()) == diagram_id)
    {
        Some(diagram) => (diagram.format.clone(), Some(diagram.content.clone())),
        None => ("plantuml".into(), None),
    }
}
fn merge_diagram(
    base: &Document,
    section_id: Option<&str>,
    diagram_id: Option<&str>,
    result: commands::DiagramResult,
) -> Vec<Section> {
    let mut sections = base.sections.clone();
    if let Some(target) = diagram_id {
        for section in &mut sections {
            for diagram in &mut section.diagrams {
                if diagram.id == target {
                    diagram.content = result.content;
                    return sections;
                }
            }
        }
    }
    if sections.is_empty() {
        sections.push(Section {
            id: id(),
            title: "Diagram".into(),
            content: String::new(),
            diagrams: vec![],
        });
    }
    let index = section_id
        .and_then(|target| sections.iter().position(|s| s.id == target))
        .unwrap_or(0);
    sections[index].diagrams.push(new_diagram(result));
    sections
}
fn consume(
    state: &Runtime,
    session: &str,
    job_id: &str,
    expected: u64,
    recover: bool,
) -> Result<Project> {
    state.transaction(|db| {
        let current = project_at(db, session, Some(expected))?;
        let mut job = job_at(db, job_id)?;
        if job.status != "ready" {
            return Err("Only a ready proposal can be accepted or recovered.".into());
        }
        if job.project_id != current.id {
            return Err("This proposal belongs to another project.".into());
        }
        let mut proposed = current.clone();
        let sections = job.sections.clone().ok_or("Proposal has no result.")?;
        if recover {
            let mut doc = job.base_document.clone();
            doc.id = id();
            doc.name = format!("{} (recovered)", doc.name);
            doc.sections = sections;
            for section in &mut doc.sections {
                section.id = id();
                for diagram in &mut section.diagrams {
                    diagram.id = id();
                }
            }
            proposed.active_document_id = doc.id.clone();
            proposed.documents.push(doc);
            job.status = "recovered".into();
        } else {
            let doc = proposed.documents.iter_mut().find(|d| d.id == job.document_id)
                .ok_or("Target document no longer exists. Recover the proposal as a new document.")?;
            if *doc != job.base_document {
                return Err("Document changed since generation started. Recover the proposal as a new document.".into());
            }
            doc.sections = sections;
            job.status = "accepted".into();
        }
        let updated = revise(&current, proposed, false)?;
        put_project(db, session, &updated)?;
        put_job(db, &job)?;
        Ok(updated)
    })
}
#[tauri::command]
pub fn accept_generation_job(
    state: State<'_, Runtime>,
    session_id: String,
    job_id: String,
    expected_revision: u64,
) -> Result<Project> {
    consume(&state, &session_id, &job_id, expected_revision, false)
}
#[tauri::command]
pub fn recover_generation_job(
    state: State<'_, Runtime>,
    session_id: String,
    job_id: String,
    expected_revision: u64,
) -> Result<Project> {
    consume(&state, &session_id, &job_id, expected_revision, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Project {
        serde_json::from_value(serde_json::json!({"schemaVersion":1,"id":"project","name":"Project","revision":0,"activeDocumentId":"doc","documents":[{"id":"doc","name":"Document","template":"arc42","language":"sk","revision":0,"sections":[{"id":"section","title":"Title","content":"Original","diagrams":[{"id":"diagram","content":"old","diagram_type":"class","format":"plantuml"}]}]}]})).unwrap()
    }
    fn request() -> GenerationRequest {
        GenerationRequest::Documentation {
            description: "Generate".into(),
            template: "arc42".into(),
            language: "sk".into(),
        }
    }
    fn memory() -> Runtime {
        Runtime::initialize(Connection::open_in_memory().unwrap()).unwrap()
    }
    fn ready(state: &Runtime, session: &str) -> Job {
        let job = create_job(state, session, "doc", 0, request()).unwrap();
        transition(state, &job.id, &["queued"], "running").unwrap();
        let mut sections = job.base_document.sections.clone();
        sections[0].content = "Generated".into();
        finish(state, &job.id, Ok(sections)).unwrap()
    }
    #[test]
    fn authoritative_revisions_cas_noop_and_restore() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let mut proposed = session.project.clone();
        proposed.revision = 999;
        proposed.documents[0].revision = 999;
        assert_eq!(
            apply_edit(&state, &session.session_id, 0, proposed.clone(), false)
                .unwrap()
                .revision,
            0
        );
        proposed.documents[0].sections[0].content = "Edit".into();
        let current = apply_edit(&state, &session.session_id, 0, proposed.clone(), false).unwrap();
        assert_eq!((current.revision, current.documents[0].revision), (1, 1));
        assert!(apply_edit(&state, &session.session_id, 0, proposed, false).is_err());
        let current = apply_edit(&state, &session.session_id, 1, fixture(), true).unwrap();
        assert_eq!((current.revision, current.documents[0].revision), (2, 2));
        let mut next = current.clone();
        next.name = "Rename".into();
        let current = apply_edit(&state, &session.session_id, 2, next, false).unwrap();
        assert_eq!((current.revision, current.documents[0].revision), (3, 2));
        let mut next = current.clone();
        let mut doc = current.documents[0].clone();
        doc.id = "new-doc".into();
        doc.sections.clear();
        doc.revision = 100;
        next.documents.push(doc);
        let current = apply_edit(&state, &session.session_id, 3, next, false).unwrap();
        assert_eq!(current.documents[1].revision, 0);
    }
    #[test]
    fn stale_proposal_recovery_and_replay_are_atomic() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let job = ready(&state, &session.session_id);
        let mut edit = fixture();
        edit.documents[0].sections[0].content = "User edit".into();
        apply_edit(&state, &session.session_id, 0, edit, false).unwrap();
        assert!(consume(&state, &session.session_id, &job.id, 1, false).is_err());
        assert_eq!(
            state
                .transaction(|db| Ok(job_at(db, &job.id)?.status))
                .unwrap(),
            "ready"
        );
        assert!(consume(&state, &session.session_id, &job.id, 0, true).is_err());
        let recovered = consume(&state, &session.session_id, &job.id, 1, true).unwrap();
        assert_eq!(recovered.documents.len(), 2);
        assert_eq!(recovered.documents[0].sections[0].content, "User edit");
        assert_eq!(recovered.documents[1].sections[0].content, "Generated");
        assert_ne!(recovered.documents[1].sections[0].id, "section");
        assert_ne!(recovered.documents[1].sections[0].diagrams[0].id, "diagram");
        assert!(consume(&state, &session.session_id, &job.id, 2, true).is_err());
    }
    #[test]
    fn acceptance_commits_result_and_status_together() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let job = ready(&state, &session.session_id);
        let project = consume(&state, &session.session_id, &job.id, 0, false).unwrap();
        assert_eq!(project.revision, 1);
        assert_eq!(project.documents[0].sections[0].content, "Generated");
        assert_eq!(
            state
                .transaction(|db| Ok(job_at(db, &job.id)?.status))
                .unwrap(),
            "accepted"
        );
        assert!(consume(&state, &session.session_id, &job.id, 1, false).is_err());
    }

    #[test]
    fn identical_text_after_restore_still_invalidates_old_proposal() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let job = ready(&state, &session.session_id);
        apply_edit(&state, &session.session_id, 0, fixture(), true).unwrap();
        assert!(consume(&state, &session.session_id, &job.id, 1, false).is_err());
        let mut foreign = fixture();
        foreign.id = "another-project".into();
        let other = open_session(&state, foreign).unwrap();
        assert!(consume(&state, &other.session_id, &job.id, 0, true).is_err());
        assert_eq!(
            state
                .transaction(|db| Ok(job_at(db, &job.id)?.status))
                .unwrap(),
            "ready"
        );
    }

    #[test]
    fn failure_is_durable_and_invalid_results_never_become_ready() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let job = create_job(&state, &session.session_id, "doc", 0, request()).unwrap();
        transition(&state, &job.id, &["queued"], "running").unwrap();
        let mut sections = job.base_document.sections.clone();
        sections[0].id = "doc".into();
        let failed = finish(&state, &job.id, Ok(sections)).unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.sections.is_none());
        assert!(failed.error.unwrap().contains("duplicate"));
        assert_eq!(
            state
                .transaction(|db| Ok(job_at(db, &job.id)?.status))
                .unwrap(),
            "failed"
        );
        assert_eq!(
            transition(&state, &job.id, &["failed"], "discarded")
                .unwrap()
                .status,
            "discarded"
        );
    }
    #[test]
    fn cancelled_job_cannot_be_revived_by_late_completion() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let job = create_job(&state, &session.session_id, "doc", 0, request()).unwrap();
        transition(&state, &job.id, &["queued"], "running").unwrap();
        transition(&state, &job.id, &["running"], "cancelled").unwrap();
        let late = finish(&state, &job.id, Ok(job.base_document.sections)).unwrap();
        assert_eq!(late.status, "cancelled");
        assert!(late.sections.is_none());
        assert!(transition(&state, &job.id, &["queued"], "running").is_err());
    }
    #[test]
    fn restart_preserves_ready_proposal_and_interrupts_unfinished_work() {
        let path = std::env::temp_dir().join(format!("archgen-runtime-{}.sqlite3", id()));
        let (ready_id, queued_id, running_id, session_id) = {
            let state = Runtime::open(&path).unwrap();
            let session = open_session(&state, fixture()).unwrap();
            let ready = ready(&state, &session.session_id);
            let queued = create_job(&state, &session.session_id, "doc", 0, request()).unwrap();
            let running = create_job(&state, &session.session_id, "doc", 0, request()).unwrap();
            transition(&state, &running.id, &["queued"], "running").unwrap();
            (ready.id, queued.id, running.id, session.session_id)
        };
        {
            let state = Runtime::open(&path).unwrap();
            state
                .transaction(|db| {
                    assert_eq!(job_at(db, &ready_id)?.status, "ready");
                    assert_eq!(job_at(db, &queued_id)?.status, "interrupted");
                    assert_eq!(job_at(db, &running_id)?.status, "interrupted");
                    assert_eq!(project_at(db, &session_id, None)?.revision, 0);
                    Ok(())
                })
                .unwrap();
            let reopened = open_session(&state, fixture()).unwrap();
            assert_eq!(
                consume(&state, &reopened.session_id, &ready_id, 0, true)
                    .unwrap()
                    .documents
                    .len(),
                2
            );
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn identity_and_duplicate_ids_are_rejected_without_write() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let mut invalid = fixture();
        invalid.id = "foreign".into();
        assert!(apply_edit(&state, &session.session_id, 0, invalid, false).is_err());
        let mut invalid = fixture();
        invalid.documents[0].sections[0].id = "doc".into();
        assert!(apply_edit(&state, &session.session_id, 0, invalid, false).is_err());
        assert_eq!(
            state
                .transaction(|db| project_at(db, &session.session_id, None))
                .unwrap(),
            fixture()
        );
    }
    #[test]
    fn exact_diagram_target_preserves_identity_and_other_content() {
        let base = fixture().documents.remove(0);
        let request = GenerationRequest::Diagram {
            description: "Edit".into(),
            diagram_type: "class".into(),
            language: "sk".into(),
            section_id: Some("missing".into()),
            diagram_id: Some("diagram".into()),
        };
        assert!(validate_request(&base, &request).is_err());
        let merged = merge_diagram(
            &base,
            Some("section"),
            Some("diagram"),
            commands::DiagramResult {
                content: "new".into(),
                diagram_type: "class".into(),
                format: "plantuml".into(),
            },
        );
        assert_eq!(merged[0].content, "Original");
        assert_eq!(merged[0].diagrams.len(), 1);
        assert_eq!(merged[0].diagrams[0].id, "diagram");
        assert_eq!(merged[0].diagrams[0].content, "new");
    }

    #[test]
    fn imported_mermaid_updates_request_and_preserve_mermaid_format() {
        let mut base = fixture().documents.remove(0);
        base.sections[0].diagrams[0].format = "mermaid".into();
        base.sections[0].diagrams[0].content = "graph TD; A-->B".into();
        let (format, existing) = diagram_generation_input(&base, Some("diagram"));
        assert_eq!(format, "mermaid");
        assert_eq!(existing.as_deref(), Some("graph TD; A-->B"));
        let merged = merge_diagram(
            &base,
            Some("section"),
            Some("diagram"),
            commands::DiagramResult {
                content: "graph TD; A-->C".into(),
                diagram_type: "class".into(),
                format,
            },
        );
        assert_eq!(merged[0].diagrams[0].format, "mermaid");
        assert_eq!(merged[0].diagrams[0].content, "graph TD; A-->C");
        assert_eq!(
            diagram_generation_input(&base, None),
            ("plantuml".into(), None)
        );
    }
}
