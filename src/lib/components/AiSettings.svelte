<script lang="ts">
  // Every field here saves immediately (there is no separate "Save settings"
  // step): picking a provider, changing a model or URL, or setting a key calls
  // its ai.ts wrapper right away and hands the fresh AiStatus back to App, the
  // same status shape PromptPanel reads for the consent text and provider chip.
  import { invoke } from "@tauri-apps/api/core";
  import Modal from "./Modal.svelte";
  import { configureAi, setAiKey, fetchOllamaModels, testProvider, currentProvider } from "../ai";
  import type { AiStatus, ProviderId } from "../ai";

  let {
    aiStatus = null as AiStatus | null,
    aiStatusError = "",
    onStatusChange = (_status: AiStatus) => {},
    onRetry = () => {},
    onClose = () => {},
  }: {
    aiStatus?: AiStatus | null;
    aiStatusError?: string;
    onStatusChange?: (status: AiStatus) => void;
    onRetry?: () => void;
    onClose?: () => void;
  } = $props();

  // The provider the dialog is showing details for is always the configured
  // one: selecting a radio button commits it immediately (see `switchProvider`),
  // so there is never a separate "pending" provider to track.
  let current = $derived(aiStatus ? currentProvider(aiStatus) : undefined);
  let needsKey = $derived(!!current && !current.local && !current.cli);

  let providerError = $state("");
  let modelError = $state("");
  let urlError = $state("");
  let timeoutError = $state("");
  let keyError = $state("");
  let keyInput = $state("");
  let testMessage = $state("");
  let testIsError = $state(false);
  let testing = $state(false);

  let ollamaModels = $state<string[] | null>(null);
  let ollamaManualEntry = $state(false);
  let ollamaModelsError = $state("");
  let ollamaLoadSeq = 0;

  // The radiogroup's own DOM, used only to correct a radio's `.checked` after a
  // rejected switch (see `switchProvider`): Svelte's one-way `checked={...}`
  // compiles to a set that is skipped once the browser has already flipped the
  // radio itself, so a failed save would otherwise leave the wrong one checked
  // forever (settings-dialog-and-e2e-quality#2).
  let radioGroup = $state<HTMLDivElement>();

  function syncRadios() {
    if (!radioGroup) return;
    for (const input of radioGroup.querySelectorAll<HTMLInputElement>('input[type="radio"]')) {
      input.checked = input.value === aiStatus?.provider;
    }
  }

  async function loadOllamaModels() {
    const seq = ++ollamaLoadSeq;
    try {
      const models = await fetchOllamaModels(invoke);
      if (seq !== ollamaLoadSeq) return; // superseded by a later reload
      ollamaModels = models; ollamaManualEntry = false; ollamaModelsError = "";
    } catch (error) {
      if (seq !== ollamaLoadSeq) return;
      ollamaModels = []; ollamaManualEntry = true; ollamaModelsError = String(error);
    }
  }

  // Fires on first open (if Ollama is already configured), on every switch to
  // Ollama, and whenever something sets `ollamaModels` back to `null`: a
  // successful URL save, or the "Reload models" button
  // (settings-dialog-and-e2e-quality#9).
  $effect(() => {
    if (current?.id === "ollama" && ollamaModels === null) void loadOllamaModels();
  });

  async function switchProvider(id: ProviderId) {
    // Per-provider errors and the in-progress key input belong to the provider
    // being left; `urlError`/`timeoutError` stay, because those two fields sit
    // outside the per-provider panel and keep showing the same rejected value
    // regardless of which provider is selected (settings-dialog-and-e2e-quality#8).
    providerError = ""; modelError = ""; keyError = ""; testMessage = ""; keyInput = "";
    try { onStatusChange(await configureAi(invoke, { provider: id })); }
    catch (error) {
      providerError = String(error);
      syncRadios(); // the browser already checked `id`; put it back on the real provider
    }
  }

  async function saveModel(model: string) {
    modelError = "";
    try { onStatusChange(await configureAi(invoke, { model })); }
    catch (error) { modelError = String(error); }
  }

  async function saveUrl(ollamaUrl: string) {
    urlError = "";
    try {
      onStatusChange(await configureAi(invoke, { ollamaUrl }));
      ollamaModels = null; // the origin changed: the old model list no longer applies
    }
    catch (error) { urlError = String(error); }
  }

  async function saveTimeout(timeoutSeconds: number) {
    timeoutError = "";
    try { onStatusChange(await configureAi(invoke, { timeoutSeconds })); }
    catch (error) { timeoutError = String(error); }
  }

  // The backend stores an integer 30..=3600 (`Option<u64>`); clamping here means
  // the dialog never sends a float, NaN or out-of-range value for Tauri to
  // reject with a raw "invalid args" string, and the field is written back
  // through the DOM directly so a second out-of-range entry is never left
  // on screen behind a value the backend already clamped
  // (settings-dialog-and-e2e-quality#7 — Svelte's one-way `value={...}` skips
  // the write once it has already rendered the same clamped number once).
  function handleTimeoutChange(e: Event & { currentTarget: HTMLInputElement }) {
    const raw = e.currentTarget.valueAsNumber;
    if (!Number.isFinite(raw)) {
      timeoutError = "Enter a number of seconds between 30 and 3600.";
      e.currentTarget.value = String(aiStatus?.timeoutSeconds ?? "");
      return;
    }
    const clamped = Math.min(3600, Math.max(30, Math.round(raw)));
    e.currentTarget.value = String(clamped);
    saveTimeout(clamped);
  }

  // Write-only: the field is cleared the moment the save succeeds, and nothing
  // here ever reads a key back out of `current` — the backend never sends one.
  async function saveKey() {
    if (!current || !keyInput) return;
    keyError = "";
    try { onStatusChange(await setAiKey(invoke, current.id, keyInput)); keyInput = ""; }
    catch (error) { keyError = String(error); }
  }

  async function removeKey() {
    if (!current) return;
    keyError = ""; keyInput = "";
    try { onStatusChange(await setAiKey(invoke, current.id, "")); }
    catch (error) { keyError = String(error); }
  }

  async function runTest() {
    if (!current || testing) return;
    const id = current.id; // ignore a reply that arrives after the panel moved on
    testing = true; testMessage = ""; testIsError = false;
    try {
      const result = await testProvider(invoke, id);
      if (current?.id === id) testMessage = result;
    } catch (error) {
      if (current?.id === id) { testMessage = String(error); testIsError = true; }
    } finally {
      testing = false;
    }
  }

  function keySourceLabel(source: string): string {
    switch (source) {
      case "env": return "Using the key from the environment.";
      case "settings": return "Using a key stored in AI settings.";
      case "not_needed": return "This provider does not need a key.";
      default: return "No key is set.";
    }
  }
</script>

<Modal title="AI settings" {onClose}>
  {#if !aiStatus}
    <p role="alert">{aiStatusError || "AI settings are not available right now."}</p>
    <button type="button" class="dialog-button" onclick={onRetry}>Try again</button>
  {:else}
    <h3 id="ai-provider-heading">Provider</h3>
    <div class="providers" role="radiogroup" aria-labelledby="ai-provider-heading" bind:this={radioGroup}>
      {#each aiStatus.providers as provider (provider.id)}
        <label class="provider-option">
          <input
            type="radio"
            name="ai-provider"
            aria-label={provider.label}
            aria-describedby={`ai-provider-avail-${provider.id} ai-provider-detail-${provider.id}`}
            value={provider.id}
            checked={provider.id === aiStatus.provider}
            onchange={() => switchProvider(provider.id)}
          />
          <span class="provider-text">
            <span class="provider-label">{provider.label} — <span id={`ai-provider-avail-${provider.id}`}>{provider.available ? "Available" : "Not available"}</span></span>
            <span id={`ai-provider-detail-${provider.id}`} class="provider-detail">{provider.detail}</span>
          </span>
        </label>
      {/each}
    </div>
    {#if providerError}<p role="alert">{providerError}</p>{/if}

    {#if current}
      <h4 class="current-provider">{current.label} settings</h4>
      {#key current.id}
        <label class="field">
          Model
          {#if current.id === "ollama" && !ollamaManualEntry}
            <select value={current.model} onchange={(e) => saveModel(e.currentTarget.value)}>
              <option value="">Choose an installed model</option>
              {#if ollamaModels === null && current.model}
                <option value={current.model}>{current.model} (loading…)</option>
              {/if}
              {#each ollamaModels ?? [] as model (model)}<option value={model}>{model}</option>{/each}
              {#if ollamaModels !== null && current.model && !ollamaModels.includes(current.model)}
                <option value={current.model}>{current.model} (not installed)</option>
              {/if}
            </select>
            {#if ollamaModels !== null && ollamaModels.length === 0}
              <p class="hint">No installed local Ollama models were found. Run "ollama pull" for one, then reload.</p>
            {/if}
          {:else}
            <input value={current.model} placeholder={current.defaultModel || "default model"} onchange={(e) => saveModel(e.currentTarget.value)} />
          {/if}
        </label>
        {#if current.id === "ollama"}
          <button type="button" class="dialog-button" onclick={() => { ollamaModels = null; }}>Reload models</button>
        {/if}
        {#if modelError}<p role="alert">{modelError}</p>{/if}
        {#if ollamaManualEntry && ollamaModelsError}<p role="alert">{ollamaModelsError}</p>{/if}

        {#if needsKey}
          <label class="field">
            API key
            <input type="password" autocomplete="off" bind:value={keyInput} />
          </label>
          <p class="key-source">{keySourceLabel(current.keySource)}</p>
          <div class="key-actions">
            <button type="button" class="dialog-button" onclick={saveKey} disabled={!keyInput}>Save key</button>
            <button type="button" class="dialog-button" onclick={removeKey} disabled={current.keySource === "none"}>Remove key</button>
          </div>
          {#if keyError}<p role="alert">{keyError}</p>{/if}
        {/if}
      {/key}
    {/if}

    <label class="field">
      Ollama URL
      <input value={aiStatus.ollamaUrl} onchange={(e) => saveUrl(e.currentTarget.value)} />
    </label>
    {#if urlError}<p role="alert">{urlError}</p>{/if}

    <label class="field">
      Timeout (seconds)
      <input type="number" min="30" max="3600" value={aiStatus.timeoutSeconds} onchange={handleTimeoutChange} />
    </label>
    {#if timeoutError}<p role="alert">{timeoutError}</p>{/if}

    <button type="button" class="dialog-button" onclick={runTest} disabled={!current || testing}>{testing ? "Testing…" : "Test provider"}</button>
    {#if testMessage}<p role={testIsError ? "alert" : "status"}>{testMessage}</p>{/if}
  {/if}
</Modal>

<style>
  h3 { font-size: 13px; margin: 0 0 8px; color: var(--text-secondary); }
  h4.current-provider { font-size: 13px; margin: 12px 0 0; color: var(--text-secondary); }
  .providers { display: flex; flex-direction: column; gap: 8px; margin-bottom: 8px; }
  .provider-option { display: flex; align-items: flex-start; gap: 8px; padding: 8px; border: 1px solid var(--border); border-radius: var(--radius); }
  .provider-text { display: flex; flex-direction: column; gap: 2px; }
  .provider-label { font-size: 13px; color: var(--text-primary); }
  .provider-detail { font-size: 12px; color: var(--text-muted); }
  .field { display: block; margin: 12px 0 4px; font-size: 12px; color: var(--text-secondary); }
  .field input, .field select { display: block; width: 100%; margin-top: 4px; padding: 7px 10px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; font-size: 13px; }
  .hint { margin: 4px 0; font-size: 12px; color: var(--text-muted); }
  .key-source { margin: 4px 0; font-size: 12px; color: var(--text-muted); }
  .key-actions { display: flex; gap: 8px; margin-bottom: 4px; }
  .dialog-button { padding: 7px 12px; color: var(--text-primary); background: var(--bg-tertiary); border: 1px solid var(--border); border-radius: 4px; }
  .dialog-button:disabled { opacity: 0.5; cursor: not-allowed; }
  [role="alert"] { color: var(--danger); font-size: 12px; margin: 4px 0; }
  [role="status"] { color: var(--success); font-size: 12px; margin: 4px 0; }
</style>
