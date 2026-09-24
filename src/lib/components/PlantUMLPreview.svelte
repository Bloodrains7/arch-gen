<script lang="ts">
  import plantumlEncoder from "plantuml-encoder";
  import { invoke } from "@tauri-apps/api/core";
  import { localSvgUrl } from "../local-svg";
  import { renderMermaidSvg } from "../mermaid";

  let { content = "", format = "plantuml" } = $props();
  let showCode = $state(false);
  let error = $state("");
  let imgLoaded = $state(false);
  let allowRemote = $state(false);
  let localUrl = $state("");
  let rendering = $state(false);
  let request = 0;

  const MERMAID_DEBOUNCE_MS = 400;

  // Mermaid is local and uploads nothing, so it renders automatically (debounced) —
  // unlike PlantUML, which stays an explicit action. Reset state when content changes,
  // and (re-)schedule a debounced render for Mermaid; a token guards every async result
  // below so a stale render for earlier content can never overwrite a newer one.
  $effect(() => {
    content;  // track
    format;
    request++;
    const token = request;
    localUrl = "";
    rendering = false;
    error = "";
    imgLoaded = false;
    allowRemote = false;
    if (format !== "mermaid") return;
    const source = content;
    const timer = setTimeout(() => renderMermaid(token, source), MERMAID_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });

  async function renderMermaid(token: number, source: string) {
    rendering = true;
    try {
      const svg = await renderMermaidSvg(source);
      if (token === request) localUrl = localSvgUrl(svg);
    } catch (err) {
      if (token === request) error = err instanceof Error ? err.message : String(err);
    } finally {
      if (token === request) rendering = false;
    }
  }

  // Convert VARCHAR(100) -> VARCHAR[100] for PlantUML rendering only
  // PlantUML treats () as method signatures, hide methods then hides these fields
  function toRenderSafe(code: string): string {
    return code.replace(/(\w+)\((\d+)\)/g, '$1[$2]');
  }

  // Encode PlantUML and build SVG URL via public PlantUML server
  let encoded = $derived(format === "plantuml" && content.includes("@startuml")
    ? plantumlEncoder.encode(toRenderSafe(content))
    : ""
  );

  let svgUrl = $derived(allowRemote && encoded ? `https://www.plantuml.com/plantuml/svg/${encoded}` : "");
  let displayUrl = $derived(localUrl || svgUrl);

  async function renderLocal() {
    const token = ++request;
    const original = content;
    rendering = true; error = ""; allowRemote = false; localUrl = ""; imgLoaded = false;
    try {
      const svg = await invoke<string>("render_local_diagram", { content: original });
      if (token === request && content === original) localUrl = localSvgUrl(svg);
    } catch (err) { if (token === request) error = String(err); }
    finally { if (token === request) rendering = false; }
  }

  function handleImgError() {
    error = format === "mermaid" ? "Failed to render diagram." : "Failed to render diagram. Check PlantUML syntax.";
    imgLoaded = false;
  }

  function handleImgLoad() {
    error = "";
    imgLoaded = true;
  }
</script>

<div class="puml-preview">
  <div class="puml-toolbar">
    <span class="puml-label">{format === "mermaid" ? "Mermaid Diagram" : "PlantUML Diagram"}</span>
    <div class="puml-actions">
      <button
        class="puml-btn"
        class:active={!showCode}
        onclick={() => showCode = false}
      >Diagram</button>
      <button
        class="puml-btn"
        class:active={showCode}
        onclick={() => showCode = true}
      >Code</button>
      {#if svgUrl}
        <a class="puml-btn" href={svgUrl} target="_blank" rel="noopener">Open SVG</a>
      {/if}
    </div>
  </div>

  {#if showCode}
    <pre class="puml-code">{content}</pre>
  {:else if encoded && !displayUrl}
    <div class="puml-render">
      <button disabled={rendering} onclick={renderLocal}>{rendering ? "Rendering locally…" : "Render locally (no upload)"}</button>
      <button disabled={rendering} onclick={() => { error = ""; allowRemote = true; }}>Render using public PlantUML server (sends diagram content)</button>
      {#if error}<div class="puml-error" role="alert">{error}</div>{/if}
    </div>
  {:else if displayUrl}
    <div class="puml-render" class:loading={!imgLoaded && !error}>
      {#if !imgLoaded && !error}
        <div class="puml-loading">
          <div class="spinner"></div>
          <span>Rendering diagram...</span>
        </div>
      {/if}
      {#if error}
        <div class="puml-error">{error}</div>
      {/if}
      <img
        src={displayUrl}
        alt={format === "mermaid" ? "Mermaid Diagram" : "PlantUML Diagram"}
        onload={handleImgLoad}
        onerror={handleImgError}
        class:hidden={!imgLoaded}
      />
    </div>
  {:else if format === "mermaid" && error}
    <div class="puml-render">
      <div class="puml-error" role="alert">{error}</div>
    </div>
    <pre class="puml-code">{content}</pre>
  {:else if format === "mermaid"}
    <div class="puml-render loading">
      <div class="puml-loading">
        <div class="spinner"></div>
        <span>Rendering diagram...</span>
      </div>
    </div>
  {:else}
    <pre class="puml-code">{content}</pre>
  {/if}
</div>

<style>
  .puml-preview {
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    margin-top: 8px;
  }

  .puml-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    background: var(--bg-tertiary);
    border-bottom: 1px solid var(--border);
  }

  .puml-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .puml-actions {
    display: flex;
    gap: 2px;
    background: var(--bg-primary);
    border-radius: 5px;
    padding: 2px;
  }

  .puml-btn {
    padding: 3px 10px;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 500;
    border-radius: 4px;
    transition: all 0.15s;
    text-decoration: none;
    cursor: pointer;
  }

  .puml-btn:hover {
    color: var(--text-primary);
  }

  .puml-btn.active {
    background: var(--accent);
    color: white;
  }

  a.puml-btn {
    border: none;
  }

  .puml-render {
    padding: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 120px;
    background: #ffffff;
    position: relative;
  }

  .puml-render.loading {
    min-height: 150px;
  }

  .puml-render img {
    max-width: 100%;
    height: auto;
  }

  .puml-render img.hidden {
    display: none;
  }

  .puml-loading {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
    font-size: 12px;
    position: absolute;
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid #ddd;
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .puml-error {
    color: var(--danger);
    font-size: 12px;
    padding: 12px;
  }

  .puml-code {
    padding: 12px;
    font-size: 12px;
    font-family: 'Cascadia Code', 'Fira Code', monospace;
    color: var(--text-secondary);
    overflow-x: auto;
    line-height: 1.5;
    white-space: pre-wrap;
    margin: 0;
  }
</style>
