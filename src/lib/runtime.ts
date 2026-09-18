import type { Project, ProjectDocument, Section } from "./project";

export type GenerationRequest =
  | { kind: "documentation"; description: string; template: string; language: string }
  | { kind: "diagram"; description: string; diagramType: string; language: string; sectionId: string | null; diagramId: string | null }
  | {
      kind: "rework";
      instruction: string;
      language: string;
      scope: "selection" | "document";
      sectionIds: string[];
      diagramIds: string[];
      includeContext: boolean;
      provider: string;
      model: string;
    };

export interface GenerationJob {
  id: string;
  projectId: string;
  documentId: string;
  baseDocument: ProjectDocument;
  request: GenerationRequest;
  status: "queued" | "running" | "ready" | "failed" | "interrupted" | "cancelled" | "accepted" | "recovered" | "discarded";
  sections: Section[] | null;
  summary: string | null;
  error: string | null;
  createdAt: string;
}

export type RuntimeInvoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

// Typing stays responsive while acknowledged edits are serialized through Rust.
// An error stops the queue: never send later snapshots over an uncertain base.
export class EditSession {
  id = "";
  project!: Project;
  error: string | null = null;
  private tail: Promise<void> = Promise.resolve();
  private invoke: RuntimeInvoke;
  constructor(invoke: RuntimeInvoke) { this.invoke = invoke; }

  async open(project: Project) {
    await this.tail;
    const result = await this.invoke<{ sessionId: string; project: Project }>("open_edit_session", { project });
    this.id = result.sessionId; this.project = result.project; this.error = null;
    return this.project;
  }

  edit(proposed: Project, invalidateDocuments = false): Promise<Project> {
    const snapshot: Project = JSON.parse(JSON.stringify(proposed));
    const operation = this.tail.then(async () => {
      if (this.error) throw new Error(this.error);
      const result = await this.invoke<Project>("apply_project_edit", {
        sessionId: this.id, expectedRevision: this.project.revision, proposed: snapshot, invalidateDocuments,
      });
      this.project = result;
      return result;
    });
    this.tail = operation.then(() => {}, error => { this.error = String(error); });
    return operation;
  }

  async flush(): Promise<Project> {
    await this.tail;
    if (this.error) throw new Error(this.error);
    return this.project;
  }

  async resolveJob(command: "accept_generation_job" | "recover_generation_job", jobId: string): Promise<Project> {
    await this.flush();
    const project = await this.invoke<Project>(command, { sessionId: this.id, jobId, expectedRevision: this.project.revision });
    this.project = project;
    return project;
  }
}

export function jobCompatible(project: Project, job: GenerationJob): boolean {
  const current = project.documents.find(d => d.id === job.documentId);
  if (job.status !== "ready" || job.projectId !== project.id || !current) return false;
  const canonical = (d: ProjectDocument) => JSON.stringify({
    id: d.id, name: d.name, template: d.template, language: d.language, revision: d.revision,
    sections: d.sections.map(s => ({ id: s.id, title: s.title, content: s.content,
      diagrams: s.diagrams.map(g => ({ id: g.id, content: g.content, diagram_type: g.diagram_type, format: g.format })) })),
  });
  return canonical(current) === canonical(job.baseDocument);
}
