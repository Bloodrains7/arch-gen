//! Durable local editing authority. Project files remain explicit user saves.
use crate::{
    ai::{self, rework, Ai, ProviderKind},
    commands,
    project::{Diagram, Document, Project, Section},
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::Mutex};
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
    #[serde(rename_all = "camelCase")]
    Rework {
        instruction: String,
        language: String,
        scope: String,
        section_ids: Vec<String>,
        diagram_ids: Vec<String>,
        include_context: bool,
        provider: String,
        #[serde(default)]
        model: String,
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
    #[serde(default)]
    summary: Option<String>,
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
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS sessions(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, project_id TEXT NOT NULL, payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS jobs_project ON jobs(project_id); CREATE TABLE IF NOT EXISTS saved_revisions(project_id TEXT PRIMARY KEY, payload TEXT NOT NULL);").map_err(err)?;
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
/// Project folders carry no revisions, so Git history stays free of counters.
/// The revisions of the last known on-disk content live here instead, which
/// keeps ready AI proposals valid across restarts on this machine.
#[derive(Serialize, Deserialize)]
struct SavedRevisions {
    fingerprint: String,
    revision: u64,
    documents: HashMap<String, u64>,
}
impl Runtime {
    pub(crate) fn remember_saved(&self, project: &Project, fingerprint: &str) -> Result<()> {
        let saved = SavedRevisions {
            fingerprint: fingerprint.into(),
            revision: project.revision,
            documents: project
                .documents
                .iter()
                .map(|d| (d.id.clone(), d.revision))
                .collect(),
        };
        self.transaction(|db| {
            db.execute("INSERT INTO saved_revisions(project_id,payload) VALUES(?1,?2) ON CONFLICT(project_id) DO UPDATE SET payload=excluded.payload", params![project.id, encode(&saved)?]).map_err(err)?;
            Ok(())
        })
    }
    /// Same content as last time continues its revisions. Content changed outside
    /// ArchGen (git pull, another editor) moves every document past anything seen before.
    pub(crate) fn restore_revisions(&self, project: &mut Project, fingerprint: &str) -> Result<()> {
        let saved: Option<SavedRevisions> = self.transaction(|db| {
            let mut query = db
                .prepare("SELECT payload FROM saved_revisions WHERE project_id=?1")
                .map_err(err)?;
            let mut rows = query
                .query_map([&project.id], |r| r.get::<_, String>(0))
                .map_err(err)?;
            rows.next().map(|row| decode(row.map_err(err)?)).transpose()
        })?;
        if let Some(saved) = saved {
            let same = saved.fingerprint == fingerprint;
            project.revision = if same {
                saved.revision
            } else {
                saved.revision.saturating_add(1).min(MAX_REVISION)
            };
            for doc in &mut project.documents {
                doc.revision = match saved.documents.get(&doc.id) {
                    Some(revision) if same => *revision,
                    _ => project.revision,
                };
            }
            project.validate()?;
        }
        self.remember_saved(project, fingerprint)
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
        GenerationRequest::Rework { .. } => {
            return Err("Rework requests are validated when the job is created.".into())
        }
    };
    if description.trim().is_empty() || language.trim().is_empty() {
        return Err("Description and language are required.".into());
    }
    encode(request)?;
    Ok(())
}

/// The provider the UI displayed, resolved once by the command from `ai.route()`.
/// `create_job_as` refuses a Rework request whose `provider` does not match.
pub(crate) struct Configured {
    pub provider: String,
    pub model: String,
}

#[tauri::command]
pub fn create_generation_job(
    state: State<'_, Runtime>,
    ai: State<'_, Ai>,
    session_id: String,
    document_id: String,
    expected_document_revision: u64,
    request: GenerationRequest,
) -> Result<Job> {
    let configured = match &request {
        GenerationRequest::Rework { .. } => {
            let route = ai.route()?;
            Some(Configured { provider: route.kind.id().into(), model: route.model.clone() })
        }
        _ => None,
    };
    create_job_as(
        &state,
        &session_id,
        &document_id,
        expected_document_revision,
        request,
        configured.as_ref(),
    )
}
/// A convenience wrapper kept for tests (production always resolves `Configured` first);
/// gated so it is not flagged as dead code in a non-test build.
#[cfg(test)]
fn create_job(
    state: &Runtime,
    session: &str,
    document: &str,
    expected: u64,
    request: GenerationRequest,
) -> Result<Job> {
    create_job_as(state, session, document, expected, request, None)
}
pub(crate) fn create_job_as(
    state: &Runtime,
    session: &str,
    document: &str,
    expected: u64,
    mut request: GenerationRequest,
    configured: Option<&Configured>,
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
        if let GenerationRequest::Rework {
            instruction,
            language,
            provider,
            model,
            scope,
            section_ids,
            diagram_ids,
            include_context,
        } = &mut request
        {
            let configured = configured.ok_or("AI settings are not available for a rework request.")?;
            if *provider != configured.provider {
                return Err("AI settings changed after this request was created. Create the request again.".into());
            }
            *model = configured.model.clone();
            if !rework::valid_instruction(instruction) {
                return Err(format!(
                    "Enter an instruction between 1 and {} characters.",
                    rework::MAX_INSTRUCTION_CHARS
                ));
            }
            if !rework::valid_language(language) {
                return Err(format!(
                    "Language must be between 1 and {} characters, with no control characters.",
                    rework::MAX_LANGUAGE_CHARS
                ));
            }
            match scope.as_str() {
                "document" => {
                    if !section_ids.is_empty() || !diagram_ids.is_empty() {
                        return Err("A whole-document rework request must not select any blocks.".into());
                    }
                }
                "selection" => {
                    let (normalized_sections, normalized_diagrams) =
                        rework::normalize_targets(doc, section_ids, diagram_ids)?;
                    if normalized_sections.is_empty() && normalized_diagrams.is_empty() {
                        return Err("The selected blocks no longer exist.".into());
                    }
                    *section_ids = normalized_sections;
                    *diagram_ids = normalized_diagrams;
                }
                _ => return Err("Unknown rework scope.".into()),
            }
            let targets = rework::Targets { section_ids, diagram_ids, include_context: *include_context };
            let (editable, total) = rework::prompt_size(doc, &targets);
            if editable > rework::MAX_EDITABLE_CHARS {
                return Err(format!(
                    "The selection is too long for one request ({editable} characters, limit {}). Select fewer blocks.",
                    rework::MAX_EDITABLE_CHARS
                ));
            }
            if total > rework::MAX_TOTAL_CHARS {
                return Err(format!(
                    "The selection is too long for one request ({total} characters, limit {}). Select fewer blocks.",
                    rework::MAX_TOTAL_CHARS
                ));
            }
        } else {
            validate_request(doc, &request)?;
        }
        let job = Job {
            id: id(),
            project_id: project.id.clone(),
            document_id: document.into(),
            base_document: doc.clone(),
            request,
            status: "queued".into(),
            sections: None,
            summary: None,
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
pub fn cancel_generation_job(state: State<'_, Runtime>, ai: State<'_, Ai>, job_id: String) -> Result<Job> {
    cancel_job(&state, &ai, &job_id)
}
pub(crate) fn cancel_job(state: &Runtime, ai: &Ai, job_id: &str) -> Result<Job> {
    // The provider token is cancelled unconditionally: a second ArchGen instance's
    // start-up sweep can flip this job to "interrupted" between the click and this call,
    // and the process behind it must still be stopped, not left uploading for nothing.
    let result = transition(state, job_id, &["queued", "running"], "cancelled");
    ai.cancel(job_id);
    result
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

/// A convenience wrapper kept for tests (production always carries a summary through
/// `finish_outcome` directly); gated so it is not flagged as dead code in a non-test build.
#[cfg(test)]
fn finish(state: &Runtime, job_id: &str, output: Result<Vec<Section>>) -> Result<Job> {
    finish_outcome(state, job_id, output.map(|sections| (sections, None)))
}
pub(crate) fn finish_outcome(
    state: &Runtime,
    job_id: &str,
    output: Result<(Vec<Section>, Option<String>)>,
) -> Result<Job> {
    state.transaction(|db| {
        let mut job = job_at(db, job_id)?;
        if job.status != "running" {
            return Ok(job);
        }
        let output = output.and_then(|(sections, summary)| {
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
            Ok((sections, summary))
        });
        match output {
            Ok((sections, summary)) => {
                job.sections = Some(sections);
                job.summary = summary;
                job.status = "ready".into();
            }
            Err(error) => {
                job.error = Some(error.chars().take(16000).collect());
                job.status = "failed".into();
            }
        }
        if encode(&job).is_err() {
            job.sections = None;
            job.summary = None;
            job.status = "failed".into();
            job.error = Some("Generation exceeds the 32 MiB job content limit.".into());
        }
        put_job(db, &job)?;
        Ok(job)
    })
}

/// `(system, user, schema)` for a Rework job's base document and stored targets.
pub(crate) fn rework_prompt(job: &Job) -> Result<(String, String, Value)> {
    let GenerationRequest::Rework { instruction, language, section_ids, diagram_ids, include_context, .. } =
        &job.request
    else {
        return Err("This job is not a rework request.".into());
    };
    let targets = rework::Targets { section_ids, diagram_ids, include_context: *include_context };
    Ok((
        rework::system_prompt(language),
        rework::user_prompt(&job.base_document, instruction, &targets),
        rework::response_schema(),
    ))
}
/// The provider and model recorded in the job must still resolve identically, or the
/// request is stale: settings changed underneath it since it was created.
pub(crate) fn check_route(provider: &str, model: &str, route: &ai::Route) -> Result<()> {
    if route.kind.id() != provider || route.model != model {
        return Err("AI settings changed after this request was created. Create the request again.".into());
    }
    Ok(())
}
/// Applies the assistant's answer to the job's base document via `rework::merge`.
pub(crate) fn rework_outcome(job: &Job, answer: Result<Value>) -> Result<rework::Outcome> {
    let GenerationRequest::Rework { section_ids, diagram_ids, include_context, .. } = &job.request else {
        return Err("This job is not a rework request.".into());
    };
    let targets = rework::Targets { section_ids, diagram_ids, include_context: *include_context };
    rework::merge(&job.base_document, &targets, &answer?, &mut || id())
}

#[tauri::command]
pub async fn run_generation_job(state: State<'_, Runtime>, ai: State<'_, Ai>, job_id: String) -> Result<Job> {
    let cancel = ai.begin(&job_id);
    let job = match transition(&state, &job_id, &["queued"], "running") {
        Ok(job) => job,
        Err(e) => {
            ai.end(&job_id);
            return Err(e);
        }
    };
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
            (
                doc.sections
                    .into_iter()
                    .map(|s| Section {
                        id: id(),
                        title: s.title,
                        content: s.content,
                        diagrams: s.diagrams.into_iter().map(new_diagram).collect(),
                    })
                    .collect(),
                None,
            )
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
                (
                    merge_diagram(
                        &job.base_document,
                        section_id.as_deref(),
                        diagram_id.as_deref(),
                        result,
                    ),
                    None,
                )
            })
        }
        GenerationRequest::Rework { provider, model, .. } => {
            let prepared = rework_prompt(&job).and_then(|(system, user, schema)| {
                let kind = ProviderKind::parse(provider).ok_or_else(|| {
                    "AI settings changed after this request was created. Create the request again.".to_string()
                })?;
                let route = ai.route_for(kind)?;
                check_route(provider, model, &route)?;
                Ok((system, user, schema, route))
            });
            let job_copy = job.clone();
            let cancel_copy = cancel.clone();
            match prepared {
                Ok((system, user, schema, route)) => tokio::task::spawn_blocking(move || {
                    let answer = ai::complete(
                        &route,
                        &ai::Completion { system: &system, user: &user, schema: &schema, max_output_tokens: 16_000, probe: false },
                        &cancel_copy,
                    );
                    rework_outcome(&job_copy, answer)
                })
                .await
                .unwrap_or_else(|e| Err(format!("The assistant request stopped unexpectedly: {e}"))),
                Err(e) => Err(e),
            }
            .map(|outcome| (outcome.sections, Some(outcome.summary)))
        }
    };
    ai.end(&job_id);
    finish_outcome(&state, &job_id, output)
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
    fn saved_revisions_continue_for_same_content_and_jump_for_foreign_changes() {
        let state = memory();
        let mut fresh = fixture();
        state.restore_revisions(&mut fresh, "a").unwrap();
        assert_eq!((fresh.revision, fresh.documents[0].revision), (0, 0));
        let mut saved = fixture();
        saved.revision = 7;
        saved.documents[0].revision = 5;
        state.remember_saved(&saved, "b").unwrap();
        let mut reopened = fixture();
        state.restore_revisions(&mut reopened, "b").unwrap();
        assert_eq!((reopened.revision, reopened.documents[0].revision), (7, 5));
        let mut pulled = fixture();
        state.restore_revisions(&mut pulled, "c").unwrap();
        assert_eq!((pulled.revision, pulled.documents[0].revision), (8, 8));
        let mut again = fixture();
        state.restore_revisions(&mut again, "c").unwrap();
        assert_eq!((again.revision, again.documents[0].revision), (8, 8));
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

    // ── Rework ───────────────────────────────────────────────────────────────

    fn rework_request(
        scope: &str,
        section_ids: Vec<String>,
        diagram_ids: Vec<String>,
        include_context: bool,
        provider: &str,
    ) -> GenerationRequest {
        GenerationRequest::Rework {
            instruction: "Rewrite it more formally.".into(),
            language: "sk".into(),
            scope: scope.into(),
            section_ids,
            diagram_ids,
            include_context,
            provider: provider.into(),
            model: String::new(),
        }
    }
    /// `fixture()` plus a second, untargeted section so tests can tell a targeted
    /// change apart from something that must never be touched.
    fn two_section_fixture() -> Project {
        let mut project = fixture();
        project.documents[0].sections.push(Section {
            id: "other".into(),
            title: "Untouched".into(),
            content: "Leave me alone".into(),
            diagrams: vec![],
        });
        project
    }
    fn configured(provider: &str, model: &str) -> Configured {
        Configured { provider: provider.into(), model: model.into() }
    }
    fn fake_route(kind: ProviderKind, model: &str) -> ai::Route {
        ai::Route {
            kind,
            model: model.into(),
            api_key: None,
            ollama_url: String::new(),
            timeout: std::time::Duration::from_secs(1),
        }
    }

    #[test]
    fn rework_request_matches_the_wire_fixture_and_rejects_snake_case() {
        let text = include_str!("../../tests/fixtures/rework-request.json");
        let request: GenerationRequest = serde_json::from_str(text).unwrap();
        match &request {
            GenerationRequest::Rework {
                instruction,
                language,
                scope,
                section_ids,
                diagram_ids,
                include_context,
                provider,
                model,
            } => {
                assert_eq!(instruction, "Prepíš formálnejšie.");
                assert_eq!(language, "sk");
                assert_eq!(scope, "selection");
                assert_eq!(section_ids, &vec!["section-a".to_string(), "section-b".to_string()]);
                assert_eq!(diagram_ids, &vec!["diagram-c".to_string()]);
                assert!(!include_context);
                assert_eq!(provider, "claude_cli");
                assert_eq!(model, "");
            }
            other => panic!("expected a rework request, got {other:?}"),
        }
        let roundtrip = serde_json::to_value(&request).unwrap();
        let original: Value = serde_json::from_str(text).unwrap();
        assert_eq!(roundtrip, original);

        let snake_case = text.replace("sectionIds", "section_ids");
        assert!(serde_json::from_str::<GenerationRequest>(&snake_case).is_err());
    }

    #[test]
    fn rework_request_fields_with_no_serde_default_are_required() {
        // Contract: sectionIds, diagramIds, includeContext (and scope) are "required, NO
        // serde default" — so adding one back would silently widen what a client may omit.
        let text = include_str!("../../tests/fixtures/rework-request.json");
        let fixture: Value = serde_json::from_str(text).unwrap();
        for key in ["sectionIds", "diagramIds", "includeContext", "scope"] {
            let mut without = fixture.clone();
            without.as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<GenerationRequest>(without).is_err(),
                "{key} must be required"
            );
        }
        // `model` is the one field Rust always overwrites at creation, so it alone may be omitted.
        let mut without_model = fixture;
        without_model.as_object_mut().unwrap().remove("model");
        assert!(serde_json::from_value::<GenerationRequest>(without_model).is_ok());
    }

    #[test]
    fn job_row_without_a_summary_field_loads_as_none() {
        let old = serde_json::json!({
            "id": "j1", "projectId": "p", "documentId": "d",
            "baseDocument": {"id": "d", "name": "Doc", "template": "arc42", "language": "en", "revision": 0, "sections": []},
            "request": {"kind": "documentation", "description": "x", "template": "arc42", "language": "en"},
            "status": "ready", "sections": null, "error": null, "createdAt": "now",
        });
        let job: Job = serde_json::from_value(old).unwrap();
        assert!(job.summary.is_none());
    }

    #[test]
    fn rework_requires_a_configured_provider_at_creation() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec![], vec![], false, "ollama");
        assert!(create_job_as(&state, &session.session_id, "doc", 0, request, None).is_err());
    }

    #[test]
    fn a_provider_other_than_the_configured_one_is_refused_at_creation() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec![], vec![], false, "openai");
        let error =
            create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                .unwrap_err();
        assert!(error.contains("changed after this request was created"), "{error}");
    }

    #[test]
    fn document_scope_with_selected_blocks_is_refused() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec!["section".into()], vec![], false, "ollama");
        assert!(
            create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                .is_err()
        );
    }

    #[test]
    fn selection_scope_with_no_valid_target_is_refused() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("selection", vec!["gone".into()], vec![], false, "ollama");
        let error =
            create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                .unwrap_err();
        assert_eq!(error, "The selected blocks no longer exist.");
    }

    #[test]
    fn a_blank_or_oversized_instruction_is_refused_at_creation() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        for instruction in ["", "   ", &"x".repeat(rework::MAX_INSTRUCTION_CHARS + 1)] {
            let request = GenerationRequest::Rework {
                instruction: instruction.into(),
                language: "sk".into(),
                scope: "document".into(),
                section_ids: vec![],
                diagram_ids: vec![],
                include_context: false,
                provider: "ollama".into(),
                model: String::new(),
            };
            let error =
                create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                    .unwrap_err();
            assert!(error.contains("instruction"), "{error}");
        }
    }

    #[test]
    fn a_blank_oversized_or_control_character_language_is_refused_at_creation() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        for language in ["", &"x".repeat(rework::MAX_LANGUAGE_CHARS + 1), "sk\n"] {
            let request = GenerationRequest::Rework {
                instruction: "Rewrite it more formally.".into(),
                language: language.into(),
                scope: "document".into(),
                section_ids: vec![],
                diagram_ids: vec![],
                include_context: false,
                provider: "ollama".into(),
                model: String::new(),
            };
            let error =
                create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                    .unwrap_err();
            assert!(error.contains("Language"), "{error}");
        }
    }

    #[test]
    fn an_unknown_scope_is_refused() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("everything", vec![], vec![], false, "ollama");
        assert!(
            create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                .is_err()
        );
    }

    #[test]
    fn selection_scope_stores_the_normalized_targets_and_the_configured_model() {
        let state = memory();
        let session = open_session(&state, two_section_fixture()).unwrap();
        let request = rework_request("selection", vec!["section".into(), "gone".into()], vec![], false, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "qwen")))
            .unwrap();
        match job.request {
            GenerationRequest::Rework { section_ids, model, .. } => {
                assert_eq!(section_ids, vec!["section".to_string()]);
                assert_eq!(model, "qwen");
            }
            other => panic!("expected a rework request, got {other:?}"),
        }
    }

    fn job_count(state: &Runtime) -> i64 {
        state
            .transaction(|db| db.query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0)).map_err(err))
            .unwrap()
    }

    #[test]
    fn an_oversized_selection_is_refused_before_a_job_is_created() {
        let state = memory();
        let mut project = fixture();
        project.documents[0].sections[0].content = "x".repeat(rework::MAX_EDITABLE_CHARS + 1);
        let session = open_session(&state, project).unwrap();
        let request = rework_request("selection", vec!["section".into()], vec![], false, "ollama");
        let error =
            create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                .unwrap_err();
        assert!(error.contains("too long for one request"), "{error}");
        assert_eq!(job_count(&state), 0, "no job row was written for a refused request");
    }

    #[test]
    fn a_selection_within_the_editable_limit_but_over_the_total_limit_with_context_is_refused_before_a_job_is_created() {
        let state = memory();
        let mut project = two_section_fixture();
        // "section" (the target) stays small; "other" (sent as read-only context) alone
        // pushes the total over the limit while the editable half stays well under it.
        project.documents[0].sections[1].content = "x".repeat(rework::MAX_TOTAL_CHARS);
        let session = open_session(&state, project).unwrap();
        let request = rework_request("selection", vec!["section".into()], vec![], true, "ollama");
        let error =
            create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
                .unwrap_err();
        assert!(error.contains("too long for one request"), "{error}");
        assert!(error.contains(&rework::MAX_TOTAL_CHARS.to_string()), "{error}");
        assert_eq!(job_count(&state), 0, "no job row was written for a refused request");
    }

    #[test]
    fn check_route_accepts_a_matching_provider_and_model_and_rejects_a_mismatch() {
        let matching = fake_route(ProviderKind::Ollama, "m1");
        assert!(check_route("ollama", "m1", &matching).is_ok());
        let mismatched_model = fake_route(ProviderKind::Ollama, "m2");
        assert!(check_route("ollama", "m1", &mismatched_model).is_err());
        let mismatched_provider = fake_route(ProviderKind::Openai, "m1");
        assert!(check_route("ollama", "m1", &mismatched_provider).is_err());
    }

    #[test]
    fn check_route_fails_the_job_before_any_provider_call() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec![], vec![], false, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m1")))
            .unwrap();
        let job = transition(&state, &job.id, &["queued"], "running").unwrap();
        let mut provider_calls = 0;
        let prepared = rework_prompt(&job).and_then(|(system, user, schema)| {
            let route = fake_route(ProviderKind::Ollama, "a-different-model");
            check_route("ollama", "m1", &route)?;
            provider_calls += 1;
            Ok((system, user, schema, route))
        });
        assert!(prepared.is_err());
        assert!(prepared.unwrap_err().contains("changed after this request was created"));
        assert_eq!(provider_calls, 0);
    }

    #[test]
    fn cancel_job_stops_the_registered_token_and_marks_the_job_cancelled() {
        let state = memory();
        let ai = Ai::without_sweep(Path::new("unused"));
        let session = open_session(&state, fixture()).unwrap();
        let job = create_job(&state, &session.session_id, "doc", 0, request()).unwrap();
        let cancel = ai.begin(&job.id);
        let cancelled = cancel_job(&state, &ai, &job.id).unwrap();
        assert_eq!(cancelled.status, "cancelled");
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn cancel_job_cancels_the_token_even_when_the_status_transition_fails() {
        // A second ArchGen instance's start-up sweep can flip a live job to "interrupted"
        // before the user's Cancel click reaches this process: the provider must still stop.
        let state = memory();
        let ai = Ai::without_sweep(Path::new("unused"));
        let session = open_session(&state, fixture()).unwrap();
        let job = create_job(&state, &session.session_id, "doc", 0, request()).unwrap();
        let cancel = ai.begin(&job.id);
        transition(&state, &job.id, &["queued"], "interrupted").unwrap();
        assert!(cancel_job(&state, &ai, &job.id).is_err());
        assert!(cancel.is_cancelled(), "the provider token is still cancelled");
    }

    #[test]
    fn a_cancel_before_complete_means_the_provider_closure_is_never_called() {
        let state = memory();
        let ai = Ai::without_sweep(Path::new("unused"));
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec![], vec![], false, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
            .unwrap();
        let cancel = ai.begin(&job.id);
        cancel_job(&state, &ai, &job.id).unwrap();
        let route = fake_route(ProviderKind::Ollama, "m");
        let schema = serde_json::json!({"type": "object"});
        let completion = ai::Completion { system: "", user: "", schema: &schema, max_output_tokens: 1, probe: false };
        assert_eq!(ai::complete(&route, &completion, &cancel).unwrap_err(), ai::process::CANCELLED);
    }

    #[test]
    fn whole_path_prompts_merges_and_can_be_accepted_without_tauri_state() {
        let state = memory();
        let session = open_session(&state, two_section_fixture()).unwrap();
        let request = rework_request("selection", vec!["section".into()], vec![], false, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
            .unwrap();
        let job = transition(&state, &job.id, &["queued"], "running").unwrap();

        let (_system, user, _schema) = rework_prompt(&job).unwrap();
        assert!(user.contains("Original"), "the targeted section is included");
        assert!(!user.contains("Untouched") && !user.contains("Leave me alone"), "the other section is not sent");

        let answer = Ok(serde_json::json!({
            "summary": "Made it more formal.",
            "sections": [{"id": "section", "title": "Title", "content": "Reworded", "diagrams": [
                {"id": "diagram", "diagram_type": "class", "format": "plantuml", "content": "old"},
            ]}],
            "diagrams": [],
        }));
        let outcome = rework_outcome(&job, answer).unwrap();
        assert!(outcome.summary.contains("Made it more formal."));
        let finished = finish_outcome(&state, &job.id, Ok((outcome.sections, Some(outcome.summary)))).unwrap();
        assert_eq!(finished.status, "ready");
        assert!(finished.summary.as_deref().is_some_and(|s| s.contains("Made it more formal.")));

        let project = consume(&state, &session.session_id, &finished.id, 0, false).unwrap();
        assert_eq!(project.documents[0].sections[0].content, "Reworded");
        assert_eq!(project.documents[0].sections[1].content, "Leave me alone");
    }

    #[test]
    fn a_diagram_target_with_context_does_not_duplicate_the_target_diagram_as_read_only() {
        let state = memory();
        let session = open_session(&state, two_section_fixture()).unwrap();
        let request = rework_request("selection", vec![], vec!["diagram".into()], true, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
            .unwrap();
        let (_system, user, _schema) = rework_prompt(&job).unwrap();
        assert_eq!(
            user.matches("\"id\": \"diagram\"").count(),
            1,
            "the targeted diagram is sent once, not also as read-only context"
        );
        assert!(user.contains("readOnlyContext") && user.contains("Leave me alone"), "the rest of the document is still sent as context");
    }

    #[test]
    fn a_failed_answer_never_reaches_merge_and_fails_the_job() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec![], vec![], false, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
            .unwrap();
        let job = transition(&state, &job.id, &["queued"], "running").unwrap();
        let outcome = rework_outcome(&job, Err("Ollama is not running at http://localhost:11434.".into()));
        assert!(outcome.is_err());
        let finished =
            finish_outcome(&state, &job.id, outcome.map(|o| (o.sections, Some(o.summary)))).unwrap();
        assert_eq!(finished.status, "failed");
        assert!(finished.error.unwrap().contains("not running"));
    }

    #[test]
    fn a_cancelled_rework_job_is_left_untouched_by_a_late_answer() {
        let state = memory();
        let session = open_session(&state, fixture()).unwrap();
        let request = rework_request("document", vec![], vec![], false, "ollama");
        let job = create_job_as(&state, &session.session_id, "doc", 0, request, Some(&configured("ollama", "m")))
            .unwrap();
        let job = transition(&state, &job.id, &["queued"], "running").unwrap();
        transition(&state, &job.id, &["running"], "cancelled").unwrap();
        let outcome = rework_outcome(&job, Err(ai::process::CANCELLED.into()));
        assert!(outcome.is_err());
        let late = finish_outcome(&state, &job.id, outcome.map(|o| (o.sections, Some(o.summary)))).unwrap();
        assert_eq!(late.status, "cancelled");
        assert!(late.error.is_none());
    }
}
