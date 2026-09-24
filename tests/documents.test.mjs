import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { DOCUMENT_TEMPLATES, CUSTOM_TEMPLATE, findTemplate, applyTemplateStructure, isBlankSection } from "../src/lib/templates.ts";
import { buildMarkdown, buildHtml, exportFileName } from "../src/lib/export.ts";
import { statusSummary, defaultRange, errorText } from "../src/lib/git.ts";

const section = (title, content = "", extra = {}) => ({ id: title, title, content, diagrams: [], ...extra });

test("templates keep written content, reuse matching sections and drop only blank ones", () => {
  const c4 = findTemplate("c4");
  const existing = [section("Overview", "Billing handles invoices."), section("Empty"), section("Guided", "<!-- only guidance -->"),
    section("system context", "Users and the bank."), section("Pictures", "", { diagrams: [{ id: "g", content: "@startuml\n@enduml", diagram_type: "class", format: "plantuml" }] })];
  const result = applyTemplateStructure(existing, c4);
  assert.deepEqual(result.map(s => s.title), ["system context", "Container Diagram", "Component Diagram", "Code / Class Diagram", "Overview", "Pictures"]);
  assert.equal(result[0], existing[3], "a matching section keeps its identity and content");
  assert.match(result[1].content, /^<!-- Deployable\/runnable units .* -->$/);
  assert.ok(isBlankSection(result[1]) && isBlankSection(existing[2]) && !isBlankSection(existing[4]));
  // Applying the same template again changes nothing.
  assert.deepEqual(applyTemplateStructure(result, c4), result);
  // Custom keeps whatever structure there is; on an empty document it starts with Overview.
  assert.equal(applyTemplateStructure(existing, CUSTOM_TEMPLATE), existing);
  assert.deepEqual(applyTemplateStructure([], CUSTOM_TEMPLATE).map(s => [s.title, s.content]), [["Overview", ""]]);
  assert.equal(findTemplate("missing"), undefined);
});

test("every template section title matches the Python engine, which generates the same sections", () => {
  const agent = readFileSync(new URL("../python-engine/agent.py", import.meta.url), "utf8");
  for (const template of DOCUMENT_TEMPLATES) {
    assert.ok(agent.includes(`"${template.id}": [`), `${template.id} is missing in TEMPLATE_SECTIONS`);
    for (const { title, guidance } of template.sections) {
      assert.ok(agent.includes(`"${title}"`), `${template.id}: ${title}`);
      assert.ok(guidance && !guidance.includes("-->"), `${template.id}: ${title} guidance`);
    }
  }
  assert.equal(new Set(DOCUMENT_TEMPLATES.map(t => t.id)).size, DOCUMENT_TEMPLATES.length);
});

const document = {
  id: "d", name: "Architektúra – Billing", template: "arc42", language: "sk", revision: 1,
  sections: [
    { id: "s1", title: "Context", content: "Text with <b>html</b>.", diagrams: [
      { id: "g1", content: "@startuml\nA -> B\n@enduml", diagram_type: "sequence", format: "plantuml" },
      { id: "g2", content: "graph TD\n```\nA-->B", diagram_type: "flow", format: "mermaid" },
    ] },
    { id: "s2", title: "Empty <section>", content: "", diagrams: [] },
  ],
};

test("Markdown export carries front matter, the document name and fenced diagrams", () => {
  const markdown = buildMarkdown(document, { projectName: 'Billing "2"', date: "2026-09-24" });
  assert.equal(markdown, `---
title: "Architektúra – Billing"
project: "Billing \\"2\\""
template: "arc42"
language: "sk"
date: 2026-09-24
---

# Architektúra – Billing

## Context

Text with <b>html</b>.

**Diagram:** sequence

\`\`\`plantuml
@startuml
A -> B
@enduml
\`\`\`

**Diagram:** flow

~~~~mermaid
graph TD
\`\`\`
A-->B
~~~~

## Empty <section>
`);
  assert.equal(exportFileName(document.name, "md"), "architektura-billing.md");
  assert.equal(exportFileName("???", "html"), "document.html");
});

test("HTML export escapes names, embeds only SVG data images and keeps other diagrams as source", () => {
  const html = buildHtml(document, { projectName: "<Billing>", date: "2026-09-24" }, md => `<p>${md.length}</p>`,
    { g1: "data:image/svg+xml;charset=utf-8,%3Csvg%3E", g2: "javascript:alert(1)" });
  assert.match(html, /<title>Architektúra – Billing<\/title>/);
  assert.match(html, /<html lang="sk">/);
  assert.match(html, /&lt;Billing&gt; · arc42 · 2026-09-24/);
  assert.match(html, /<li><a href="#section-2">Empty &lt;section&gt;<\/a><\/li>/);
  assert.match(html, /<figure><img src="data:image\/svg\+xml;charset=utf-8,%3Csvg%3E" alt="sequence diagram">/);
  assert.doesNotMatch(html, /javascript:/);
  assert.match(html, /flow diagram \(mermaid source\)<\/div>\n<pre><code>graph TD\n```\nA--&gt;B<\/code><\/pre>/);
  assert.match(html, /<h2 id="section-1">Context<\/h2>\n<p>22<\/p>/);
});

test("Git status summary and the default release range", () => {
  const status = { root: "/r", branch: "main", head: "abc1234def", upstream: "origin/main", ahead: 2, behind: 1, changes: [{ path: "a.md", status: "modified" }], truncated: false };
  assert.equal(statusSummary(status), "main · ↑2 ↓1 · 1 change");
  assert.equal(statusSummary({ ...status, upstream: null, changes: [], branch: null }), "detached abc1234 · no upstream · clean");
  assert.equal(statusSummary({ ...status, ahead: 0, behind: 0, truncated: true, changes: Array(500).fill(status.changes[0]) }), "main · 500+ changes");

  const tag = (name, commit) => ({ name, kind: "tag", commit, date: "" });
  const refs = { root: "/r", branch: "main", head: "c3", refs: [tag("v1.1.0", "c2"), tag("v1.0.0", "c1"), { name: "main", kind: "branch", commit: "c3", date: "" }] };
  assert.deepEqual(defaultRange(refs), { from: "v1.1.0", to: "HEAD", version: "Unreleased" });
  assert.deepEqual(defaultRange({ ...refs, head: "c2" }), { from: "v1.0.0", to: "v1.1.0", version: "1.1.0" });
  assert.deepEqual(defaultRange({ ...refs, head: "c2", refs: [tag("release-7", "c2")] }), { from: "", to: "release-7", version: "release-7" });
  assert.deepEqual(defaultRange({ ...refs, refs: [] }), { from: "", to: "HEAD", version: "Unreleased" });
  assert.equal(errorText(new Error("boom")), "boom");
  assert.equal(errorText("plain"), "plain");
});
