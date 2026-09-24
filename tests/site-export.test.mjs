import { test } from "node:test";
import assert from "node:assert/strict";
import { buildSite, renderDiagrams } from "../src/lib/site-export.ts";

const document = (name, sections, extra = {}) => ({ id: name, name, template: "arc42", language: "sk", revision: 0, sections, ...extra });
const section = (title, content, diagrams = []) => ({ id: title, title, content, diagrams });
const diagram = (id, format, diagram_type, content = "content") => ({ id, format, diagram_type, content });

const project = (documents, extra = {}) => ({ schemaVersion: 1, id: "p", name: "Project", revision: 0, activeDocumentId: documents[0].id, documents, ...extra });

test("buildSite lists an index page, one page per document, images for rendered diagrams and toc/mkdocs files", () => {
  const proj = project([
    document("Billing", [
      section("Context", "Users and the bank.", [
        diagram("g1", "plantuml", "sequence", "@startuml\nA -> B\n@enduml"),
        diagram("g2", "mermaid", "flow", "graph TD\nA-->B"),
      ]),
    ]),
  ]);
  const { files, stats } = buildSite(proj, { projectName: "Project", date: "2026-09-24" }, { g1: "<svg>seq</svg>" });

  assert.deepEqual(stats, { documents: 1, diagramsRendered: 1, diagramsAsSource: 1 });
  const byPath = Object.fromEntries(files.map(f => [f.path, f.content]));
  assert.ok(byPath["index.md"]);
  assert.ok(byPath["billing.md"]);
  assert.ok(byPath["toc.yml"]);
  assert.ok(byPath["mkdocs.yml"]);
  assert.equal(byPath["images/billing/context-1.svg"], "<svg>seq</svg>");
  assert.equal(Object.keys(byPath).length, 5);

  const page = byPath["billing.md"];
  assert.match(page, /^---\ntitle: "Billing"\nproject: "Project"\ntemplate: "arc42"\nlanguage: "sk"\ndate: 2026-09-24\n---\n\n# Billing/);
  // The rendered diagram becomes a relative image link; the source diagram stays fenced, unchanged from buildMarkdown.
  assert.match(page, /\*\*Diagram:\*\* sequence\n\n!\[sequence diagram\]\(images\/billing\/context-1\.svg\)/);
  assert.match(page, /\*\*Diagram:\*\* flow\n\n```mermaid\ngraph TD\nA-->B\n```/);
  assert.doesNotMatch(page, /@startuml/, "a rendered diagram's source is not also embedded");
});

test("toc.yml lists index first, in DocFX name/href form; mkdocs.yml is a minimal MkDocs nav", () => {
  const proj = project([document("Billing", []), document("Ops", [])], { name: "Acme Docs" });
  const { files } = buildSite(proj, { projectName: "Acme Docs", date: "2026-09-24" }, {});
  const byPath = Object.fromEntries(files.map(f => [f.path, f.content]));

  assert.equal(byPath["toc.yml"], `- name: "Overview"
  href: index.md
- name: "Billing"
  href: billing.md
- name: "Ops"
  href: ops.md
`);
  assert.equal(byPath["mkdocs.yml"], `site_name: "Acme Docs"
nav:
  - "Overview": index.md
  - "Billing": billing.md
  - "Ops": ops.md
`);
});

test("index.md carries the project name, generated date and a document table with template/language/section counts", () => {
  const proj = project([
    document("Billing", [section("A", ""), section("B", "")]),
    document("Ops", [section("Only", "")], { template: "c4", language: "en" }),
  ], { name: "Acme Docs" });
  const { files } = buildSite(proj, { projectName: "Acme Docs", date: "2026-09-24" }, {});
  const index = Object.fromEntries(files.map(f => [f.path, f.content]))["index.md"];
  assert.match(index, /^# Acme Docs\n\n_Generated: 2026-09-24_\n\n\| Document \| Template \| Language \| Sections \|/);
  assert.match(index, /\| \[Billing\]\(billing\.md\) \| arc42 \| sk \| 2 \|/);
  assert.match(index, /\| \[Ops\]\(ops\.md\) \| c4 \| en \| 1 \|/);
});

test("YAML names are quoted safely (colon, hash, quotes, diacritics) and the index table escapes | [ ]", () => {
  const proj = project([document('Ops | Billing: "Core" #v1 – Áý', [])], { name: 'Docs: "Team" #1' });
  const { files } = buildSite(proj, { projectName: proj.name, date: "2026-09-24" }, {});
  const byPath = Object.fromEntries(files.map(f => [f.path, f.content]));
  assert.match(byPath["toc.yml"], /name: "Ops \| Billing: \\"Core\\" #v1 – Áý"/);
  assert.match(byPath["mkdocs.yml"], /site_name: "Docs: \\"Team\\" #1"/);
  assert.match(byPath["mkdocs.yml"], /- "Ops \| Billing: \\"Core\\" #v1 – Áý": /);
  // JSON.stringify never emits an unescaped double quote or a bare newline inside the scalar.
  assert.doesNotMatch(byPath["toc.yml"], /[^\\]"Core"/);
  assert.match(byPath["index.md"], /\[Ops \\\| Billing: "Core" #v1 – Áý\]\(/);
});

test("document and section slugs collide safely (case-insensitively) and a document is never named index", () => {
  const proj = project([
    document("Billing", [section("Context", ""), section("context", "")]),
    document("billing", []),
    document("Index", []),
  ]);
  const { files } = buildSite(proj, { projectName: "Project", date: "2026-09-24" }, {});
  const paths = files.map(f => f.path).sort();
  assert.deepEqual(paths, ["billing-2.md", "billing.md", "index-2.md", "index.md", "mkdocs.yml", "toc.yml"]);
});

test("a diagram without an id, or one the renderer skipped, is never referenced as an image", () => {
  const proj = project([document("Billing", [section("Context", "", [
    diagram(undefined, "plantuml", "class"),
    diagram("g1", "plantuml", "class"),
  ])])]);
  // `rendered` has no entry for either diagram: the id-less one can never get one, and the
  // named one simply was not rendered.
  const { files, stats } = buildSite(proj, { projectName: "Project", date: "2026-09-24" }, {});
  assert.deepEqual(stats, { documents: 1, diagramsRendered: 0, diagramsAsSource: 2 });
  assert.ok(!files.some(f => f.path.startsWith("images/")));
});

test("renderDiagrams collects only what the renderer resolves, keyed by diagram id", async () => {
  const proj = project([document("Billing", [section("Context", "", [
    diagram("g1", "plantuml", "class"),
    diagram("g2", "mermaid", "class"),
    diagram(undefined, "plantuml", "class"),
  ])])]);
  const seen = [];
  const rendered = await renderDiagrams(proj, async d => {
    seen.push(d.id);
    return d.format === "plantuml" ? `<svg>${d.id}</svg>` : null;
  });
  assert.deepEqual(seen, ["g1", "g2"]); // the id-less diagram is never handed to the renderer
  assert.deepEqual(rendered, { g1: "<svg>g1</svg>" });
});
