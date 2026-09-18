import { test } from "node:test";
import assert from "node:assert/strict";
import {
  emptySelection, isSectionSelected, isDiagramSelected, toggleSection, toggleDiagram, pruneSelection, selectionSummary,
} from "../src/lib/selection.ts";

test("emptySelection starts with nothing ticked", () => {
  assert.deepEqual(emptySelection(), { sectionIds: [], diagramIds: [] });
});

test("toggleSection adds then removes a section id", () => {
  const selected = toggleSection(emptySelection(), "s1");
  assert.deepEqual(selected, { sectionIds: ["s1"], diagramIds: [] });
  assert.ok(isSectionSelected(selected, "s1"));
  assert.deepEqual(toggleSection(selected, "s1"), emptySelection());
});

test("toggleDiagram adds then removes a diagram id when its section is not selected", () => {
  const selected = toggleDiagram(emptySelection(), "s1", "d1");
  assert.deepEqual(selected, { sectionIds: [], diagramIds: ["d1"] });
  assert.ok(isDiagramSelected(selected, "d1"));
  assert.deepEqual(toggleDiagram(selected, "s1", "d1"), emptySelection());
});

test("toggleDiagram is a no-op while its section is selected", () => {
  const withSection = toggleSection(emptySelection(), "s1");
  assert.equal(toggleDiagram(withSection, "s1", "d1"), withSection);
  assert.equal(isDiagramSelected(withSection, "d1"), false);
});

test("pruneSelection returns the same object by reference when nothing was dropped", () => {
  const sections = [{ id: "s1", title: "A", content: "", diagrams: [{ id: "d1", content: "", diagram_type: "class", format: "plantuml" }] }];
  const selection = { sectionIds: ["s1"], diagramIds: [] };
  assert.equal(pruneSelection(selection, sections), selection);
});

test("pruneSelection drops section and diagram ids that no longer exist", () => {
  const sections = [{ id: "s1", title: "A", content: "", diagrams: [] }];
  const selection = { sectionIds: ["s1", "gone"], diagramIds: ["also-gone"] };
  assert.deepEqual(pruneSelection(selection, sections), { sectionIds: ["s1"], diagramIds: [] });
});

test("pruneSelection drops a diagram id once its own section is selected", () => {
  const sections = [{ id: "s1", title: "A", content: "", diagrams: [{ id: "d1", content: "", diagram_type: "class", format: "plantuml" }] }];
  const selection = { sectionIds: ["s1"], diagramIds: ["d1"] };
  assert.deepEqual(pruneSelection(selection, sections), { sectionIds: ["s1"], diagramIds: [] });
});

test("pruneSelection keeps a diagram id selected while its section is not", () => {
  const sections = [{ id: "s1", title: "A", content: "", diagrams: [{ id: "d1", content: "", diagram_type: "class", format: "plantuml" }] }];
  const selection = { sectionIds: [], diagramIds: ["d1"] };
  assert.equal(pruneSelection(selection, sections), selection);
});

test("selectionSummary uses singular and plural wording", () => {
  assert.equal(selectionSummary(emptySelection()), "");
  assert.equal(selectionSummary({ sectionIds: ["s1"], diagramIds: [] }), "1 section");
  assert.equal(selectionSummary({ sectionIds: ["s1", "s2"], diagramIds: [] }), "2 sections");
  assert.equal(selectionSummary({ sectionIds: [], diagramIds: ["d1"] }), "1 diagram");
  assert.equal(selectionSummary({ sectionIds: ["s1", "s2"], diagramIds: ["d1"] }), "2 sections, 1 diagram");
  assert.equal(selectionSummary({ sectionIds: [], diagramIds: ["d1", "d2"] }), "2 diagrams");
});

test("isSectionSelected and isDiagramSelected read membership", () => {
  const selection = { sectionIds: ["s1"], diagramIds: ["d1"] };
  assert.equal(isSectionSelected(selection, "s1"), true);
  assert.equal(isSectionSelected(selection, "s2"), false);
  assert.equal(isDiagramSelected(selection, "d1"), true);
  assert.equal(isDiagramSelected(selection, "d2"), false);
});
