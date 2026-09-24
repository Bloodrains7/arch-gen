//! Projects are plain files in a folder, meant to live in a Git repository:
//!
//! ```text
//! <project>/archgen.json                      id, name, ordered document folders
//! <project>/documents/<doc>/document.json     id, name, template, language, ordered sections
//! <project>/documents/<doc>/<section>.md      "# Title" + Markdown content
//! <project>/documents/<doc>/<section>.<type>.puml   diagram source
//! ```
//!
//! Revisions and the active tab are session state and never reach the disk, so
//! a save only touches files whose content changed. Saved history is Git history.
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
const MAX_REVISION: u64 = 9_007_199_254_740_991;
const MANIFEST: &str = "archgen.json";
const DOCUMENTS: &str = "documents";
const DOCUMENT_MANIFEST: &str = "document.json";
const FORMAT: &str = "archgen-project";

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

// ── On-disk manifests ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct ProjectManifest {
    format: String,
    version: u32,
    id: String,
    name: String,
    documents: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct DocumentManifest {
    id: String,
    name: String,
    template: String,
    language: String,
    #[serde(default)]
    sections: Vec<SectionEntry>,
}

/// IDs are optional on disk so a person or an agent can add a section by hand;
/// a missing ID is minted on load and written back by the next save.
#[derive(Serialize, Deserialize)]
struct SectionEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    file: String,
    /// Only present when the `# heading` of the file cannot carry the exact title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    diagrams: Vec<DiagramEntry>,
}

#[derive(Serialize, Deserialize)]
struct DiagramEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type")]
    diagram_type: String,
    format: String,
    file: String,
}

// ── Text and names ───────────────────────────────────────────────────────────

/// Git may check files out with CRLF; content is LF everywhere inside ArchGen.
fn normalize(text: &str) -> String {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
}

fn strip_newline(text: &str) -> String {
    text.strip_suffix('\n').unwrap_or(text).to_string()
}

fn heading(title: &str) -> String {
    let flat = title.split(['\r', '\n']).collect::<Vec<_>>().join(" ");
    let flat = flat.trim();
    if flat.is_empty() {
        "(untitled)".into()
    } else {
        flat.into()
    }
}

fn section_text(title: &str, content: &str) -> String {
    let content = normalize(content);
    if content.is_empty() {
        format!("# {}\n", heading(title))
    } else {
        format!("# {}\n\n{content}\n", heading(title))
    }
}

fn parse_section(text: &str) -> (Option<String>, String) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    match first.strip_prefix("# ") {
        Some(title) => (
            Some(title.trim().to_string()),
            strip_newline(rest.strip_prefix('\n').unwrap_or(rest)),
        ),
        None => (None, strip_newline(text)),
    }
}

fn fold(c: char) -> &'static str {
    match c {
        'á' | 'ä' | 'â' | 'à' | 'ã' | 'å' | 'ą' => "a",
        'č' | 'ć' | 'ç' => "c",
        'ď' | 'đ' => "d",
        'é' | 'ě' | 'ë' | 'è' | 'ê' | 'ę' => "e",
        'í' | 'ì' | 'î' | 'ï' => "i",
        'ľ' | 'ĺ' | 'ł' => "l",
        'ň' | 'ń' | 'ñ' => "n",
        'ó' | 'ô' | 'ö' | 'ò' | 'õ' | 'ő' | 'ø' => "o",
        'ŕ' | 'ř' => "r",
        'š' | 'ś' => "s",
        'ß' => "ss",
        'ť' => "t",
        'ú' | 'ů' | 'ü' | 'ù' | 'û' | 'ű' => "u",
        'ý' | 'ÿ' => "y",
        'ž' | 'ź' | 'ż' => "z",
        _ => "-",
    }
}

/// ASCII, lowercase and short: identical on every filesystem and in every Git client.
pub(crate) fn slug(text: &str, fallback: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            out.push_str(fold(c));
        }
    }
    let mut compact = String::new();
    for c in out.chars() {
        if c != '-' || !compact.ends_with('-') {
            compact.push(c);
        }
    }
    let compact: String = compact.trim_matches('-').chars().take(60).collect();
    let compact = compact.trim_matches('-').to_string();
    let reserved = matches!(compact.as_str(), "con" | "prn" | "aux" | "nul")
        || (compact.len() == 4
            && (compact.starts_with("com") || compact.starts_with("lpt"))
            && compact.ends_with(|c: char| c.is_ascii_digit()));
    if compact.is_empty() {
        fallback.into()
    } else if reserved {
        format!("{compact}-{fallback}")
    } else {
        compact
    }
}

fn unique(used: &mut HashSet<String>, stem: &str, suffix: &str) -> String {
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}{suffix}")
        } else {
            format!("{stem}-{n}{suffix}")
        };
        if used.insert(name.clone()) {
            return name;
        }
        n += 1;
    }
}

fn extension(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "plantuml" | "puml" => "puml",
        "mermaid" => "mmd",
        _ => "txt",
    }
}

/// `CON.md`, `nul.txt`, `COM1.puml`: on Windows these open a device, not a file,
/// whatever the extension. Generated names never are one (see `slug`).
fn windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// Manifests come from cloned repositories: a name is one ordinary path component.
pub(crate) fn safe_name(name: &str) -> Result<&str, String> {
    let unsafe_name = name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 200
        || name.starts_with(' ')
        || name.ends_with([' ', '.'])
        // Only refused where it is a device: elsewhere such a hand-written file still opens.
        || (cfg!(windows) && windows_device_name(name))
        || name
            .chars()
            .any(|c| c.is_control() || r#"/\:*?"<>|"#.contains(c));
    if unsafe_name {
        Err(format!("Unsafe file name in the project manifest: {name:?}"))
    } else {
        Ok(name)
    }
}

// ── Reading ──────────────────────────────────────────────────────────────────

trait Source {
    /// `relative` uses `/`. `None` means the file does not exist.
    fn read(&mut self, relative: &str, limit: usize) -> Result<Option<Vec<u8>>, String>;
}

struct Folder<'a>(&'a Path);

impl Source for Folder<'_> {
    fn read(&mut self, relative: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let path = self.0.join(relative);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > limit as u64 => {
                Err("Project exceeds the 32 MiB content limit.".into())
            }
            Ok(_) => std::fs::read(&path)
                .map(Some)
                .map_err(|e| format!("{relative}: {e}")),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{relative}: {e}")),
        }
    }
}

pub(crate) fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["-c", "core.quotepath=false"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

pub(crate) fn git_error(error: impl std::fmt::Display) -> String {
    format!("Git is needed for project history and could not be started: {error}")
}

/// One `git cat-file --batch` process answers every read of a historical snapshot.
struct Commit {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    commit: String,
}

impl Commit {
    fn open(root: &Path, commit: &str) -> Result<Self, String> {
        if !(7..=64).contains(&commit.len()) || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid commit identifier.".into());
        }
        let mut child = git(root)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(git_error)?;
        Ok(Self {
            stdin: child.stdin.take().ok_or("Git input is unavailable.")?,
            stdout: BufReader::new(child.stdout.take().ok_or("Git output is unavailable.")?),
            child,
            commit: commit.into(),
        })
    }
}

impl Drop for Commit {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Source for Commit {
    fn read(&mut self, relative: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
        // `<commit>:./path` resolves against the project folder, wherever it sits in the repository.
        writeln!(self.stdin, "{}:./{relative}", self.commit).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let mut header = String::new();
        self.stdout
            .read_line(&mut header)
            .map_err(|e| e.to_string())?;
        let header = header.trim_end();
        if header.is_empty() {
            return Err("Git could not read this commit.".into());
        }
        if header.ends_with(" missing") {
            return Ok(None);
        }
        let mut parts = header.rsplitn(3, ' ');
        let size: usize = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or("Unexpected Git answer.")?;
        if parts.next() != Some("blob") {
            return Err(format!("{relative} is not a file in this commit."));
        }
        if size > limit {
            return Err("Project exceeds the 32 MiB content limit.".into());
        }
        let mut bytes = vec![0; size + 1];
        self.stdout
            .read_exact(&mut bytes)
            .map_err(|e| e.to_string())?;
        bytes.pop();
        Ok(Some(bytes))
    }
}

struct Reader<S: Source> {
    source: S,
    budget: usize,
    hasher: DefaultHasher,
    files: Vec<String>,
}

impl<S: Source> Reader<S> {
    fn text(&mut self, relative: &str) -> Result<Option<String>, String> {
        let Some(bytes) = self.source.read(relative, self.budget)? else {
            return Ok(None);
        };
        self.budget -= bytes.len();
        let text = normalize(
            &String::from_utf8(bytes).map_err(|_| format!("{relative} is not UTF-8 text."))?,
        );
        relative.hash(&mut self.hasher);
        text.hash(&mut self.hasher);
        self.files.push(relative.into());
        Ok(Some(text))
    }
    fn required(&mut self, relative: &str) -> Result<String, String> {
        self.text(relative)?
            .ok_or_else(|| format!("{relative} is missing from the project."))
    }
}

#[derive(Debug)]
struct Loaded {
    project: Project,
    /// Identifies the exact on-disk content, independent of line endings.
    fingerprint: String,
    files: Vec<String>,
}

fn read_manifest<S: Source>(reader: &mut Reader<S>) -> Result<ProjectManifest, String> {
    let text = reader.text(MANIFEST)?.ok_or(
        "This folder is not an ArchGen project: archgen.json is missing. Nothing was changed.",
    )?;
    let manifest: ProjectManifest =
        serde_json::from_str(&text).map_err(|e| format!("{MANIFEST}: {e}"))?;
    if manifest.format != FORMAT || manifest.version != 1 {
        return Err("Unsupported project format. Update ArchGen to open this project.".into());
    }
    Ok(manifest)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn load_from<S: Source>(source: S) -> Result<Loaded, String> {
    let mut reader = Reader {
        source,
        budget: MAX_PAYLOAD,
        hasher: DefaultHasher::new(),
        files: Vec::new(),
    };
    let manifest = read_manifest(&mut reader)?;
    let mut documents = Vec::new();
    for folder in &manifest.documents {
        let base = format!("{DOCUMENTS}/{}", safe_name(folder)?);
        let relative = format!("{base}/{DOCUMENT_MANIFEST}");
        let doc: DocumentManifest = serde_json::from_str(&reader.required(&relative)?)
            .map_err(|e| format!("{relative}: {e}"))?;
        let mut sections = Vec::new();
        for entry in doc.sections {
            let file = safe_name(&entry.file)?;
            let (heading, content) = parse_section(&reader.required(&format!("{base}/{file}"))?);
            let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
            let mut diagrams = Vec::new();
            for diagram in entry.diagrams {
                let source = reader.required(&format!("{base}/{}", safe_name(&diagram.file)?))?;
                diagrams.push(Diagram {
                    id: diagram.id.unwrap_or_else(new_id),
                    content: strip_newline(&source),
                    diagram_type: diagram.diagram_type,
                    format: diagram.format,
                });
            }
            sections.push(Section {
                id: entry.id.unwrap_or_else(new_id),
                title: entry.title.or(heading).unwrap_or_else(|| stem.to_string()),
                content,
                diagrams,
            });
        }
        documents.push(Document {
            id: doc.id,
            name: doc.name,
            template: doc.template,
            language: doc.language,
            revision: 0,
            sections,
        });
    }
    let project = Project {
        schema_version: 1,
        id: manifest.id,
        name: manifest.name,
        revision: 0,
        active_document_id: documents.first().map(|d| d.id.clone()).unwrap_or_default(),
        documents,
    };
    project.validate()?;
    Ok(Loaded {
        project,
        fingerprint: format!("{:016x}", reader.hasher.finish()),
        files: reader.files,
    })
}

// ── Writing ──────────────────────────────────────────────────────────────────

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    Ok(serde_json::to_string_pretty(value).map_err(|e| e.to_string())? + "\n")
}

/// Every project file in write order: content first, the manifests that point at it last.
fn plan(project: &Project) -> Result<Vec<(String, String)>, String> {
    let mut content = Vec::new();
    let mut manifests = Vec::new();
    let mut folders = HashSet::new();
    let mut listed = Vec::new();
    for doc in &project.documents {
        let folder = unique(&mut folders, &slug(&doc.name, "document"), "");
        let base = format!("{DOCUMENTS}/{folder}");
        let mut names = HashSet::from([DOCUMENT_MANIFEST.to_string()]);
        let mut sections = Vec::new();
        for section in &doc.sections {
            let file = unique(&mut names, &slug(&section.title, "section"), ".md");
            let stem = file.trim_end_matches(".md").to_string();
            let mut diagrams = Vec::new();
            for diagram in &section.diagrams {
                let name = unique(
                    &mut names,
                    &format!("{stem}.{}", slug(&diagram.diagram_type, "diagram")),
                    &format!(".{}", extension(&diagram.format)),
                );
                content.push((
                    format!("{base}/{name}"),
                    normalize(&diagram.content) + "\n",
                ));
                diagrams.push(DiagramEntry {
                    id: Some(diagram.id.clone()),
                    diagram_type: diagram.diagram_type.clone(),
                    format: diagram.format.clone(),
                    file: name,
                });
            }
            content.push((
                format!("{base}/{file}"),
                section_text(&section.title, &section.content),
            ));
            sections.push(SectionEntry {
                id: Some(section.id.clone()),
                file,
                title: (heading(&section.title) != section.title).then(|| section.title.clone()),
                diagrams,
            });
        }
        manifests.push((
            format!("{base}/{DOCUMENT_MANIFEST}"),
            json(&DocumentManifest {
                id: doc.id.clone(),
                name: doc.name.clone(),
                template: doc.template.clone(),
                language: doc.language.clone(),
                sections,
            })?,
        ));
        listed.push(folder);
    }
    manifests.push((
        MANIFEST.into(),
        json(&ProjectManifest {
            format: FORMAT.into(),
            version: 1,
            id: project.id.clone(),
            name: project.name.clone(),
            documents: listed,
        })?,
    ));
    content.extend(manifests);
    Ok(content)
}

pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid project path.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut name = path.file_name().ok_or("Invalid project path.")?.to_owned();
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = parent.join(name);
    let result = std::fs::write(&temp, text).and_then(|_| std::fs::rename(&temp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|e| format!("{}: {e}", path.display()))
}

/// `expected` is the fingerprint of the last load or save; `None` creates a new project folder.
fn save(root: &Path, project: &Project, expected: Option<&str>) -> Result<String, String> {
    project.validate()?;
    let files = plan(project)?;
    if files.iter().map(|(_, text)| text.len()).sum::<usize>() > MAX_PAYLOAD {
        return Err("Project exceeds the 32 MiB content limit.".into());
    }
    let owned: HashSet<String> = match expected {
        // Save As never adopts or overwrites an existing project.
        None => {
            if root.join(MANIFEST).exists() || root.join(DOCUMENTS).exists() {
                return Err("This folder already contains archgen.json or a documents folder. Choose another folder.".into());
            }
            HashSet::new()
        }
        Some(expected) => {
            let current = load_from(Folder(root))?;
            if current.project.id != project.id || current.fingerprint != expected {
                return Err("Project changed on disk (another editor, git pull or checkout). Open it again, or use Save as to keep your version in a new folder.".into());
            }
            current.files.into_iter().collect()
        }
    };
    let owned_lower: HashSet<String> = owned.iter().map(|f| f.to_lowercase()).collect();
    let mut changed = Vec::new();
    for (relative, text) in &files {
        match std::fs::read(root.join(relative)) {
            Ok(_) if !owned_lower.contains(&relative.to_lowercase()) => {
                return Err(format!(
                    "{relative} already exists and does not belong to this project. Nothing was written."
                ));
            }
            Ok(existing) if std::str::from_utf8(&existing).is_ok_and(|e| normalize(e) == *text) => {}
            Ok(_) => changed.push((relative, text)),
            Err(e) if e.kind() == ErrorKind::NotFound => changed.push((relative, text)),
            Err(e) => return Err(format!("{relative}: {e}")),
        }
    }
    for (relative, text) in changed {
        write_atomic(&root.join(relative), text)?;
    }
    // Only files the previous manifests named are ever deleted; case-insensitive
    // so a renamed folder on Windows does not delete what was just written.
    let planned: HashSet<String> = files.iter().map(|(f, _)| f.to_lowercase()).collect();
    for stale in owned.iter().filter(|f| !planned.contains(&f.to_lowercase())) {
        let path = root.join(stale);
        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent); // succeeds only when empty
        }
    }
    Ok(load_from(Folder(root))?.fingerprint)
}

/// Writes a new project folder for other modules' tests and returns its fingerprint.
#[cfg(test)]
pub(crate) fn save_for_tests(root: &Path, project: &Project) -> String {
    save(root, project, None).unwrap()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedProject {
    project: Project,
    fingerprint: String,
}

#[tauri::command]
pub async fn load_project(
    state: tauri::State<'_, crate::runtime::Runtime>,
    path: String,
) -> Result<LoadedProject, String> {
    let loaded = tokio::task::spawn_blocking(move || load_from(Folder(Path::new(&path))))
        .await
        .map_err(|e| e.to_string())??;
    let mut project = loaded.project;
    state.restore_revisions(&mut project, &loaded.fingerprint)?;
    Ok(LoadedProject {
        project,
        fingerprint: loaded.fingerprint,
    })
}

/// Returns the fingerprint to pass as `expected_fingerprint` of the next save.
#[tauri::command]
pub async fn save_project(
    state: tauri::State<'_, crate::runtime::Runtime>,
    path: String,
    project: Project,
    expected_fingerprint: Option<String>,
) -> Result<String, String> {
    let saved = project.clone();
    let fingerprint = tokio::task::spawn_blocking(move || {
        save(Path::new(&path), &project, expected_fingerprint.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;
    state.remember_saved(&saved, &fingerprint)?;
    Ok(fingerprint)
}

// ── History (Git) ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    commit: String,
    saved_at: String,
    summary: String,
}

pub(crate) fn ensure_project(root: &Path, project_id: &str) -> Result<(), String> {
    let mut reader = Reader {
        source: Folder(root),
        budget: MAX_PAYLOAD,
        hasher: DefaultHasher::new(),
        files: Vec::new(),
    };
    if read_manifest(&mut reader)?.id != project_id {
        return Err("This history belongs to another project.".into());
    }
    Ok(())
}

/// The fingerprint of what is on disk now, for a check that the folder still
/// holds exactly the content the user last opened or saved (see `git::commit`).
pub(crate) fn disk_fingerprint(root: &Path, project_id: &str) -> Result<String, String> {
    let loaded = load_from(Folder(root))?;
    if loaded.project.id != project_id {
        return Err("This folder holds another project.".into());
    }
    Ok(loaded.fingerprint)
}

fn history(root: &Path, project_id: &str, skip: usize) -> Result<Vec<HistoryEntry>, String> {
    ensure_project(root, project_id)?;
    let output = git(root)
        .args(["log", &format!("--skip={skip}"), "-n", "50"])
        .args(["--format=%H%x1f%cI%x1f%s", "--", "."])
        .stdin(Stdio::null())
        .output()
        .map_err(git_error)?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        if error.contains("does not have any commits") {
            return Ok(Vec::new());
        }
        if error.contains("not a git repository") {
            return Err("This project folder is not in a Git repository. Run git init and commit the folder to get history.".into());
        }
        return Err(format!("Git history failed: {}", error.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\u{1f}');
            Some(HistoryEntry {
                commit: parts.next()?.into(),
                saved_at: parts.next()?.into(),
                summary: parts.next()?.into(),
            })
        })
        .collect())
}

fn historical_project(root: &Path, project_id: &str, commit: &str) -> Result<Project, String> {
    ensure_project(root, project_id)?;
    let project = load_from(Commit::open(root, commit)?)?.project;
    if project.id != project_id {
        return Err("This commit holds another project.".into());
    }
    Ok(project)
}

#[tauri::command]
pub async fn list_project_history(
    path: String,
    project_id: String,
    skip: Option<usize>,
) -> Result<Vec<HistoryEntry>, String> {
    tokio::task::spawn_blocking(move || {
        history(Path::new(&path), &project_id, skip.unwrap_or(0))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn load_project_revision(
    path: String,
    project_id: String,
    commit: String,
) -> Result<Project, String> {
    tokio::task::spawn_blocking(move || historical_project(Path::new(&path), &project_id, &commit))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Project {
        serde_json::from_value(serde_json::json!({"schemaVersion":1,"id":"project","name":"Projekt","revision":3,"activeDocumentId":"doc","documents":[{"id":"doc","name":"Dokument","template":"arc42","language":"sk","revision":2,"sections":[
            {"id":"section","title":"Schéma údajov","content":"Žiadne stratené zmeny\n\n# Nadpis v obsahu","diagrams":[{"id":"diagram","content":"@startuml\nA -> B\n@enduml","diagram_type":"class","format":"plantuml"},{"id":"diagram2","content":"graph TD","diagram_type":"class","format":"mermaid"}]},
            {"id":"empty","title":"Schéma údajov","content":"","diagrams":[]},
            {"id":"odd","title":"  two\nlines ","content":"\nleading and trailing\n","diagrams":[]}]}]})).unwrap()
    }

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "archgen-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Revisions and the active tab are session state; everything else must survive.
    fn content(project: &Project) -> Project {
        let mut project = project.clone();
        project.revision = 0;
        project.active_document_id = project.documents[0].id.clone();
        for doc in &mut project.documents {
            doc.revision = 0;
        }
        project
    }

    #[test]
    fn roundtrip_is_lossless_and_readable() {
        let dir = TempDir::new();
        let project = fixture();
        save(&dir.0, &project, None).unwrap();
        assert_eq!(load_from(Folder(&dir.0)).unwrap().project, content(&project));
        let base = dir.0.join("documents/dokument");
        assert_eq!(
            std::fs::read_to_string(base.join("schema-udajov.md")).unwrap(),
            "# Schéma údajov\n\nŽiadne stratené zmeny\n\n# Nadpis v obsahu\n"
        );
        assert_eq!(std::fs::read_to_string(base.join("schema-udajov-2.md")).unwrap(), "# Schéma údajov\n");
        assert_eq!(
            std::fs::read_to_string(base.join("schema-udajov.class.puml")).unwrap(),
            "@startuml\nA -> B\n@enduml\n"
        );
        assert!(base.join("schema-udajov.class.mmd").exists());
        let manifest = std::fs::read_to_string(dir.0.join(MANIFEST)).unwrap();
        assert!(!manifest.contains("revision") && !manifest.contains("activeDocument"));
    }

    #[test]
    fn unchanged_save_writes_nothing_and_stale_writer_is_refused() {
        let dir = TempDir::new();
        let mut project = fixture();
        let first = save(&dir.0, &project, None).unwrap();
        assert!(save(&dir.0, &project, None).is_err());
        let file = dir.0.join("documents/dokument/schema-udajov.md");
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        project.revision = 9;
        assert_eq!(save(&dir.0, &project, Some(&first)).unwrap(), first);
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written);

        project.documents[0].sections[0].content = "Updated".into();
        let second = save(&dir.0, &project, Some(&first)).unwrap();
        assert_ne!(second, first);
        assert!(save(&dir.0, &project, Some(&first)).unwrap_err().contains("changed on disk"));
        std::fs::write(&file, "# Schéma údajov\n\nEdited by git pull\n").unwrap();
        assert!(save(&dir.0, &project, Some(&second)).is_err());
        assert!(std::fs::read_to_string(&file).unwrap().contains("git pull"));
    }

    #[test]
    fn checkout_line_endings_are_not_a_change() {
        let dir = TempDir::new();
        let project = fixture();
        let fingerprint = save(&dir.0, &project, None).unwrap();
        let file = dir.0.join("documents/dokument/schema-udajov.md");
        let crlf = std::fs::read_to_string(&file).unwrap().replace('\n', "\r\n");
        std::fs::write(&file, &crlf).unwrap();
        let loaded = load_from(Folder(&dir.0)).unwrap();
        assert_eq!(loaded.fingerprint, fingerprint);
        assert_eq!(loaded.project, content(&project));
        save(&dir.0, &project, Some(&fingerprint)).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), crlf);
    }

    #[test]
    fn renames_remove_only_project_files() {
        let dir = TempDir::new();
        let mut project = fixture();
        let fingerprint = save(&dir.0, &project, None).unwrap();
        let old = dir.0.join("documents/dokument");
        std::fs::write(old.join("notes.txt"), "mine").unwrap();
        project.documents[0].name = "Architektúra".into();
        project.documents[0].sections.remove(1);
        let fingerprint = save(&dir.0, &project, Some(&fingerprint)).unwrap();
        assert_eq!(load_from(Folder(&dir.0)).unwrap().project, content(&project));
        assert!(dir.0.join("documents/architektura/schema-udajov.md").exists());
        assert_eq!(
            std::fs::read_dir(&old).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(),
            vec![std::ffi::OsString::from("notes.txt")]
        );

        // A foreign file in the way is never overwritten, and nothing else is written either.
        std::fs::write(dir.0.join("documents/architektura/readme.md"), "mine").unwrap();
        project.documents[0].sections[0].title = "README".into();
        project.documents[0].sections[0].content = "must not be written".into();
        assert!(save(&dir.0, &project, Some(&fingerprint)).unwrap_err().contains("does not belong"));
        assert_eq!(load_from(Folder(&dir.0)).unwrap().fingerprint, fingerprint);
    }

    #[test]
    fn hand_written_projects_load_and_unsafe_manifests_do_not() {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.0.join("documents/api")).unwrap();
        std::fs::write(dir.0.join(MANIFEST), "\u{feff}{\"format\":\"archgen-project\",\"version\":1,\"id\":\"p\",\"name\":\"Hand\",\"documents\":[\"api\"]}").unwrap();
        std::fs::write(dir.0.join("documents/api/document.json"), r#"{"id":"d","name":"API","template":"custom","language":"en","sections":[{"file":"overview.md","diagrams":[{"type":"sequence","format":"plantuml","file":"flow.puml"}]},{"file":"plain.md"}]}"#).unwrap();
        std::fs::write(dir.0.join("documents/api/overview.md"), "# Overview\r\n\r\nText\r\n").unwrap();
        std::fs::write(dir.0.join("documents/api/plain.md"), "no heading").unwrap();
        std::fs::write(dir.0.join("documents/api/flow.puml"), "@startuml\n@enduml\n").unwrap();
        let loaded = load_from(Folder(&dir.0)).unwrap();
        let sections = &loaded.project.documents[0].sections;
        assert_eq!((sections[0].title.as_str(), sections[0].content.as_str()), ("Overview", "Text"));
        assert_eq!((sections[1].title.as_str(), sections[1].content.as_str()), ("plain", "no heading"));
        assert_eq!(sections[0].diagrams[0].content, "@startuml\n@enduml");
        assert!(!sections[0].id.is_empty() && !sections[0].diagrams[0].id.is_empty());
        // Minted IDs are written back; the hand-made file names give way to the generated ones.
        save(&dir.0, &loaded.project, Some(&loaded.fingerprint)).unwrap();
        assert_eq!(load_from(Folder(&dir.0)).unwrap().project, loaded.project);
        assert!(dir.0.join("documents/api/overview.sequence.puml").exists());
        assert!(!dir.0.join("documents/api/flow.puml").exists());

        for name in ["../outside", "a/b", "C:evil", "..", ""] {
            std::fs::write(dir.0.join(MANIFEST), serde_json::json!({"format":FORMAT,"version":1,"id":"p","name":"x","documents":[name]}).to_string()).unwrap();
            assert!(load_from(Folder(&dir.0)).unwrap_err().contains("Unsafe"), "{name}");
        }
        let empty = TempDir::new();
        assert!(load_from(Folder(&empty.0)).unwrap_err().contains("archgen.json is missing"));
        assert!(save(&empty.0, &fixture(), Some("0")).is_err());
        assert_eq!(std::fs::read_dir(&empty.0).unwrap().count(), 0);
    }

    #[test]
    fn invalid_project_never_creates_files() {
        let dir = TempDir::new();
        let mut project = fixture();
        project.documents[0].sections[0].id = "doc".into();
        assert!(save(&dir.0, &project, None).is_err());
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
    }

    #[test]
    fn windows_device_names_are_recognized_with_any_extension() {
        for name in ["con", "CON.md", "nul.txt", "Aux.puml", "com1.md", "LPT9", "prn .md"] {
            assert!(windows_device_name(name), "{name}");
        }
        for name in ["console.md", "com10.md", "com0.md", "icon.md", "null.md", "lpt.md", "con-section.md"] {
            assert!(!windows_device_name(name), "{name}");
        }
        // Every generated name stays clear of them.
        for title in ["CON", "nul", "COM1", "lpt9"] {
            assert!(!windows_device_name(&format!("{}.md", slug(title, "section"))), "{title}");
        }
        assert_eq!(safe_name("CON.md").is_err(), cfg!(windows));
    }

    #[test]
    fn slugs_are_portable() {
        assert_eq!(slug("5. Building Block View", "section"), "5-building-block-view");
        assert_eq!(slug("Žltý kôň / úpäť", "section"), "zlty-kon-upat");
        assert_eq!(slug("???", "section"), "section");
        assert_eq!(slug("CON", "section"), "con-section");
        assert_eq!(slug("com1", "section"), "com1-section");
        assert!(slug(&"x".repeat(200), "section").len() <= 60);
    }

    fn run(root: &Path, args: &[&str]) {
        let status = git(root)
            .args(["-c", "user.name=ArchGen Test", "-c", "user.email=test@example.invalid"])
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=true"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn history_reads_immutable_snapshots_from_git() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let repo = TempDir::new();
        let root = repo.0.join("docs/architecture");
        let mut project = fixture();
        assert!(save(&root, &project, None).is_ok());
        assert!(history(&root, "project", 0).unwrap_err().contains("not in a Git repository"));
        run(&repo.0, &["init", "-q"]);
        assert!(history(&root, "project", 0).unwrap().is_empty());
        run(&repo.0, &["add", "."]);
        run(&repo.0, &["commit", "-q", "-m", "First version"]);
        let fingerprint = load_from(Folder(&root)).unwrap().fingerprint;
        project.name = "New name".into();
        project.documents[0].sections[0].content = "Second".into();
        save(&root, &project, Some(&fingerprint)).unwrap();
        run(&repo.0, &["add", "."]);
        run(&repo.0, &["commit", "-q", "-m", "Second version"]);
        std::fs::write(repo.0.join("unrelated.txt"), "x").unwrap();
        run(&repo.0, &["add", "."]);
        run(&repo.0, &["commit", "-q", "-m", "Outside the project"]);

        let entries = history(&root, "project", 0).unwrap();
        assert_eq!(
            entries.iter().map(|e| e.summary.as_str()).collect::<Vec<_>>(),
            vec!["Second version", "First version"]
        );
        assert!(!entries[0].saved_at.is_empty());
        assert_eq!(history(&root, "project", 1).unwrap().len(), 1);
        assert_eq!(
            historical_project(&root, "project", &entries[1].commit).unwrap(),
            content(&fixture())
        );
        assert_eq!(load_from(Folder(&root)).unwrap().project.name, "New name");
        assert!(historical_project(&root, "other", &entries[1].commit).is_err());
        assert!(historical_project(&root, "project", "HEAD; rm -rf").is_err());
        assert!(historical_project(&root, "project", &"0".repeat(40)).is_err());
        assert!(history(&root, "other", 0).is_err());
    }
}
