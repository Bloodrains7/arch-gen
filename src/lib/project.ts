export interface Diagram {
  id?: string;
  content: string;
  diagram_type: string;
  format: string;
}

export interface Section {
  id?: string;
  title: string;
  content: string;
  diagrams: Diagram[];
}

export interface ProjectDocument {
  id: string;
  name: string;
  template: string;
  language: string;
  revision: number;
  sections: Section[];
}

export interface Project {
  schemaVersion: 1;
  id: string;
  name: string;
  revision: number;
  activeDocumentId: string;
  documents: ProjectDocument[];
}

export function normalizeSections(sections: Section[]): Section[] {
  return sections.map(section => ({
    ...section,
    id: section.id ?? crypto.randomUUID(),
    diagrams: section.diagrams.map(diagram => ({ ...diagram, id: diagram.id ?? crypto.randomUUID() })),
  }));
}

export function createDocument(name = "Untitled"): ProjectDocument {
  return { id: crypto.randomUUID(), name, template: "arc42", language: "en", revision: 0, sections: [] };
}

export function createProject(): Project {
  const document = createDocument();
  return { schemaVersion: 1, id: crypto.randomUUID(), name: "Untitled project", revision: 0,
    activeDocumentId: document.id, documents: [document] };
}

export function updateDocument(project: Project, id: string, patch: Partial<Pick<ProjectDocument, "name" | "template" | "language" | "sections">>): Project {
  const document = project.documents.find(item => item.id === id);
  if (!document) return project;
  const next = { ...document, ...patch };
  if (patch.sections) next.sections = normalizeSections(patch.sections);
  if (JSON.stringify(next) === JSON.stringify(document)) return project;
  next.revision++;
  return { ...project, revision: project.revision + 1,
    documents: project.documents.map(item => item.id === id ? next : item) };
}

export interface UpdateTarget {
  projectId: string;
  documentId: string;
  revision: number;
  template: string;
  language: string;
  name: string;
}

export function captureTarget(project: Project): UpdateTarget {
  const document = project.documents.find(item => item.id === project.activeDocumentId)!;
  return { projectId: project.id, documentId: document.id, revision: document.revision,
    template: document.template, language: document.language, name: document.name };
}

export function applyGeneratedSections(project: Project, target: UpdateTarget, sections: Section[]): Project | null {
  const document = project.documents.find(item => item.id === target.documentId);
  if (project.id !== target.projectId || !document || document.revision !== target.revision) return null;
  return updateDocument(project, target.documentId, { sections });
}

export interface UndoEntry { project: Project; label: string }
export interface UndoHistory { past: UndoEntry[]; future: UndoEntry[]; group?: string; lastEdit: number }
export const emptyHistory = (): UndoHistory => ({ past: [], future: [], lastEdit: 0 });

function trimUndo(entries: UndoEntry[]): UndoEntry[] {
  let size = 0;
  const kept: UndoEntry[] = [];
  for (const entry of entries.slice(-50).reverse()) {
    const bytes = JSON.stringify(entry.project).length * 2;
    if (kept.length && size + bytes > 32 * 1024 * 1024) break;
    kept.unshift(entry); size += bytes;
  }
  return kept;
}

export function recordChange(history: UndoHistory, current: Project, next: Project, label: string, group?: string, now = Date.now()): UndoHistory {
  if (current === next || JSON.stringify(current) === JSON.stringify(next)) return history;
  const coalesce = group !== undefined && history.group === group && now - history.lastEdit < 750 && history.future.length === 0;
  const past = coalesce ? history.past : trimUndo([...history.past, { project: JSON.parse(JSON.stringify(current)), label }]);
  return { past, future: [], group, lastEdit: now };
}

// Restoring content is a new change, never a rewind of the revision clock.
export function restoreSnapshot(current: Project, snapshot: Project): Project {
  if (current.id !== snapshot.id) throw new Error("Cannot restore history from another project.");
  const revision = current.revision + 1;
  const restored = parseProject(JSON.parse(JSON.stringify(snapshot)));
  return parseProject({ ...restored, revision, documents: restored.documents.map(d => ({ ...d, revision })) });
}

export function moveHistory(project: Project, history: UndoHistory, direction: "undo" | "redo"): { project: Project; history: UndoHistory; label: string } | null {
  const from = direction === "undo" ? history.past : history.future;
  const entry = from.at(-1);
  if (!entry) return null;
  const reverse = { project: JSON.parse(JSON.stringify(project)), label: entry.label };
  return {
    project: restoreSnapshot(project, entry.project), label: entry.label,
    history: direction === "undo"
      ? { past: history.past.slice(0, -1), future: trimUndo([...history.future, reverse]), lastEdit: 0 }
      : { past: trimUndo([...history.past, reverse]), future: history.future.slice(0, -1), lastEdit: 0 },
  };
}

export interface SectionChange { key: string; before?: Section; after?: Section; kind: "added" | "removed" | "changed" | "unchanged" }

export function compareSections(before: Section[], after: Section[]): SectionChange[] {
  const matched = new Set<Section>();
  const changes: SectionChange[] = before.map((section, i) => {
    // Generated full documents do not carry section IDs. Unique titles are only
    // a display alignment hint; they never overwrite identity in the proposal.
    const sameId = after.find(s => section.id && s.id === section.id);
    const sameTitle = before.filter(s => s.title === section.title).length === 1 && after.filter(s => s.title === section.title).length === 1
      ? after.find(s => s.title === section.title) : undefined;
    const candidate = sameId ?? (sameTitle && !before.some(s => s.id && s.id === sameTitle.id) ? sameTitle : undefined);
    const next = candidate && !matched.has(candidate) ? candidate : undefined;
    if (next) matched.add(next);
    const text = (s: Section) => JSON.stringify({ title: s.title, content: s.content,
      diagrams: s.diagrams.map(d => ({ content: d.content, diagram_type: d.diagram_type, format: d.format })) });
    return { key: `before-${i}`, before: section, after: next,
      kind: !next ? "removed" : text(section) === text(next) ? "unchanged" : "changed" };
  });
  after.forEach((section, i) => { if (!matched.has(section)) changes.push({ key: `after-${i}`, after: section, kind: "added" }); });
  return changes;
}

export function sectionSource(section?: Section): string {
  if (!section) return "(Not present)";
  return [`# ${section.title}`, section.content, ...section.diagrams.map(d => `Diagram: ${d.diagram_type} (${d.format})\n${d.content}`)].join("\n\n");
}

// Treat files as untrusted input, including files with a valid SQLite header.
export function parseProject(value: unknown): Project {
  const ids = new Set<string>();
  const object = (v: unknown): Record<string, unknown> => {
    if (!v || typeof v !== "object" || Array.isArray(v)) throw new Error("Invalid project object.");
    return v as Record<string, unknown>;
  };
  const str = (v: unknown): string => {
    if (typeof v !== "string") throw new Error("Invalid project text field.");
    return v;
  };
  const id = (v: unknown): string => {
    const text = str(v);
    if (!text || ids.has(text)) throw new Error("Missing or duplicate project ID.");
    ids.add(text); return text;
  };
  const revision = (v: unknown): number => {
    if (!Number.isSafeInteger(v) || (v as number) < 0) throw new Error("Invalid project revision.");
    return v as number;
  };
  const array = (v: unknown): unknown[] => {
    if (!Array.isArray(v)) throw new Error("Invalid project list.");
    return v;
  };
  const p = object(value);
  if (p.schemaVersion !== 1) throw new Error("Unsupported project version. Update ArchGen to open this file.");
  const result: Project = {
    schemaVersion: 1, id: id(p.id), name: str(p.name), revision: revision(p.revision),
    activeDocumentId: str(p.activeDocumentId),
    documents: array(p.documents).map(value => {
      const d = object(value);
      return { id: id(d.id), name: str(d.name), template: str(d.template), language: str(d.language),
        revision: revision(d.revision), sections: array(d.sections).map(value => {
          const s = object(value);
          return { id: id(s.id), title: str(s.title), content: str(s.content), diagrams: array(s.diagrams).map(value => {
            const g = object(value);
            return { id: id(g.id), content: str(g.content), diagram_type: str(g.diagram_type), format: str(g.format) };
          }) };
        }) };
    }),
  };
  if (!result.documents.some(d => d.id === result.activeDocumentId) || result.documents.some(d => d.revision > result.revision)) {
    throw new Error("Invalid active document or document revision.");
  }
  return result;
}
