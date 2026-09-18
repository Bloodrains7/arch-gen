<script lang="ts">
  import { marked } from "marked";
  import DOMPurify from "dompurify";

  let { content = "" } = $props();

  // Configure marked
  marked.setOptions({
    breaks: true,
    gfm: true,
  });

  let html = $derived(DOMPurify.sanitize(marked.parse(content || "") as string, {
    USE_PROFILES: { html: true }, FORBID_TAGS: ["img", "video", "audio", "source", "style"], FORBID_ATTR: ["style"],
  }));
</script>

<div class="md-preview">
  {@html html}
</div>

<style>
  .md-preview {
    padding: 16px;
    font-size: 14px;
    line-height: 1.7;
    color: var(--text-primary);
    min-height: 100px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow-y: auto;
    max-height: 600px;
  }

  .md-preview :global(h1) {
    font-size: 22px;
    font-weight: 700;
    margin: 0 0 12px;
    padding-bottom: 6px;
    border-bottom: 2px solid var(--accent);
    color: var(--text-primary);
  }

  .md-preview :global(h2) {
    font-size: 18px;
    font-weight: 600;
    margin: 20px 0 8px;
    color: var(--text-primary);
  }

  .md-preview :global(h3) {
    font-size: 15px;
    font-weight: 600;
    margin: 16px 0 6px;
    color: var(--text-primary);
  }

  .md-preview :global(h4) {
    font-size: 13px;
    font-weight: 600;
    margin: 12px 0 4px;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .md-preview :global(p) {
    margin: 6px 0;
  }

  .md-preview :global(strong) {
    color: var(--text-primary);
    font-weight: 600;
  }

  .md-preview :global(em) {
    color: var(--text-secondary);
    font-style: italic;
  }

  .md-preview :global(ul),
  .md-preview :global(ol) {
    padding-left: 20px;
    margin: 6px 0;
  }

  .md-preview :global(li) {
    margin: 3px 0;
  }

  .md-preview :global(table) {
    width: 100%;
    border-collapse: collapse;
    margin: 10px 0;
    font-size: 13px;
  }

  .md-preview :global(th) {
    background: var(--bg-hover);
    border: 1px solid var(--border);
    padding: 8px 10px;
    text-align: left;
    font-weight: 600;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    color: var(--text-secondary);
  }

  .md-preview :global(td) {
    border: 1px solid var(--border);
    padding: 6px 10px;
    color: var(--text-primary);
  }

  .md-preview :global(tr:hover td) {
    background: var(--bg-hover);
  }

  .md-preview :global(code) {
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    font-family: 'Cascadia Code', 'Fira Code', monospace;
    font-size: 12px;
    color: var(--accent);
  }

  .md-preview :global(pre) {
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px;
    overflow-x: auto;
    margin: 10px 0;
  }

  .md-preview :global(pre code) {
    background: none;
    border: none;
    padding: 0;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.5;
  }

  .md-preview :global(blockquote) {
    border-left: 3px solid var(--accent);
    margin: 10px 0;
    padding: 8px 14px;
    background: var(--accent-dim);
    border-radius: 0 var(--radius) var(--radius) 0;
    color: var(--text-secondary);
  }

  .md-preview :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
    margin: 16px 0;
  }

  .md-preview :global(a) {
    color: var(--accent);
    text-decoration: none;
  }

  .md-preview :global(a:hover) {
    text-decoration: underline;
  }

  .md-preview :global(input[type="checkbox"]) {
    margin-right: 6px;
    accent-color: var(--accent);
  }

  .md-preview :global(img) {
    max-width: 100%;
    border-radius: var(--radius);
  }
</style>
