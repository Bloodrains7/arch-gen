<script lang="ts">
  import { CUSTOM_TEMPLATE, DOCUMENT_TEMPLATES } from "../templates";

  let {
    selectedTemplate = "arc42",
    selectedLanguage = "en",
    onApplyTemplate = (_id: string) => {},
    onLanguageChange = (_l: string) => {},
  } = $props();

  const languages = [
    { id: "en", name: "English", desc: "Full English documentation" },
    { id: "sk", name: "Slovensky", desc: "SK text, EN technical terms" },
  ];

  const templateGroups = (["Architecture", "Design & decisions", "Delivery"] as const).map(group => ({
    group,
    templates: [...DOCUMENT_TEMPLATES.filter(t => t.group === group), ...(group === "Design & decisions" ? [CUSTOM_TEMPLATE] : [])],
  }));

  const diagramTypes = [
    { id: "uml-sequence", label: "Sequence", icon: "↕", type: "uml" },
    { id: "uml-class", label: "Class", icon: "◫", type: "uml" },
    { id: "uml-component", label: "Component", icon: "⧈", type: "uml" },
    { id: "uml-usecase", label: "Use Case", icon: "◯", type: "uml" },
    { id: "bpmn-process", label: "BPMN Process", icon: "▷", type: "bpmn" },
    { id: "archimate", label: "ArchiMate", icon: "△", type: "archimate" },
    { id: "c4-context", label: "C4 Context", icon: "◎", type: "c4" },
    { id: "c4-container", label: "C4 Container", icon: "▣", type: "c4" },
  ];

  const taskTemplates = [
    { id: "user-story", label: "User Story", icon: "◆", kind: "task" },
    { id: "tech-task", label: "Tech Task", icon: "⚙", kind: "task" },
    { id: "bug-report", label: "Bug Report", icon: "⚠", kind: "task" },
    { id: "spike", label: "Spike / Research", icon: "◈", kind: "task" },
    { id: "adr", label: "Decision (ADR)", icon: "◇", kind: "task" },
    { id: "api-spec", label: "API Spec", icon: "⬡", kind: "task" },
    { id: "db-migration", label: "DB Migration", icon: "▦", kind: "task" },
    { id: "meeting-notes", label: "Meeting Notes", icon: "▤", kind: "task" },
    { id: "notes", label: "Notes / Scratchpad", icon: "▧", kind: "task" },
  ];

  function handleDragStart(e: DragEvent, item: any) {
    e.dataTransfer!.setData("application/json", JSON.stringify(item));
    e.dataTransfer!.effectAllowed = "copy";
  }

</script>

<aside class="sidebar">
  <div class="logo">
    <span class="logo-icon">◇</span>
    <span class="logo-text">ArchGen</span>
  </div>

  <div class="section">
    <h3 class="section-title">Template</h3>
    <p class="section-hint">Adds the structure; content you wrote is kept.</p>
    {#each templateGroups as { group, templates } (group)}
      <h4 class="template-group">{group}</h4>
      <div class="template-list">
        {#each templates as tmpl (tmpl.id)}
          <button
            class="template-item"
            class:active={selectedTemplate === tmpl.id}
            onclick={() => onApplyTemplate(tmpl.id)}
          >
            <span class="template-name">{tmpl.name}</span>
            <span class="template-desc">{tmpl.desc}</span>
          </button>
        {/each}
      </div>
    {/each}
  </div>

  <div class="section">
    <h3 class="section-title">Language</h3>
    <div class="lang-toggle">
      {#each languages as lang}
        <button
          class="lang-opt"
          class:active={selectedLanguage === lang.id}
          onclick={() => onLanguageChange(lang.id)}
          title={lang.desc}
        >{lang.name}</button>
      {/each}
    </div>
  </div>

  <div class="section">
    <h3 class="section-title">Diagrams</h3>
    <p class="section-hint">Drag into a section</p>
    <div class="diagram-grid">
      {#each diagramTypes as diagram}
        <div
          class="diagram-chip"
          draggable="true"
          ondragstart={(e) => handleDragStart(e, diagram)}
          role="button"
          tabindex="0"
        >
          <span class="chip-icon">{diagram.icon}</span>
          <span class="chip-label">{diagram.label}</span>
        </div>
      {/each}
    </div>
  </div>

  <div class="section">
    <h3 class="section-title">Dev Tasks</h3>
    <p class="section-hint">Drag into a section</p>
    <div class="diagram-grid">
      {#each taskTemplates as task}
        <div
          class="diagram-chip task-chip"
          draggable="true"
          ondragstart={(e) => handleDragStart(e, task)}
          role="button"
          tabindex="0"
        >
          <span class="chip-icon task-icon">{task.icon}</span>
          <span class="chip-label">{task.label}</span>
        </div>
      {/each}
    </div>
  </div>

</aside>

<style>
  .sidebar {
    width: 260px;
    background: var(--bg-secondary);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    flex-shrink: 0;
  }

  .logo {
    padding: 20px;
    display: flex;
    align-items: center;
    gap: 10px;
    border-bottom: 1px solid var(--border);
  }

  .logo-icon {
    font-size: 24px;
    color: var(--accent);
  }

  .logo-text {
    font-size: 18px;
    font-weight: 700;
    letter-spacing: -0.5px;
  }

  .section {
    padding: 16px 20px;
    border-bottom: 1px solid var(--border);
  }

  .section-title {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 1px;
    color: var(--text-muted);
    margin-bottom: 10px;
    font-weight: 600;
  }

  .section-hint {
    font-size: 11px;
    color: var(--text-muted);
    margin-bottom: 10px;
  }

  .template-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .template-item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 8px 10px;
    background: transparent;
    border-radius: var(--radius);
    color: var(--text-secondary);
    text-align: left;
    transition: all 0.15s;
  }

  .template-item:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .template-item.active {
    background: var(--accent-dim);
    color: var(--accent);
  }

  .template-name {
    font-size: 13px;
    font-weight: 500;
  }

  .template-desc {
    font-size: 11px;
    color: var(--text-muted);
    margin-top: 2px;
  }

  .template-item.active .template-desc {
    color: var(--accent);
    opacity: 0.7;
  }

  .diagram-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }

  .diagram-chip {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 8px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    cursor: grab;
    transition: all 0.15s;
    user-select: none;
  }

  .diagram-chip:hover {
    border-color: var(--accent);
    background: var(--accent-dim);
  }

  .diagram-chip:active {
    cursor: grabbing;
    transform: scale(0.96);
  }

  .chip-icon {
    font-size: 14px;
    color: var(--accent);
  }

  .chip-label {
    font-size: 11px;
    color: var(--text-secondary);
    font-weight: 500;
  }

  .template-group {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
    margin: 10px 0 4px;
  }

  .task-chip:hover {
    border-color: var(--success);
    background: rgba(76, 206, 138, 0.1);
  }

  .task-icon {
    color: var(--success) !important;
  }

  .lang-toggle {
    display: flex;
    gap: 4px;
    background: var(--bg-tertiary);
    border-radius: var(--radius);
    padding: 3px;
  }

  .lang-opt {
    flex: 1;
    padding: 6px 10px;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 500;
    border-radius: 4px;
    transition: all 0.15s;
  }

  .lang-opt:hover {
    color: var(--text-secondary);
  }

  .lang-opt.active {
    background: var(--accent);
    color: white;
  }
</style>
