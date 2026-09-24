// Markdown and HTML exports of one document. Pure functions: the caller supplies
// the Markdown renderer (marked + DOMPurify in the app) and any diagram images.
import type { ProjectDocument } from "./project";

export interface ExportMeta {
  projectName: string;
  /** YYYY-MM-DD */
  date: string;
}

/** A file name that works on every filesystem: "Architektúra – Billing" → "architektura-billing". */
export function exportFileName(name: string, extension: string): string {
  const stem = name.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase()
    .replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 60).replace(/-+$/, "");
  return `${stem || "document"}.${extension}`;
}

// A JSON string is a valid double-quoted YAML scalar.
const yaml = (value: string) => JSON.stringify(value);

/**
 * Markdown with YAML front matter (title, project, template, language, date), as
 * read by static site generators, DocFX / Microsoft Learn-style sites and wikis.
 */
export function buildMarkdown(document: ProjectDocument, meta: ExportMeta): string {
  const parts = [
    `---\ntitle: ${yaml(document.name)}\nproject: ${yaml(meta.projectName)}\ntemplate: ${yaml(document.template)}\nlanguage: ${yaml(document.language)}\ndate: ${meta.date}\n---`,
    `# ${document.name}`,
  ];
  for (const section of document.sections) {
    parts.push(`## ${section.title}`);
    if (section.content.trim()) parts.push(section.content.trim());
    for (const diagram of section.diagrams) {
      const fence = diagram.content.includes("```") ? "~~~~" : "```";
      parts.push(`**Diagram:** ${diagram.diagram_type}\n\n${fence}${diagram.format}\n${diagram.content}\n${fence}`);
    }
  }
  return parts.join("\n\n") + "\n";
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

const STYLE = `body { font-family: 'Segoe UI', Calibri, Arial, sans-serif; max-width: 900px; margin: 40px auto; padding: 0 20px; color: #1a1a1a; line-height: 1.6; }
h1 { color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; }
h2 { color: #2c3e50; border-bottom: 1px solid #bdc3c7; padding-bottom: 6px; margin-top: 30px; }
h3 { color: #34495e; }
table { border-collapse: collapse; width: 100%; margin: 12px 0; }
th, td { border: 1px solid #bdc3c7; padding: 8px 12px; text-align: left; }
th { background: #ecf0f1; font-weight: 600; }
pre { background: #f8f9fa; border: 1px solid #dee2e6; border-radius: 4px; padding: 12px; overflow-x: auto; font-size: 13px; }
code { background: #f8f9fa; padding: 2px 5px; border-radius: 3px; font-size: 13px; }
blockquote { border-left: 4px solid #3498db; margin: 12px 0; padding: 8px 16px; background: #f0f7ff; }
.meta { color: #6b7080; font-size: 13px; }
.toc { background: #f8f9fa; border: 1px solid #dee2e6; border-radius: 6px; padding: 12px 24px; }
figure { margin: 16px 0; text-align: center; }
figure img { max-width: 100%; }
figcaption, .diagram-label { font-size: 11px; text-transform: uppercase; letter-spacing: 1px; color: #7f8c8d; }
@media print { body { margin: 20px; } h2 { page-break-after: avoid; } .toc { page-break-after: always; } }`;

/**
 * A self-contained HTML page (opens in Word and prints to PDF). `images` maps a
 * diagram ID to a sanitized `data:image/svg+xml` URL; other diagrams stay source.
 */
export function buildHtml(document: ProjectDocument, meta: ExportMeta, renderMarkdown: (markdown: string) => string, images: Record<string, string> = {}): string {
  const toc = document.sections.map((section, i) => `<li><a href="#section-${i + 1}">${escapeHtml(section.title)}</a></li>`).join("\n");
  const body = document.sections.map((section, i) => {
    const diagrams = section.diagrams.map(diagram => {
      const image = diagram.id ? images[diagram.id] : undefined;
      const label = `${escapeHtml(diagram.diagram_type)} diagram`;
      return image && image.startsWith("data:image/svg+xml")
        ? `<figure><img src="${escapeHtml(image)}" alt="${label}"><figcaption>${label}</figcaption></figure>`
        : `<div class="diagram-label">${label} (${escapeHtml(diagram.format)} source)</div>\n<pre><code>${escapeHtml(diagram.content)}</code></pre>`;
    }).join("\n");
    return `<h2 id="section-${i + 1}">${escapeHtml(section.title)}</h2>\n${renderMarkdown(section.content)}\n${diagrams}`;
  }).join("\n");
  return `<!DOCTYPE html>
<html lang="${escapeHtml(document.language)}"><head>
<meta charset="utf-8">
<meta name="generator" content="ArchGen">
<title>${escapeHtml(document.name)}</title>
<style>
${STYLE}
</style>
</head><body>
<h1>${escapeHtml(document.name)}</h1>
<p class="meta">${escapeHtml(meta.projectName)} · ${escapeHtml(document.template)} · ${escapeHtml(meta.date)}</p>
${document.sections.length > 1 ? `<nav class="toc"><strong>Contents</strong><ol>\n${toc}\n</ol></nav>` : ""}
${body}
</body></html>
`;
}
