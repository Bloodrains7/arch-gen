// Contracts of the Git commands in src-tauri/src/git.rs and release.rs.
export interface GitRef { name: string; kind: "tag" | "branch" | "remote"; commit: string; date: string }
export interface GitRefs { root: string; branch: string | null; head: string | null; refs: GitRef[] }

export interface GitChange { path: string; status: "modified" | "added" | "deleted" | "renamed" | "conflicted" | "untracked" }
export interface GitStatus {
  root: string;
  branch: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  changes: GitChange[];
  truncated: boolean;
}
export interface GitCommitted { commit: string; summary: string; files: number }
export interface TemplateFile { file: string; content: string; error: string | null }

/** "main ↑1 ↓2 · 3 changes" — short enough for the history bar. */
export function statusSummary(status: GitStatus): string {
  const branch = status.branch ?? (status.head ? `detached ${status.head.slice(0, 7)}` : "no commits yet");
  const sync = status.upstream ? [status.ahead ? `↑${status.ahead}` : "", status.behind ? `↓${status.behind}` : ""].filter(Boolean).join(" ") : "no upstream";
  const count = status.changes.length + (status.truncated ? "+" : "");
  const changes = status.changes.length ? `${count} change${status.changes.length === 1 ? "" : "s"}` : "clean";
  return [branch, sync, changes].filter(Boolean).join(" · ");
}

/**
 * The usual release range: from the newest tag to HEAD, or, when HEAD is that
 * tag, from the tag before it — so opening the dialog right after tagging a
 * release shows that release.
 */
export function defaultRange(refs: GitRefs): { from: string; to: string; version: string } {
  const tags = refs.refs.filter(r => r.kind === "tag");
  if (tags.length && refs.head && tags[0].commit === refs.head) {
    return { from: tags.find(t => t.commit !== refs.head)?.name ?? "", to: tags[0].name, version: tags[0].name.replace(/^v(?=\d)/, "") };
  }
  return { from: tags[0]?.name ?? "", to: "HEAD", version: "Unreleased" };
}

/** Tauri rejects with the Rust error string; anything else (a thrown Error) keeps its message only. */
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
