// Whole-project export as a docs-as-code folder: one Markdown page per document plus an
// index, ready to publish with DocFX / Microsoft Learn, MkDocs, an Azure DevOps wiki or
// GitHub (Markdown and fenced ```mermaid blocks render natively there). Pure and
// synchronous — the caller renders diagrams first (see `DiagramRenderer` below) and hands
// the results in, so this module never touches the network, the filesystem or Tauri.
import type { Diagram, Project, ProjectDocument } from "./project";
import { documentBody, frontMatter, slugify, yamlScalar, type ExportMeta } from "./export.ts";

export interface SiteFile {
  path: string;
  content: string;
}

export interface SiteStats {
  documents: number;
  diagramsRendered: number;
  diagramsAsSource: number;
}

export interface SiteResult {
  files: SiteFile[];
  stats: SiteStats;
}

/** Renders one diagram to sanitized SVG text, or `null` to leave it as fenced source in
 * the page. The single place to plug in another local renderer (e.g. Mermaid) later —
 * `renderDiagrams` below just calls whatever is passed in, once per diagram. */
export type DiagramRenderer = (diagram: Diagram) => Promise<string | null>;

/** Runs `renderer` over every diagram in the project and collects a `rendered` map for
 * `buildSite`, keyed by diagram id. Diagrams without an id, or the renderer returned
 * `null` for, are left out and `buildSite` keeps them as source. */
export async function renderDiagrams(project: Project, renderer: DiagramRenderer): Promise<Record<string, string>> {
  const rendered: Record<string, string> = {};
  for (const document of project.documents) {
    for (const section of document.sections) {
      for (const diagram of section.diagrams) {
        if (!diagram.id) continue;
        const svg = await renderer(diagram);
        if (svg) rendered[diagram.id] = svg;
      }
    }
  }
  return rendered;
}

// Windows cannot have a file or folder named exactly one of these, with or without an
// extension (CON.md is as unusable as CON) — mirrors project.rs's own `slug`.
const RESERVED_NAMES = /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])$/;

/** `exportFileName`'s slug, but also steps around a Windows-reserved device name —
 * this is a page/folder name (`foo.md`, `images/foo/`), where `exportFileName`'s own
 * callers only ever produce a `.md`/`.html` file next to files the OS already accepted. */
function siteSlug(text: string, fallback: string): string {
  const base = slugify(text, fallback);
  return RESERVED_NAMES.test(base) ? `${base}-${fallback}` : base;
}

/** `base`, then `base-2`, `base-3`, … until `used` (compared case-insensitively, for
 * Windows) does not already hold it. Reserving names up front in `used` (e.g. "index")
 * keeps them from ever being handed out. */
function uniqueSlug(base: string, used: Set<string>): string {
  let candidate = base;
  for (let n = 2; used.has(candidate.toLowerCase()); n++) candidate = `${base}-${n}`;
  used.add(candidate.toLowerCase());
  return candidate;
}

// Also escapes [ and ] since this text sits inside a Markdown table's link column.
function tableCell(text: string): string {
  return text.replace(/\|/g, "\\|").replace(/[[\]]/g, "\\$&").replace(/\r?\n/g, " ").trim();
}

function buildIndex(project: Project, meta: ExportMeta, pages: { document: ProjectDocument; slug: string }[]): string {
  const rows = pages.map(({ document, slug }) =>
    `| [${tableCell(document.name)}](${slug}.md) | ${tableCell(document.template)} | ${tableCell(document.language)} | ${document.sections.length} |`);
  return [
    `# ${project.name}`,
    `_Generated: ${meta.date}_`,
    ["| Document | Template | Language | Sections |", "| --- | --- | --- | --- |", ...rows].join("\n"),
  ].join("\n\n") + "\n";
}

function buildToc(pages: { document: ProjectDocument; slug: string }[]): string {
  const entry = (name: string, href: string) => `- name: ${yamlScalar(name)}\n  href: ${href}`;
  return [entry("Overview", "index.md"), ...pages.map(({ document, slug }) => entry(document.name, `${slug}.md`))].join("\n") + "\n";
}

function buildMkdocs(project: Project, pages: { document: ProjectDocument; slug: string }[]): string {
  const nav = (name: string, href: string) => `  - ${yamlScalar(name)}: ${href}`;
  return [
    `site_name: ${yamlScalar(project.name)}`,
    "nav:",
    nav("Overview", "index.md"),
    ...pages.map(({ document, slug }) => nav(document.name, `${slug}.md`)),
  ].join("\n") + "\n";
}

/**
 * Builds the whole exported site as an in-memory file list. `rendered` maps a diagram id
 * to already-sanitized SVG text (see `renderDiagrams`); a diagram missing from it is kept
 * as fenced source, exactly like the single-document export.
 */
export function buildSite(project: Project, meta: ExportMeta, rendered: Record<string, string>): SiteResult {
  // "index" is reserved for index.md; a document can never be handed that slug.
  const usedDocSlugs = new Set<string>(["index"]);
  const pages = project.documents.map(document => ({ document, slug: uniqueSlug(siteSlug(document.name, "document"), usedDocSlugs) }));

  const files: SiteFile[] = [];
  let diagramsRendered = 0;
  let diagramsAsSource = 0;

  for (const { document, slug: docSlug } of pages) {
    const usedSectionSlugs = new Set<string>();
    const imagePaths = new Map<string, string>(); // diagram.id -> path relative to the page
    for (const section of document.sections) {
      const sectionSlug = uniqueSlug(siteSlug(section.title, "section"), usedSectionSlugs);
      section.diagrams.forEach((diagram, index) => {
        const svg = diagram.id ? rendered[diagram.id] : undefined;
        if (svg === undefined) { diagramsAsSource++; return; }
        diagramsRendered++;
        const path = `images/${docSlug}/${sectionSlug}-${index + 1}.svg`;
        if (diagram.id) imagePaths.set(diagram.id, path);
        files.push({ path, content: svg });
      });
    }
    const body = documentBody(document, diagram => diagram.id ? imagePaths.get(diagram.id) : undefined);
    files.push({ path: `${docSlug}.md`, content: [frontMatter(document, meta), ...body].join("\n\n") + "\n" });
  }

  files.push({ path: "index.md", content: buildIndex(project, meta, pages) });
  files.push({ path: "toc.yml", content: buildToc(pages) });
  files.push({ path: "mkdocs.yml", content: buildMkdocs(project, pages) });

  return { files, stats: { documents: project.documents.length, diagramsRendered, diagramsAsSource } };
}
