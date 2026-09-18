<script lang="ts">
  import type { Section, Diagram } from "../project";
  import type { GenerationRequest } from "../runtime";

  let {
    sections = [],
    isGenerating = $bindable(false),
    selectedTemplate,
    selectedLanguage = "en",
    onGenerate = async (_request: GenerationRequest) => {},
  } = $props();
  let targetDiagramId = $state("");
  let prompt = $state("");
  let error = $state("");
  let mode: "auto" | "diagram" | "docs" = $state("auto");

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

  async function handleGenerate() {
    if (!prompt.trim() || isGenerating) return;

    isGenerating = true;
    error = "";

    const intent = mode === "auto" ? detectIntent(prompt) : mode;
    const source: Section[] = JSON.parse(JSON.stringify(sections));
    const instruction = prompt;
    const language = selectedLanguage;
    const template = selectedTemplate;

    try {
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
    } catch (err: any) {
      error = String(err);
      console.error("Generation failed:", err);
    } finally {
      isGenerating = false;
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
  {#if sections.some((s: Section) => s.diagrams.length > 0)}
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
  <div class="prompt-container">
    <textarea
      class="prompt-input"
      bind:value={prompt}
      placeholder={mode === "diagram"
        ? "Describe a diagram... (e.g., 'class diagram for User entity with name, email, role')"
        : mode === "docs"
        ? "Describe your system... (e.g., 'E-commerce platform with microservices and payment gateway')"
        : "Describe what you need — diagram or full documentation (auto-detected)..."
      }
      rows="2"
      onkeydown={handleKeydown}
    ></textarea>
    <button
      class="generate-btn"
      onclick={handleGenerate}
      disabled={isGenerating || !prompt.trim()}
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
      <button class="mode-opt" class:active={mode === "auto"} onclick={() => mode = "auto"}>Auto</button>
      <button class="mode-opt" class:active={mode === "diagram"} onclick={() => mode = "diagram"}>Diagram</button>
      <button class="mode-opt" class:active={mode === "docs"} onclick={() => mode = "docs"}>Docs</button>
    </div>
    <span>Enter to generate</span>
    <span>Shift+Enter for new line</span>
    <span>Template: <strong>{selectedTemplate}</strong></span>
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

  .error-dismiss {
    background: transparent;
    color: var(--danger);
    font-size: 16px;
    padding: 0 4px;
    flex-shrink: 0;
    margin-left: 8px;
  }
</style>
