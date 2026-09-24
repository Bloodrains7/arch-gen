<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import Modal from "./Modal.svelte";
  import { errorText, statusSummary } from "../git";
  import type { GitCommitted, GitStatus } from "../git";

  let {
    projectPath = null as string | null,
    projectId = "",
    projectName = "",
    // The on-disk content the app last saved or opened; changes after every save.
    fingerprint = null as string | null,
    dirty = false,
    busy = false,
    onStatus = (_message: string) => {},
  } = $props();

  let status = $state<GitStatus | null>(null);
  let error = $state("");
  let loading = $state(false);
  let working = $state(false);
  let commitOpen = $state(false);
  let message = $state("");
  let commitError = $state("");
  let request = 0;

  async function refresh() {
    const path = projectPath;
    const token = ++request;
    if (!path) { status = null; error = ""; return; }
    loading = true;
    try {
      const next = await invoke<GitStatus>("git_project_status", { path, projectId });
      if (token === request) { status = next; error = ""; }
    } catch (e) {
      if (token === request) { status = null; error = errorText(e); }
    } finally { if (token === request) loading = false; }
  }

  $effect(() => {
    // Re-read after every open and save, and when the project changes.
    projectPath; projectId; fingerprint;
    void refresh();
  });

  function openCommit() {
    message = `docs: update ${projectName || "architecture documentation"}`;
    commitError = "";
    commitOpen = true;
    void refresh();
  }

  async function commit() {
    if (!projectPath || !fingerprint || dirty || working) return;
    working = true; commitError = "";
    try {
      const done = await invoke<GitCommitted>("git_commit_project", { path: projectPath, projectId, expectedFingerprint: fingerprint, message });
      commitOpen = false;
      onStatus(`Committed ${done.commit.slice(0, 8)} (${done.files} file${done.files === 1 ? "" : "s"}): ${done.summary}`);
    } catch (e) { commitError = errorText(e); }
    finally { working = false; void refresh(); }
  }

  async function push() {
    if (!projectPath || !status?.upstream || working) return;
    if (!(await confirm(`Push branch ${status.branch} to ${status.upstream}? This sends your commits to the remote repository.`, { title: "Push to remote", kind: "info" }))) return;
    working = true;
    try { onStatus(await invoke<string>("git_push_project", { path: projectPath, projectId })); }
    catch (e) { onStatus(`Push failed: ${errorText(e)}`); }
    finally { working = false; void refresh(); }
  }

  const labels: Record<string, string> = { modified: "M", added: "A", deleted: "D", renamed: "R", conflicted: "U", untracked: "?" };
</script>

{#if projectPath}
  <span class="git-status" title={status ? `Repository ${status.root}` : error}>
    {#if loading && !status}Git…{:else if status}Git: {statusSummary(status)}{:else}Git: {error.split(".")[0] || "unavailable"}{/if}
  </span>
  <button disabled={loading || busy} onclick={refresh} title="Re-read Git status of the project folder">Refresh</button>
  <button disabled={busy || working || !status} onclick={openCommit}>Commit…</button>
  <button disabled={busy || working || !status?.upstream || !status.ahead} onclick={push}
    title={status?.upstream ? `Push ${status.branch} to ${status.upstream}` : "The branch has no upstream"}>Push</button>
{/if}

{#if commitOpen && status}
  <Modal title="Commit project folder" onClose={() => commitOpen = false}>
    <p>Commits only this project folder on branch <strong>{status.branch ?? "(detached)"}</strong>. Files staged elsewhere in the repository stay staged and are not part of this commit.</p>
    {#if dirty}<p class="warn" role="alert">There are unsaved edits. Save the project first: the commit contains what is saved on disk.</p>{/if}
    {#if status.changes.length}
      <ul class="changes" aria-label="Changed files">
        {#each status.changes as change (change.path)}
          <li><span class="code" title={change.status}>{labels[change.status]}</span> {change.path}</li>
        {/each}
        {#if status.truncated}<li>…and more</li>{/if}
      </ul>
    {:else}
      <p>No changes in the project folder since the last commit.</p>
    {/if}
    <label class="message">Commit message
      <textarea rows="4" bind:value={message}></textarea>
    </label>
    {#if commitError}<p class="warn" role="alert">{commitError}</p>{/if}
    <div class="actions">
      <button disabled={dirty || working || !message.trim() || !status.changes.length} onclick={commit}>{working ? "Committing…" : "Commit"}</button>
    </div>
  </Modal>
{/if}

<style>
  .git-status { font-size: 12px; color: var(--text-secondary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 360px; }
  button { padding: 7px 12px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; }
  button:disabled { opacity: 0.5; cursor: default; }
  .changes { list-style: none; margin: 12px 0; max-height: 220px; overflow-y: auto; font-family: monospace; font-size: 12px; }
  .changes li { padding: 2px 0; }
  .code { display: inline-block; width: 16px; color: var(--accent); }
  .message { display: flex; flex-direction: column; gap: 6px; margin-top: 12px; font-size: 13px; }
  textarea { padding: 8px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; font-family: inherit; }
  .warn { color: var(--warning); margin: 8px 0; }
  .actions { display: flex; gap: 10px; padding-top: 12px; }
</style>
