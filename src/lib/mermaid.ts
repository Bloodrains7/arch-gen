// Local, offline Mermaid rendering. `mermaid` is loaded with a dynamic import() so it
// never enters the main bundle and is only fetched (from the app's own assets, never
// the network) the first time a Mermaid diagram is actually shown.
//
// Output contract: renderMermaidSvg() never touches application DOM and returns plain
// SVG text; the caller must still pass it through localSvgUrl() (local-svg.ts) before
// display, exactly like the PlantUML renderer's output. local-svg.ts forbids
// <foreignObject>, so every diagram type is configured here for `htmlLabels: false` —
// labels as SVG <text>, not HTML — or its label would simply be stripped by that
// sanitizer instead of rendered.
import type { MermaidConfig } from "mermaid";

// Mirrors the 256 KiB limit the Rust local renderer applies to PlantUML (src-tauri/src/renderer.rs).
export const MAX_MERMAID_SOURCE_BYTES = 256 * 1024;

function byteLength(source: string): number {
  return new TextEncoder().encode(source).length;
}

// Mermaid's own parse/render errors are long, implementation-specific and often span
// several lines (grammar dumps, caret pointers). Keep only the first non-blank line so
// the preview shows something a diagram author can act on.
export function mermaidErrorMessage(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const firstLine = raw.split("\n").map((line) => line.trim()).find((line) => line.length > 0) ?? "";
  const short = firstLine.length > 200 ? `${firstLine.slice(0, 200)}…` : firstLine;
  return short ? `Invalid Mermaid diagram: ${short}` : "Invalid Mermaid diagram.";
}

// Mermaid mounts its SVG, <style> block and inline styles in the live page while it
// measures text, so any CSS that reaches it can fetch (url(), @import, image-set()) —
// and the source comes from project files, possibly a cloned repository. Directives and
// front-matter config can set themeCSS, fonts or per-diagram CSS values at any depth, so
// they are refused outright; style statements may only use plain values (names, #hex,
// numbers, rgb()/hsl()), never url(), escapes or at-rules. Returns why, or null.
const STYLE_STATEMENT = /^\s*(?:style|classDef|linkStyle|cssClass)\b(.*)$/i;
const STYLE_CALL = /^\s*(?:Update\w*Style|UpdateLayoutConfig)\s*\((.*)\)\s*$/i;
export function mermaidSourceProblem(source: string): string | null {
  const text = source.replace(/\r\n?/g, "\n");
  if (text.includes("%%{")) return "Mermaid init directives (%%{…}%%) are not supported in the local preview: they can load remote resources. Remove the directive.";
  const front = /^\s*---\n([\s\S]*?)\n---/.exec(text);
  if (front && /^\s*config\s*:/m.test(front[1])) return "Mermaid front-matter config is not supported in the local preview: it can load remote resources. Keep only title: in the front matter.";
  for (const line of text.split("\n")) {
    const values = (STYLE_STATEMENT.exec(line) ?? STYLE_CALL.exec(line))?.[1];
    // Any "(" left after the colour functions could open url(), image-set() and the like;
    // a backslash could spell one with a CSS escape; "@" starts an at-rule.
    if (values !== undefined && /[\\@(]/.test(values.replace(/\b(?:rgba?|hsla?)\(/gi, ""))) {
      return `Mermaid style statements may only use plain values (names, #hex, numbers, rgb()/hsl()) in the local preview: ${line.trim().slice(0, 80)}`;
    }
  }
  return null;
}

const MERMAID_CONFIG: MermaidConfig = {
  startOnLoad: false,
  // Mermaid's own sanitizing pass (drops script/click bindings); local-svg.ts strips
  // <script>/<a>/href/on* independently either way, so this is defence in depth.
  securityLevel: "strict",
  // Root-level htmlLabels takes precedence over every diagram-specific setting (the
  // per-diagram ones below are deprecated but kept for older Mermaid semantics).
  htmlLabels: false,
  flowchart: { htmlLabels: false },
  class: { htmlLabels: false },
  // Governs Mermaid's own "diagram too big" placeholder; our explicit byte check below
  // is what actually decides whether a diagram is rendered at all, so keep this above
  // that check's threshold rather than let Mermaid silently swap in a placeholder text.
  maxTextSize: MAX_MERMAID_SOURCE_BYTES,
  // Defence in depth behind mermaidSourceProblem(): directives may never change these.
  secure: ["secure", "securityLevel", "startOnLoad", "maxTextSize", "suppressErrorRendering", "maxEdges",
    "themeCSS", "themeVariables", "fontFamily", "altFontFamily", "htmlLabels"],
};

type MermaidModule = typeof import("mermaid");
let modulePromise: Promise<MermaidModule> | null = null;

// Imported and initialized exactly once, on first use.
function loadMermaid(): Promise<MermaidModule> {
  if (!modulePromise) {
    modulePromise = import("mermaid").then((mod) => {
      mod.default.initialize(MERMAID_CONFIG);
      return mod;
    });
  }
  return modulePromise;
}

// Mermaid colors/strokes/fonts almost entirely through a <style> block it inserts at
// the top of the SVG (CSS class selectors), not through presentation attributes.
// local-svg.ts forbids both <style> and style="" outright (a CSS rule's text is never
// scanned for url(), unlike an attribute value, so allowing it back in would reopen the
// exact hole that rule closes) — so without this step every shape would fall back to
// the SVG default fill (opaque black), text and all, making the whole diagram unreadable.
// Bake the same values in as plain presentation attributes before that sanitizer runs.
const INLINE_PROPS = [
  "fill", "stroke", "stroke-width", "stroke-dasharray", "stroke-linecap", "stroke-linejoin",
  "opacity", "fill-opacity", "stroke-opacity",
  "font-family", "font-size", "font-weight", "font-style",
  "text-anchor", "dominant-baseline", "alignment-baseline",
];

function inlineComputedStyles(svgMarkup: string): string {
  const host = document.createElement("div");
  // Laid out like a normal element, so computed values match what would have been
  // painted, but never visible and never affects page layout or scroll.
  host.style.cssText = "position:fixed;top:0;left:0;visibility:hidden;pointer-events:none;";
  host.innerHTML = svgMarkup;
  document.body.appendChild(host);
  try {
    const svg = host.querySelector("svg");
    if (!svg) return svgMarkup;
    for (const el of svg.querySelectorAll("*")) {
      const computed = getComputedStyle(el);
      for (const prop of INLINE_PROPS) {
        const value = computed.getPropertyValue(prop);
        if (value) el.setAttribute(prop, value);
      }
    }
    for (const style of svg.querySelectorAll("style")) style.remove();
    return svg.outerHTML;
  } finally {
    host.remove();
  }
}

let renderCounter = 0;
// Mermaid keeps render state at module scope and documents its render() as needing to
// run serially; we do not rely on that alone; every render here goes through this queue.
let queue: Promise<void> = Promise.resolve();

async function renderOnce(source: string): Promise<string> {
  let mermaid: MermaidModule["default"];
  try {
    mermaid = (await loadMermaid()).default;
  } catch (err) {
    // Not a diagram-syntax problem, so it gets its own message rather than
    // mermaidErrorMessage()'s "Invalid Mermaid diagram: " framing.
    throw new Error(`Could not load the Mermaid renderer: ${err instanceof Error ? err.message : String(err)}`);
  }
  const id = `mermaid-diagram-${++renderCounter}`;
  try {
    const { svg } = await mermaid.render(id, source);
    return inlineComputedStyles(svg);
  } catch (err) {
    throw new Error(mermaidErrorMessage(err));
  } finally {
    // mermaid.render() inserts a temporary `#d<id>` element into <body> while it works
    // and removes it itself once serialization succeeds — but not on a thrown parse
    // error, so we always clean up anything it might have left behind.
    for (const domId of [id, `d${id}`, `i${id}`]) document.getElementById(domId)?.remove();
  }
}

/**
 * Renders Mermaid source to SVG text, entirely offline (no fonts, no icon packs, no
 * network of any kind). Pass the result through localSvgUrl() before displaying it.
 */
export function renderMermaidSvg(source: string): Promise<string> {
  if (byteLength(source) > MAX_MERMAID_SOURCE_BYTES) {
    return Promise.reject(new Error(`Diagram exceeds the ${MAX_MERMAID_SOURCE_BYTES / 1024} KiB render limit.`));
  }
  const problem = mermaidSourceProblem(source);
  if (problem) return Promise.reject(new Error(problem));
  const result = queue.then(() => renderOnce(source), () => renderOnce(source));
  queue = result.then(() => undefined, () => undefined);
  return result;
}
