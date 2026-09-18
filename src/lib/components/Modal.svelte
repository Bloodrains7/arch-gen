<script lang="ts">
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  let { title, onClose, children }: { title: string; onClose: () => void; children: Snippet } = $props();
  let element: HTMLDialogElement;
  onMount(() => { element.showModal(); });
</script>

<dialog bind:this={element} aria-label={title} oncancel={(event) => { event.preventDefault(); onClose(); }}>
  <header><h2>{title}</h2><button onclick={onClose}>Close</button></header>
  <div class="body">{@render children()}</div>
</dialog>

<style>
  dialog { width: min(1080px, 92vw); max-height: 88vh; padding: 0; color: var(--text-primary); background: var(--bg-secondary); border: 1px solid var(--border); border-radius: 10px; }
  dialog::backdrop { background: #0009; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 16px 20px; border-bottom: 1px solid var(--border); position: sticky; top: 0; background: var(--bg-secondary); z-index: 1; }
  h2 { font-size: 18px; margin: 0; }
  button { padding: 7px 12px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; }
  .body { padding: 20px; }
</style>
