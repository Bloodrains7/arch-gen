<script lang="ts">
  import { compareSections, sectionSource } from "../project";
  import type { Section } from "../project";
  let { before, after }: { before: Section[]; after: Section[] } = $props();
  let changes = $derived(compareSections(before, after));
  let showUnchanged = $state(false);
</script>

<p class="summary">
  {changes.filter(c => c.kind === "added").length} added ·
  {changes.filter(c => c.kind === "removed").length} removed ·
  {changes.filter(c => c.kind === "changed").length} changed ·
  {changes.filter(c => c.kind === "unchanged").length} unchanged sections
</p>
<label><input type="checkbox" bind:checked={showUnchanged} /> Show unchanged sections</label>
<p class="hint">Before / after source comparison. The proposed section order is shown below.</p>
<p class="order">Proposed order: {after.map(s => s.title).join(" → ") || "(empty document)"}</p>
{#each changes.filter(c => showUnchanged || c.kind !== "unchanged") as change (change.key)}
  <section class="change" data-kind={change.kind}>
    <h4>{change.after?.title ?? change.before?.title} — {change.kind}</h4>
    <div class="comparison">
      <div><h5>Before</h5><pre>{sectionSource(change.before)}</pre></div>
      <div><h5>Proposed</h5><pre>{sectionSource(change.after)}</pre></div>
    </div>
  </section>
{/each}

<style>
  .summary, label { font-size: 13px; }
  .hint, .order { font-size: 12px; color: var(--text-muted); overflow-wrap: anywhere; }
  .change { border: 1px solid var(--border); border-radius: 6px; margin: 12px 0; padding: 12px; }
  h4 { margin: 0 0 8px; font-size: 14px; }
  h5 { margin: 0 0 8px; color: var(--text-muted); }
  .comparison { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 12px; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; margin: 0; padding: 10px; background: var(--bg-primary); max-height: 360px; overflow: auto; font-size: 12px; }
  [data-kind="added"] { border-left: 3px solid var(--success); }
  [data-kind="removed"] { border-left: 3px solid var(--danger); }
  [data-kind="changed"] { border-left: 3px solid var(--accent); }
  @media (max-width: 700px) { .comparison { grid-template-columns: 1fr; } }
</style>
