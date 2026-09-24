<script lang="ts">
  import MonacoEditor from "./MonacoEditor.svelte";
  import MarkdownPreview from "./MarkdownPreview.svelte";
  import PlantUMLPreview from "./PlantUMLPreview.svelte";
  import type { Section } from "../project";
  import type { GenerationRequest } from "../runtime";
  import { isSectionSelected, isDiagramSelected, emptySelection } from "../selection";
  import { buildDiagramRequest, currentProvider } from "../ai";
  import type { AiStatus } from "../ai";
  import { confirm } from "@tauri-apps/plugin-dialog";

  let {
    sections = [],
    documentName = "Architecture Documentation",
    isGenerating,
    selectedLanguage = "en",
    selection = emptySelection(),
    aiStatus = null as AiStatus | null,
    onSectionsChange = (_s: any[], _group?: string) => {},
    onGenerate = async (_request: GenerationRequest) => {},
    onError = (_message: string) => {},
    onToggleSection = (_id: string) => {},
    onToggleDiagram = (_sectionId: string, _diagramId: string) => {},
  } = $props();

  // Dropping a diagram chip sends its description straight to `onGenerate`, with no
  // instruction panel in between to show the consent sentence — so this is where that
  // consent actually has to be asked, before anything reaches a non-local provider. The
  // local provider needs no dialog (nothing leaves this computer).
  async function consentToSendDiagram(description: string): Promise<boolean> {
    const provider = aiStatus ? currentProvider(aiStatus) : undefined;
    if (!provider || provider.local) return true;
    return await confirm(
      `Sends "${description}" as the diagram description, with the document name and template, to ${provider.recipient} via ${provider.label}. Continue?`,
      { title: "Send to AI provider", kind: "warning" },
    );
  }
  let dragOverIndex: number | null = $state(null);
  let previewMode: "edit" | "preview" | "split" = $state("edit");
  let sectionMode: Record<number, "edit" | "preview"> = $state({});

  function handleDragOver(e: DragEvent, index: number) {
    e.preventDefault();
    e.dataTransfer!.dropEffect = "copy";
    dragOverIndex = index;
  }

  function handleDragLeave(e: DragEvent) {
    const target = e.currentTarget as HTMLElement;
    const related = e.relatedTarget as HTMLElement | null;
    if (related && target.contains(related)) return;
    dragOverIndex = null;
  }

  const taskTemplateContent: Record<string, { title: string; content: string }> = {
    "user-story": {
      title: "User Story",
      content: `**As a** [role]
**I want** [feature/action]
**So that** [benefit/value]

### Acceptance Criteria
- [ ] Given [context], when [action], then [expected result]
- [ ] Given [context], when [action], then [expected result]

### Definition of Done
- [ ] Code reviewed
- [ ] Unit tests written and passing
- [ ] Integration tests passing
- [ ] Documentation updated
- [ ] Deployed to staging

### Story Points: _
### Priority: _
### Sprint: _`,
    },
    "tech-task": {
      title: "Technical Task",
      content: `### Summary
[Brief description of the technical work]

### Background / Context
[Why is this needed? What problem does it solve?]

### Technical Approach
1. [Step 1]
2. [Step 2]
3. [Step 3]

### Affected Components
- [ ] Component A
- [ ] Component B

### Dependencies
- [Dependency 1]

### Risks / Considerations
- [Risk 1]

### Estimation: _ hours
### Priority: _`,
    },
    "bug-report": {
      title: "Bug Report",
      content: `### Summary
[One-line description of the bug]

### Environment
- **Version:**
- **OS:**
- **Browser/Runtime:**

### Steps to Reproduce
1. [Step 1]
2. [Step 2]
3. [Step 3]

### Expected Behavior
[What should happen]

### Actual Behavior
[What actually happens]

### Screenshots / Logs
[Attach evidence]

### Severity: [ ] Critical  [ ] High  [ ] Medium  [ ] Low
### Assigned to: _`,
    },
    "spike": {
      title: "Spike / Research",
      content: `### Research Question
[What do we need to find out?]

### Background
[Why is this research needed?]

### Scope
- **In scope:**
- **Out of scope:**

### Approach
1. [Research step 1]
2. [Prototype / PoC]
3. [Evaluate results]

### Success Criteria
- [ ] [Question answered with evidence]
- [ ] [Decision document created]
- [ ] [PoC demonstrated to team]

### Time-box: _ hours/days
### Output: [Document / PoC / ADR]`,
    },
    "adr": {
      title: "ADR-NNNN: [short title of the decision]",
      content: `**Status:** proposed | accepted | rejected | superseded by ADR-NNNN
**Date:** [YYYY-MM-DD] · **Deciders:** [names]

### Context and Problem Statement
[The situation and the question to decide, in two or three sentences.]

### Decision Drivers
- [Driver, e.g. a quality goal or constraint]

### Considered Options
1. [Option 1]
2. [Option 2]

### Decision Outcome
Chosen option: "[option]", because [justification tied to the drivers].

### Consequences
- Good, because [...]
- Bad, because [...]

### Confirmation
[How compliance with this decision is checked.]`,
    },
    "api-spec": {
      title: "API Specification",
      content: `### Endpoint
\`[METHOD] /api/v1/[resource]\`

### Description
[What does this endpoint do?]

### Request
**Headers:**
| Header | Value | Required |
|--------|-------|----------|
| Authorization | Bearer {token} | Yes |

**Path Parameters:**
| Param | Type | Description |
|-------|------|-------------|
| id | string | Resource ID |

**Request Body:**
\`\`\`json
{
  "field": "value"
}
\`\`\`

### Response
**200 OK:**
\`\`\`json
{
  "id": "...",
  "status": "success"
}
\`\`\`

**Error Codes:**
| Code | Description |
|------|-------------|
| 400 | Bad Request |
| 401 | Unauthorized |
| 404 | Not Found |

### Rate Limiting: _
### Authentication: _`,
    },
    "meeting-notes": {
      title: "Meeting Notes",
      content: `### Meeting Title
[Subject]

**Date:** [YYYY-MM-DD]
**Time:** [HH:MM - HH:MM]
**Location / Call:** [Room / Link]

### Attendees
| Name | Role | Present |
|------|------|---------|
| | | Yes/No |

### Agenda
1. [Topic 1]
2. [Topic 2]
3. [Topic 3]

### Discussion Notes
#### Topic 1
- [Key point]
- [Decision made]

#### Topic 2
- [Key point]
- [Open question]

### Decisions Made
| # | Decision | Owner | Deadline |
|---|----------|-------|----------|
| 1 | | | |

### Action Items
- [ ] [Action] - **Owner:** [Name] - **Due:** [Date]
- [ ] [Action] - **Owner:** [Name] - **Due:** [Date]

### Next Steps
- [Follow-up meeting date]
- [Items to prepare]

### Parking Lot
- [Deferred topics for future discussion]`,
    },
    "notes": {
      title: "Notes / Scratchpad",
      content: `### Quick Notes

> Use this section for raw ideas, observations, and references.
> These notes can be used to generate documentation — select the content and use the prompt to transform it.

---

#### Ideas
-

#### References
-

#### Questions to Resolve
- [ ]
- [ ]

#### Key Decisions
-

---
*Tip: Write your notes here, then use the prompt panel to generate structured documentation from them.*`,
    },
    "db-migration": {
      title: "Database Migration",
      content: `### Migration Name
[Descriptive name]

### Type: [ ] Schema Change  [ ] Data Migration  [ ] Index  [ ] Seed Data

### Changes
\`\`\`sql
-- UP
ALTER TABLE ...

-- DOWN (rollback)
ALTER TABLE ...
\`\`\`

### Affected Tables
| Table | Change | Impact |
|-------|--------|--------|
| | | |

### Data Impact
- **Rows affected (estimate):**
- **Downtime required:** Yes / No
- **Backward compatible:** Yes / No

### Rollback Plan
1. [Step 1]
2. [Step 2]

### Pre-migration Checklist
- [ ] Backup taken
- [ ] Tested on staging
- [ ] Team notified
- [ ] Monitoring in place`,
    },
  };

  async function handleDrop(e: DragEvent, sectionIndex: number) {
    e.preventDefault();
    dragOverIndex = null;

    const data = e.dataTransfer!.getData("application/json");
    if (!data) return;

    const item = JSON.parse(data);

    // Handle task template drops
    if (item.kind === "task") {
      const template = taskTemplateContent[item.id];
      if (template) {
        const newSection = {
          title: template.title,
          content: template.content,
          diagrams: [],
        };
        // Insert after the current section
        onSectionsChange([
          ...sections.slice(0, sectionIndex + 1),
          newSection,
          ...sections.slice(sectionIndex + 1),
        ]);
      }
      return;
    }

    // Handle diagram drops
    if (!aiStatus) { onError("AI status is not loaded yet."); return; }
    if (!(await consentToSendDiagram(sections[sectionIndex].title))) return;
    const source: Section[] = JSON.parse(JSON.stringify(sections));
    try {
      await onGenerate(buildDiagramRequest(
        source[sectionIndex].title,
        item.id?.startsWith("uml-") ? item.id.slice(4) : item.id?.startsWith("c4-") ? item.id.replace("c4-", "c4_") : item.type,
        selectedLanguage,
        source[sectionIndex].id ?? null, null,
        aiStatus,
      ));
    } catch (err) {
      onError(`Failed to generate diagram: ${err}`);
      console.error("Failed to generate diagram:", err);
    }
  }

  function removeSection(index: number) {
    onSectionsChange(sections.filter((_: any, i: number) => i !== index));
  }

  function removeDiagram(sectionIndex: number, diagramIndex: number) {
    const updated = [...sections];
    updated[sectionIndex] = {
      ...updated[sectionIndex],
      diagrams: updated[sectionIndex].diagrams.filter((_: any, i: number) => i !== diagramIndex),
    };
    onSectionsChange(updated);
  }

  async function handleDropOnEmpty(e: DragEvent) {
    e.preventDefault();
    dragOverIndex = null;
    const data = e.dataTransfer!.getData("application/json");
    if (!data) return;
    const item = JSON.parse(data);

    if (item.kind === "task") {
      const template = taskTemplateContent[item.id];
      if (template) {
        onSectionsChange([{ title: template.title, content: template.content, diagrams: [] }]);
      }
    } else {
      if (!aiStatus) { onError("AI status is not loaded yet."); return; }
      if (!(await consentToSendDiagram(item.label))) return;
      try {
        await onGenerate(buildDiagramRequest(
          item.label,
          item.id?.startsWith("uml-") ? item.id.slice(4) : item.id?.startsWith("c4-") ? item.id.replace("c4-", "c4_") : item.type,
          selectedLanguage,
          null, null,
          aiStatus,
        ));
      } catch (err) {
        onError(`Failed to generate diagram: ${err}`);
        console.error("Failed to generate diagram:", err);
      }
    }
  }

  function addSection() {
    onSectionsChange([...sections, { title: "New Section", content: "", diagrams: [] }]);
  }

  function updateTitle(index: number, e: Event) {
    const target = e.target as HTMLElement;
    const updated = [...sections];
    updated[index] = { ...updated[index], title: target.textContent || "" };
    onSectionsChange(updated, `title:${sections[index].id}`);
  }

  // Browsers can replace the text node inside contenteditable. Updating the
  // element itself keeps external undo/restore in sync without moving the caret
  // during ordinary typing (the DOM already has the incoming value then).
  function editableTitle(node: HTMLElement, value: string) {
    const sync = (title: string) => { if (node.textContent !== title) node.textContent = title; };
    sync(value);
    return { update: sync };
  }

  function updateContent(index: number, val: string) {
    const updated = [...sections];
    updated[index] = { ...updated[index], content: val };
    onSectionsChange(updated, `content:${sections[index].id}`);
  }
</script>

<div class="canvas">
  {#if sections.length === 0}
    <div
      class="empty-state"
      class:drag-over-empty={dragOverIndex === -1}
      ondragover={(e) => { e.preventDefault(); dragOverIndex = -1; }}
      ondragleave={() => dragOverIndex = null}
      ondrop={(e) => handleDropOnEmpty(e)}
      role="region"
    >
      <div class="empty-icon">◇</div>
      <h2>Start your architecture documentation</h2>
      <p>Select a template, describe your system below, or drag a task/diagram here.</p>
    </div>
  {:else}
    <div class="view-toggle">
      <button class="view-btn" class:active={previewMode === "edit"} onclick={() => previewMode = "edit"}>Edit</button>
      <button class="view-btn" class:active={previewMode === "split"} onclick={() => previewMode = "split"}>Split</button>
      <button class="view-btn" class:active={previewMode === "preview"} onclick={() => previewMode = "preview"}>Preview</button>
    </div>

    {#if previewMode === "preview"}
      <div class="preview-document">
        <h1 class="doc-title">{documentName}</h1>
        {#each sections as section, i (section.id)}
          <div class="preview-section">
            <MarkdownPreview content={`## ${section.title}\n\n${section.content}`} />
            {#if section.diagrams.length > 0}
              {#each section.diagrams as diagram}
                <PlantUMLPreview content={diagram.content} format={diagram.format} />
              {/each}
            {/if}
          </div>
        {/each}
      </div>
    {:else}
      <div class="sections">
        {#each sections as section, i (section.id)}
          <div
            class="section-card"
            class:drag-over={dragOverIndex === i}
            class:selected={isSectionSelected(selection, section.id ?? "")}
            ondragover={(e) => handleDragOver(e, i)}
            ondragleave={(e) => handleDragLeave(e)}
            ondrop={(e) => handleDrop(e, i)}
            role="region"
          >
            <div class="section-header">
              <input
                type="checkbox"
                class="block-select"
                aria-label={`Select section ${i + 1}: ${section.title || "untitled"} for AI`}
                checked={isSectionSelected(selection, section.id ?? "")}
                onchange={() => onToggleSection(section.id ?? "")}
              />
              <h2
                class="section-title"
                contenteditable="true"
                aria-label={section.title || "Section title"}
                use:editableTitle={section.title}
                oninput={(e) => updateTitle(i, e)}
              ></h2>
              <div class="section-actions">
                <button
                  class="mode-btn"
                  class:active={sectionMode[i] === "preview"}
                  onclick={() => sectionMode[i] = sectionMode[i] === "preview" ? "edit" : "preview"}
                  title="Toggle preview"
                >{sectionMode[i] === "preview" ? "Edit" : "Preview"}</button>
                <button class="remove-btn" onclick={() => removeSection(i)} title="Remove section">×</button>
              </div>
            </div>

            {#if previewMode === "split"}
              <div class="split-view">
                <div class="split-editor">
                  <MonacoEditor value={section.content} language="markdown" onChange={(v: string) => updateContent(i, v)} />
                </div>
                <div class="split-preview">
                  <MarkdownPreview content={section.content} />
                </div>
              </div>
            {:else if sectionMode[i] === "preview"}
              <MarkdownPreview content={section.content} />
            {:else}
              <MonacoEditor value={section.content} language="markdown" onChange={(v: string) => updateContent(i, v)} />
            {/if}

            {#if section.diagrams.length > 0}
              <div class="diagram-list">
                {#each section.diagrams as diagram, di}
                  <div class="diagram-block" class:selected={isDiagramSelected(selection, diagram.id ?? "") || isSectionSelected(selection, section.id ?? "")}>
                    <div class="diagram-remove">
                      <input
                        type="checkbox"
                        class="block-select"
                        aria-label={`Select diagram ${di + 1} (${diagram.diagram_type}) in section ${i + 1}: ${section.title} for AI`}
                        checked={isDiagramSelected(selection, diagram.id ?? "") || isSectionSelected(selection, section.id ?? "")}
                        disabled={isSectionSelected(selection, section.id ?? "")}
                        onchange={() => onToggleDiagram(section.id ?? "", diagram.id ?? "")}
                      />
                      <button class="remove-btn small" onclick={() => removeDiagram(i, di)}>×</button>
                    </div>
                    <PlantUMLPreview content={diagram.content} format={diagram.format} />
                  </div>
                {/each}
              </div>
            {/if}

            <div class="drop-hint" class:visible={dragOverIndex === i}>
              Drop diagram here
            </div>
          </div>
        {/each}

        <button class="add-section-btn" onclick={addSection}>
          + Add Section
        </button>
      </div>
    {/if}
  {/if}

  {#if isGenerating}
    <div class="generating-overlay">
      <div class="spinner"></div>
      <span>Generating documentation...</span>
    </div>
  {/if}
</div>

<style>
  .canvas {
    flex: 1;
    overflow-y: auto;
    padding: 24px 32px;
    position: relative;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
    text-align: center;
    gap: 12px;
  }

  .empty-icon {
    font-size: 48px;
    color: var(--accent);
    opacity: 0.3;
  }

  .empty-state h2 {
    font-size: 20px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .empty-state p {
    font-size: 14px;
    max-width: 400px;
    line-height: 1.5;
  }

  .drag-over-empty {
    border: 2px dashed var(--accent);
    border-radius: var(--radius-lg);
    background: var(--accent-dim);
  }

  .sections {
    display: flex;
    flex-direction: column;
    gap: 16px;
    max-width: 900px;
    margin: 0 auto;
    padding-bottom: 120px;
  }

  .section-card {
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 20px;
    transition: all 0.2s;
    position: relative;
  }

  .section-card.drag-over {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-dim);
  }

  .section-card.selected, .diagram-block.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-dim);
  }

  .block-select {
    flex-shrink: 0;
    accent-color: var(--accent);
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: space-between;
    margin-bottom: 12px;
  }

  .section-title {
    flex: 1;
    font-size: 16px;
    font-weight: 600;
    color: var(--text-primary);
    outline: none;
    padding: 2px 4px;
    border-radius: 4px;
    transition: background 0.15s;
  }

  .section-title:focus {
    background: var(--bg-tertiary);
  }

  .remove-btn {
    background: transparent;
    color: var(--text-muted);
    font-size: 18px;
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius);
    transition: all 0.15s;
  }

  .remove-btn:hover {
    background: var(--danger);
    color: white;
  }

  .remove-btn.small {
    font-size: 14px;
    width: 22px;
    height: 22px;
  }

  .diagram-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 12px;
  }

  .diagram-block {
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .diagram-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 10px;
    background: var(--bg-tertiary);
    border-bottom: 1px solid var(--border);
  }

  .diagram-type {
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .diagram-code {
    padding: 12px;
    font-size: 12px;
    font-family: 'Cascadia Code', 'Fira Code', monospace;
    color: var(--text-secondary);
    overflow-x: auto;
    line-height: 1.5;
    white-space: pre-wrap;
  }

  .drop-hint {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(108, 140, 255, 0.06);
    border-radius: var(--radius-lg);
    color: var(--accent);
    font-size: 14px;
    font-weight: 500;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.2s;
  }

  .drop-hint.visible {
    opacity: 1;
  }

  .add-section-btn {
    padding: 12px;
    background: transparent;
    border: 1px dashed var(--border);
    border-radius: var(--radius-lg);
    color: var(--text-muted);
    font-size: 13px;
    font-weight: 500;
    transition: all 0.15s;
  }

  .add-section-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
    background: var(--accent-dim);
  }

  .generating-overlay {
    position: absolute;
    inset: 0;
    background: rgba(15, 17, 23, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--accent);
    font-size: 14px;
    font-weight: 500;
    backdrop-filter: blur(4px);
    z-index: 10;
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* View toggle bar */
  .view-toggle {
    display: flex;
    gap: 2px;
    margin-bottom: 16px;
    max-width: 900px;
    margin-left: auto;
    margin-right: auto;
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 3px;
    width: fit-content;
  }

  .view-btn {
    padding: 5px 16px;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
    font-weight: 500;
    border-radius: 5px;
    transition: all 0.15s;
  }

  .view-btn:hover {
    color: var(--text-secondary);
  }

  .view-btn.active {
    background: var(--accent);
    color: white;
  }

  /* Section actions row */
  .section-actions {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .mode-btn {
    padding: 3px 10px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    border-radius: 5px;
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 500;
    transition: all 0.15s;
  }

  .mode-btn:hover {
    border-color: var(--accent);
    color: var(--text-primary);
  }

  .mode-btn.active {
    background: var(--accent-dim);
    border-color: var(--accent);
    color: var(--accent);
  }

  /* Split view */
  .split-view {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    min-height: 150px;
  }

  .split-editor, .split-preview {
    min-width: 0;
    overflow: hidden;
  }

  /* Full preview mode */
  .preview-document {
    max-width: 900px;
    margin: 0 auto;
    padding-bottom: 120px;
  }

  .doc-title {
    font-size: 24px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 24px;
    padding-bottom: 12px;
    border-bottom: 2px solid var(--accent);
  }

  .preview-section {
    margin-bottom: 20px;
  }

  .preview-diagram {
    background: var(--bg-secondary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    margin: 12px 0;
    overflow: hidden;
  }

  .diagram-remove {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-end;
    padding: 4px 4px 0;
  }

  .diagram-label {
    padding: 6px 10px;
    font-size: 11px;
    font-weight: 600;
    color: var(--accent);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    background: var(--bg-tertiary);
    border-bottom: 1px solid var(--border);
  }
</style>
