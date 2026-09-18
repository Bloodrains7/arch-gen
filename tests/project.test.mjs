import { test } from "node:test";
import assert from "node:assert/strict";
import { createProject, createDocument, updateDocument, captureTarget, applyGeneratedSections, parseProject, emptyHistory, recordChange, moveHistory, restoreSnapshot, compareSections } from "../src/lib/project.ts";

const sections = () => [{ title: "Data", content: "Žiadne stratené zmeny", diagrams: [{ content: "@startuml\n@enduml", diagram_type: "class", format: "plantuml" }] }];

test("saved project round trips documents and stable section/diagram IDs", () => {
  let project = createProject();
  project = updateDocument(project, project.activeDocumentId, { sections: sections() });
  const restored = parseProject(JSON.parse(JSON.stringify(project)));
  assert.deepEqual(restored, project);
  assert.ok(restored.documents[0].sections[0].diagrams[0].id);
  assert.equal(updateDocument(restored, restored.activeDocumentId, { sections: restored.documents[0].sections }), restored);
});

test("async result goes to original document after switching tabs", () => {
  const project = createProject();
  const target = captureTarget(project);
  const second = createDocument("Second");
  const switched = { ...project, documents: [...project.documents, second], activeDocumentId: second.id };
  const result = applyGeneratedSections(switched, target, sections());
  assert.equal(result.activeDocumentId, second.id);
  assert.equal(result.documents[0].sections.length, 1);
  assert.equal(result.documents[1].sections.length, 0);
});

test("edits, template changes, deletion and different project reject stale results", () => {
  const project = createProject();
  const target = captureTarget(project);
  for (const changed of [
    updateDocument(project, target.documentId, { sections: sections() }),
    updateDocument(project, target.documentId, { template: "c4" }),
    { ...project, documents: [] }, createProject(),
  ]) assert.equal(applyGeneratedSections(changed, target, sections()), null);
});

test("unrelated document edits do not invalidate generation", () => {
  const project = createProject();
  const target = captureTarget(project);
  const second = createDocument();
  const changed = updateDocument({ ...project, documents: [...project.documents, second] }, second.id, { sections: sections() });
  assert.ok(applyGeneratedSections(changed, target, sections()));
});

test("project parser rejects future versions, duplicate IDs and broken active references", () => {
  const project = createProject();
  for (const invalid of [null, {}, { ...project, schemaVersion: 2 }, { ...project, activeDocumentId: "missing" },
    { ...project, documents: [project.documents[0], project.documents[0]] }, { ...project, revision: -1 }]) {
    assert.throws(() => parseProject(invalid));
  }
});

test("undo/redo preserves content and IDs with monotonically increasing revisions", () => {
  const original = createProject();
  const changed = updateDocument(original, original.activeDocumentId, { sections: sections() });
  const history = recordChange(emptyHistory(), original, changed, "Add section");
  const undo = moveHistory(changed, history, "undo");
  assert.equal(undo.project.documents[0].sections.length, 0);
  assert.ok(undo.project.revision > changed.revision);
  const redo = moveHistory(undo.project, undo.history, "redo");
  assert.deepEqual(redo.project.documents[0].sections, changed.documents[0].sections);
  assert.ok(redo.project.revision > undo.project.revision);
  assert.equal(applyGeneratedSections(redo.project, captureTarget(changed), sections()), null, "undo and redo must not revive an old AI target");
});

test("typing coalesces, structural edits do not, and edits after undo invalidate redo", () => {
  const original = createProject();
  const first = updateDocument(original, original.activeDocumentId, { name: "A" });
  const second = updateDocument(first, first.activeDocumentId, { name: "AB" });
  let history = recordChange(emptyHistory(), original, first, "Typing", "name", 1000);
  history = recordChange(history, first, second, "Typing", "name", 1100);
  assert.equal(history.past.length, 1);
  const third = updateDocument(second, second.activeDocumentId, { sections: sections() });
  history = recordChange(history, second, third, "Add section", undefined, 1200);
  assert.equal(history.past.length, 2);
  const undo = moveHistory(third, history, "undo");
  const edited = updateDocument(undo.project, undo.project.activeDocumentId, { language: "sk" });
  const branched = recordChange(undo.history, undo.project, edited, "Change language");
  assert.equal(branched.future.length, 0);
});

test("snapshot restore cannot cross projects and can undo document deletion", () => {
  const original = createProject();
  const second = createDocument();
  const full = { ...original, documents: [...original.documents, second], revision: 3 };
  const deleted = { ...full, documents: [second], activeDocumentId: second.id, revision: 4 };
  const restored = restoreSnapshot(deleted, full);
  assert.equal(restored.documents.length, 2);
  assert.equal(restored.documents[0].id, original.documents[0].id);
  assert.ok(restored.documents.every(d => d.revision === restored.revision));
  assert.throws(() => restoreSnapshot(createProject(), full));
});

test("preview identifies additions, deletions and diagram-only changes", () => {
  const base = [{ id: "s", title: "Data", content: "Text", diagrams: [{ id: "d", content: "Before", diagram_type: "class", format: "plantuml" }] },
    { id: "removed", title: "Old", content: "", diagrams: [] }];
  const after = [{ ...base[0], diagrams: [{ ...base[0].diagrams[0], content: "After" }] },
    { id: "added", title: "New", content: "", diagrams: [] }];
  assert.deepEqual(compareSections(base, after).map(c => c.kind), ["changed", "removed", "added"]);
  assert.equal(compareSections(base, base)[0].kind, "unchanged");
  assert.equal(compareSections([base[0]], [{ ...base[0], id: "generated" }])[0].kind, "unchanged", "unique titles may align regenerated section previews");
});

test("undo retains at most 50 independent edits", () => {
  let project = createProject();
  let history = emptyHistory();
  for (let i = 0; i < 60; i++) {
    const next = updateDocument(project, project.activeDocumentId, { name: `Document ${i}` });
    history = recordChange(history, project, next, "Rename document");
    project = next;
  }
  assert.equal(history.past.length, 50);
});
