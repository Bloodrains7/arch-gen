<script lang="ts">
  import type { Section, Diagram } from "../project";
  import type { GenerationRequest } from "../runtime";
  import { emptySelection, selectionSummary } from "../selection";
  import { buildReworkRequest, consentText, currentProvider, providerChip } from "../ai";
  import type { AiStatus } from "../ai";

  let {
    sections = [],
    isGenerating = $bindable(false),
    selectedTemplate,
    selectedLanguage = "en",
    selection = emptySelection(),
    includeContext = false,
    aiStatus = null as AiStatus | null,
    aiStatusError = "",
    // Whole-document rework armed for the session+document App is currently
    // showing (App drops this the moment it no longer applies — see the
    // `scope` derivation below).
    documentScopeArmed = false,
    session = 0,
    onGenerate = async (_request: GenerationRequest) => {},
    onClearSelection = () => {},
    onOpenAiSettings = () => {},
    onArmDocumentScope = () => {},
    onIncludeContextChange = (_value: boolean) => {},
  } = $props();
  let targetDiagramId = $state("");
  let prompt = $state("");
  let error = $state("");
  let userMode: "auto" | "diagram" | "docs" | "rework" = $state("auto");

  const DIAGRAM_KEYWORDS = [
    "diagram", "class diagram", "sequence", "component", "use case",
    "uml", "bpmn", "archimate", "c4", "plantuml", "mermaid",
    "vizuali", "nakresli", "vygeneruj diagram", "entity", "er diagram", "erd",
    "activity", "state diagram", "deployment",
    "update", "uprav", "zmen", "pridaj", "odober", "refactor",
  ];

  const UPDATE_KEYWORDS = [
    "update", "uprav", "zmen", "pridaj", "odober", "refactor",
    "modify", "change", "add", "remove", "move", "extract", "split",
    "presun", "vytiahni", "separuj", "daj do",
    "zmaz", "zmaž", "delete", "odstan", "odstráň", "vyhoď", "vyhod",
    "premenuj", "rename",
  ];

  function detectIntent(text: string): "diagram" | "docs" {
    const lower = text.toLowerCase();
    if (DIAGRAM_KEYWORDS.some(kw => lower.includes(kw))) return "diagram";
    // If sections have existing diagrams and prompt looks like an update, treat as diagram
    const hasDiagrams = sections.some((s: any) => s.diagrams?.length > 0);
    if (hasDiagrams && UPDATE_KEYWORDS.some(kw => lower.includes(kw))) return "diagram";
    return "docs";
  }

  function detectDiagramType(text: string): string {
    const lower = text.toLowerCase();
    if (lower.includes("erd") || lower.includes("er ") || lower.includes("entity")) return "entity-relationship";
    if (lower.includes("class")) return "class";
    if (lower.includes("sequence")) return "sequence";
    if (lower.includes("component")) return "component";
    if (lower.includes("use case")) return "usecase";
    if (lower.includes("activity")) return "activity";
    if (lower.includes("state")) return "state";
    if (lower.includes("deployment")) return "deployment";
    if (lower.includes("bpmn")) return "bpmn";
    if (lower.includes("archimate")) return "archimate";
    if (lower.includes("c4")) return "c4";
    return "class";
  }

  // Ticking any block always means "rework it", regardless of what was last
  // clicked in the mode toggle — and it is never written back into `userMode`,
  // so clearing the selection returns to whatever the user had chosen before.
  let selectionActive = $derived(selection.sectionIds.length > 0 || selection.diagramIds.length > 0);
  // Deliberately inferred (not annotated to the "auto"|"diagram"|"docs"|"rework"
  // union): TypeScript's control-flow narrowing otherwise carries an exclusion
  // from one `=== "..."` comparison below into the next, which is wrong for a
  // reactive value that can hold any of the four at each read. The one place
  // that must see the precise union (the exhaustive switch) casts explicitly.
  let effectiveMode = $derived(selectionActive ? "rework" : userMode);
  // Whole-document rework is reachable only through App's `documentScopeArmed`
  // (set solely by a Rework click with nothing ticked, and dropped by every
  // toggle, clear and created request) — never inferred from an empty
  // selection. "none" means nothing is targeted and nothing may be sent; it is
  // typed in, not cast away, so a future scope value can't slip past the
  // exhaustive checks below unnoticed.
  let scope: "selection" | "document" | "none" = $derived(
    selectionActive ? "selection" : documentScopeArmed ? "document" : "none"
  );
  // Read once per mode instead of comparing `effectiveMode` inline throughout the markup below.
  let isReworkMode = $derived(effectiveMode === "rework");
  let isDiagramMode = $derived(effectiveMode === "diagram");
  let isDocsMode = $derived(effectiveMode === "docs");
  let isAutoMode = $derived(effectiveMode === "auto");
  let activeProvider = $derived(aiStatus ? currentProvider(aiStatus) : undefined);
  let reworkUnavailable = $derived(!aiStatus || !activeProvider || !activeProvider.available);
  let reworkBlockReason = $derived(
    aiStatusError ? aiStatusError
    : !aiStatus ? ""
    : !activeProvider ? "AI is not configured. Open AI settings to choose a provider."
    : !activeProvider.available ? activeProvider.detail
    : scope === "none" ? "Tick blocks, or click Rework to rework the whole document."
    : ""
  );
  let consent = $derived(aiStatus ? consentText(aiStatus, scope, includeContext) : "");
  let blocked = $derived(isReworkMode && (reworkUnavailable || scope === "none"));
  let promptPlaceholder = $derived(
    isDiagramMode ? "Describe a diagram... (e.g., 'class diagram for User entity with name, email, role')"
    : isDocsMode ? "Describe your system... (e.g., 'E-commerce platform with microservices and payment gateway')"
    : isReworkMode && scope !== "none" ? `Describe how to rework ${scope === "selection" ? "the selected content" : "the whole document"}...`
    : isReworkMode ? "Tick blocks, or click Rework again with nothing ticked, to choose what to rework..."
    : "Describe what you need — diagram or full documentation (auto-detected)..."
  );

  async function handleGenerate() {
    if (!prompt.trim() || isGenerating || blocked) return;

    isGenerating = true;
    error = "";
    // A session change (New/Open project, mid-flight) must not let this call's
    // `finally` clear a *later* generation's spinner or overwrite its error.
    const startedSession = session;

    const source: Section[] = JSON.parse(JSON.stringify(sections));
    const instruction = prompt;
    const language = selectedLanguage;
    const template = selectedTemplate;

    // Selection and userMode can only ever produce one of these four values (see
    // `effectiveMode` above); this local binding lets the switch below narrow on
    // it and reject a fifth case at compile time.
    const mode = effectiveMode as "auto" | "diagram" | "docs" | "rework";
    // Captured once so the "none" narrowing below survives into the call to
    // `buildReworkRequest`, which never accepts a "none" scope.
    const currentScope = scope;

    try {
      switch (mode) {
        case "auto":
        case "diagram":
        case "docs": {
          const intent = mode === "auto" ? detectIntent(instruction) : mode;
          if (intent === "diagram") {
            let diagramType = detectDiagramType(instruction);
            const lower = instruction.toLowerCase();
            const isUpdate = UPDATE_KEYWORDS.some(kw => lower.includes(kw));
            const candidates = source.flatMap(s => s.diagrams);
            let target: Diagram | undefined;
            if (isUpdate && candidates.length) {
              target = candidates.find(d => d.id === targetDiagramId) ?? (candidates.length === 1 ? candidates[0] : undefined);
              if (!target) throw new Error("Select the diagram to update before generating.");
              diagramType = target.diagram_type;
            }

            await onGenerate({
              kind: "diagram",
              description: instruction,
              diagramType: diagramType,
              language,
              sectionId: (target ? source.find(s => s.diagrams.some(d => d.id === target!.id))?.id : source[0]?.id) ?? null,
              diagramId: target?.id ?? null,
            });
          } else {
            // Generate full documentation
            await onGenerate({
              kind: "documentation",
              description: instruction,
              template,
              language,
            });
          }
          break;
        }
        case "rework": {
          if (!aiStatus) throw new Error("AI status is not loaded yet.");
          // Unreachable in practice (`blocked` already disables the button and
          // Enter for scope "none"); kept as the hard stop the contract
          // requires so a future change to `blocked` can never let scope
          // "none" reach the wire.
          if (currentScope === "none") throw new Error("Tick blocks, or click Rework to rework the whole document.");
          await onGenerate(buildReworkRequest(selection, currentScope, instruction, language, includeContext, aiStatus));
          break;
        }
        default: {
          const exhaustive: never = mode;
          throw new Error(`Unhandled generation mode: ${exhaustive}`);
        }
      }
    } catch (err: any) {
      if (startedSession === session) { error = String(err); console.error("Generation failed:", err); }
    } finally {
      if (startedSession === session) isGenerating = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleGenerate();
    }
  }
</script>

<div class="prompt-panel">
  {#if !isReworkMode && sections.some((s: Section) => s.diagrams.length > 0)}
    <label>Diagram to update
      <select aria-label="Diagram to update" bind:value={targetDiagramId}>
        <option value="">Select a diagram (required if there are several)</option>
        {#each sections as section}
          {#each section.diagrams as diagram, i}
            <option value={diagram.id}>{section.title} — {diagram.diagram_type} {i + 1}</option>
          {/each}
        {/each}
      </select>
    </label>
  {/if}
  {#if isReworkMode}
    <div class="rework-bar">
      <span class="selection-summary">{selectionActive ? selectionSummary(selection) : scope === "document" ? "Whole document" : "Nothing selected"}</span>
      {#if selectionActive}<button type="button" onclick={() => onClearSelection()}>Clear selection</button>{/if}
      {#if activeProvider}<span class="provider-chip">{providerChip(activeProvider)}</span>{/if}
      <button type="button" onclick={() => onOpenAiSettings()}>AI settings</button>
    </div>
    <p id="rework-consent" class="rework-consent">{consent}</p>
    {#if scope === "selection"}
      <label class="include-context"><input type="checkbox" checked={includeContext} onchange={(e) => onIncludeContextChange(e.currentTarget.checked)} /> Include the rest of the document as read-only context</label>
    {/if}
    {#if reworkBlockReason}<p class="rework-blocked">{reworkBlockReason}</p>{/if}
  {/if}
  <div class="prompt-container">
    <textarea
      class="prompt-input"
      bind:value={prompt}
      placeholder={promptPlaceholder}
      rows="2"
      onkeydown={handleKeydown}
    ></textarea>
    <button
      class="generate-btn"
      onclick={handleGenerate}
      disabled={isGenerating || !prompt.trim() || blocked}
      aria-describedby={isReworkMode ? "rework-consent" : undefined}
    >
      {#if isGenerating}
        Generating...
      {:else}
        Generate
      {/if}
    </button>
  </div>
  {#if error}
    <div class="error-bar">
      <span>{error}</span>
      <button class="error-dismiss" onclick={() => error = ""}>×</button>
    </div>
  {/if}
  <div class="prompt-hints">
    <div class="mode-toggle">
      <button class="mode-opt" class:active={isAutoMode} disabled={selectionActive} title={selectionActive ? "Clear selection to use Docs/Diagram" : undefined} onclick={() => userMode = "auto"}>Auto</button>
      <button class="mode-opt" class:active={isDiagramMode} disabled={selectionActive} title={selectionActive ? "Clear selection to use Docs/Diagram" : undefined} onclick={() => userMode = "diagram"}>Diagram</button>
      <button class="mode-opt" class:active={isDocsMode} disabled={selectionActive} title={selectionActive ? "Clear selection to use Docs/Diagram" : undefined} onclick={() => userMode = "docs"}>Docs</button>
      <button class="mode-opt" class:active={isReworkMode} onclick={() => { userMode = "rework"; if (!selectionActive) onArmDocumentScope(); }}>Rework</button>
    </div>
    <span>Enter to generate</span>
    <span>Shift+Enter for new line</span>
    <span>Template: <strong>{selectedTemplate}</strong></span>
    {#if !isReworkMode}<span>Local Ollama (Python engine)</span>{/if}
  </div>
</div>

<style>
  .prompt-panel {
    border-top: 1px solid var(--border);
    background: var(--bg-secondary);
    padding: 16px 32px;
  }

  .prompt-container {
    display: flex;
    gap: 12px;
    max-width: 900px;
    margin: 0 auto;
    align-items: flex-end;
  }

  .prompt-input {
    flex: 1;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-primary);
    padding: 12px 16px;
    font-size: 14px;
    line-height: 1.5;
    resize: none;
    transition: border-color 0.15s;
  }

  .prompt-input::placeholder {
    color: var(--text-muted);
  }

  .prompt-input:focus {
    border-color: var(--accent);
  }

  .generate-btn {
    padding: 12px 24px;
    background: var(--accent);
    color: white;
    border-radius: var(--radius);
    font-size: 14px;
    font-weight: 600;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .generate-btn:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .generate-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .prompt-hints {
    display: flex;
    gap: 16px;
    max-width: 900px;
    margin: 8px auto 0;
    font-size: 11px;
    color: var(--text-muted);
  }

  .prompt-hints strong {
    color: var(--accent);
  }

  .error-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    max-width: 900px;
    margin: 8px auto 0;
    padding: 8px 12px;
    background: rgba(240, 96, 96, 0.12);
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    color: var(--danger);
    font-size: 12px;
    word-break: break-word;
  }

  .mode-toggle {
    display: flex;
    gap: 2px;
    background: var(--bg-tertiary);
    border-radius: 5px;
    padding: 2px;
    margin-right: 8px;
  }

  .mode-opt {
    padding: 2px 10px;
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 500;
    border-radius: 4px;
    transition: all 0.15s;
  }

  .mode-opt:hover {
    color: var(--text-secondary);
  }

  .mode-opt.active {
    background: var(--accent);
    color: white;
  }

  .mode-opt:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .error-dismiss {
    background: transparent;
    color: var(--danger);
    font-size: 16px;
    padding: 0 4px;
    flex-shrink: 0;
    margin-left: 8px;
  }

  .rework-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 900px;
    margin: 0 auto 8px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .rework-bar button {
    padding: 4px 10px;
    font-size: 11px;
    color: var(--text-primary);
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    border-radius: 4px;
  }

  .provider-chip {
    padding: 2px 8px;
    font-size: 11px;
    color: var(--text-muted);
    background: var(--bg-tertiary);
    border-radius: 4px;
  }

  .rework-consent, .rework-blocked {
    max-width: 900px;
    margin: 0 auto 8px;
    font-size: 12px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .rework-blocked {
    color: var(--danger);
  }

  .include-context {
    display: block;
    max-width: 900px;
    margin: 0 auto 8px;
    font-size: 12px;
    color: var(--text-secondary);
  }
</style>
