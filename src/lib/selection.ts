// Which sections and diagrams are ticked for AI rework. A section covers all of
// its own diagrams, so a diagram never stays individually selected once its
// section is: `toggleDiagram` refuses that state and `pruneSelection` repairs it
// if a section is selected after one of its diagrams already was.
import type { Section } from "./project";

export interface BlockSelection {
  sectionIds: string[];
  diagramIds: string[];
}

export function emptySelection(): BlockSelection {
  return { sectionIds: [], diagramIds: [] };
}

export function isSectionSelected(selection: BlockSelection, sectionId: string): boolean {
  return selection.sectionIds.includes(sectionId);
}

export function isDiagramSelected(selection: BlockSelection, diagramId: string): boolean {
  return selection.diagramIds.includes(diagramId);
}

export function toggleSection(selection: BlockSelection, sectionId: string): BlockSelection {
  return isSectionSelected(selection, sectionId)
    ? { ...selection, sectionIds: selection.sectionIds.filter(id => id !== sectionId) }
    : { ...selection, sectionIds: [...selection.sectionIds, sectionId] };
}

// A no-op while the diagram's own section is selected: the section already covers it.
export function toggleDiagram(selection: BlockSelection, sectionId: string, diagramId: string): BlockSelection {
  if (isSectionSelected(selection, sectionId)) return selection;
  return isDiagramSelected(selection, diagramId)
    ? { ...selection, diagramIds: selection.diagramIds.filter(id => id !== diagramId) }
    : { ...selection, diagramIds: [...selection.diagramIds, diagramId] };
}

// Drops IDs that no longer exist in `sections`, and any diagram whose section is
// itself selected. Returns the very same object when nothing needed dropping, so
// callers (a `$derived`) can tell "unchanged" from "changed" by reference.
export function pruneSelection(selection: BlockSelection, sections: Section[]): BlockSelection {
  const sectionIds = new Set(sections.map(s => s.id).filter((id): id is string => !!id));
  const diagramSection = new Map<string, string>();
  for (const section of sections) {
    if (!section.id) continue;
    for (const diagram of section.diagrams) {
      if (diagram.id) diagramSection.set(diagram.id, section.id);
    }
  }
  const prunedSectionIds = selection.sectionIds.filter(id => sectionIds.has(id));
  const selectedSections = new Set(prunedSectionIds);
  const prunedDiagramIds = selection.diagramIds.filter(id => {
    const sectionId = diagramSection.get(id);
    return sectionId !== undefined && !selectedSections.has(sectionId);
  });
  const unchanged = prunedSectionIds.length === selection.sectionIds.length && prunedDiagramIds.length === selection.diagramIds.length;
  return unchanged ? selection : { sectionIds: prunedSectionIds, diagramIds: prunedDiagramIds };
}

export function selectionSummary(selection: BlockSelection): string {
  const parts: string[] = [];
  const sections = selection.sectionIds.length;
  const diagrams = selection.diagramIds.length;
  if (sections) parts.push(`${sections} section${sections === 1 ? "" : "s"}`);
  if (diagrams) parts.push(`${diagrams} diagram${diagrams === 1 ? "" : "s"}`);
  return parts.join(", ");
}
