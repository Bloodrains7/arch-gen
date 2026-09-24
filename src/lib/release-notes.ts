// Release notes from Git commits and a Markdown template, without AI.
//
// A template is Markdown with a small front matter that sorts commits into groups,
// and Mustache-style tags ({{name}}, {{#list}}…{{/list}}, {{^empty}}…{{/empty}})
// for the body. The result is plain Markdown: it can become a document or a section,
// be reworked by AI like any other block, or be saved as CHANGELOG / RELEASE_NOTES.
import type { Section } from "./project";

export interface GitCommit {
  hash: string;
  shortHash: string;
  parents: number;
  author: string;
  date: string;
  subject: string;
  body: string;
}

export interface GitRange {
  root: string;
  from: string | null;
  to: string;
  commits: GitCommit[];
  truncated: boolean;
}

export interface IssueRef { id: string; url: string }

export interface ChangeItem {
  hash: string;
  shortHash: string;
  author: string;
  /** YYYY-MM-DD */
  date: string;
  /** Conventional Commits type in lowercase; "other" when the subject does not follow it, "merge" for merges. */
  type: string;
  scope: string;
  breaking: boolean;
  /** The BREAKING CHANGE footer, or the description when only `!` marks it. */
  breakingNote: string;
  description: string;
  subject: string;
  body: string;
  issues: IssueRef[];
  /** "#12, ABC-7": the issues not already written in the description, ready to print. */
  issueList: string;
  /** Empty unless the template sets `commitUrl`. */
  commitUrl: string;
}

export interface TemplateGroup { title: string; types: string[] }

export interface ReleaseTemplate {
  name: string;
  description: string;
  groups: TemplateGroup[];
  exclude: string[];
  issueUrl: string;
  commitUrl: string;
  body: string;
}

export interface ReleaseOptions {
  version: string;
  date: string;
  /** What the user typed for the range, for display ("v1.2.0"); empty means the beginning of history. */
  from: string;
  to: string;
  repository: string;
}

export interface ReleaseResult {
  markdown: string;
  included: ChangeItem[];
  hidden: ChangeItem[];
}

// ── Commits ──────────────────────────────────────────────────────────────────

const CONVENTIONAL = /^([a-zA-Z][\w-]*)(?:\(([^()\r\n]*)\))?(!)?:[ \t]+(.+)$/;
const BREAKING_FOOTER = /^BREAKING[ -]CHANGE:[ \t]*(.+(?:\n(?!\n)(?![\w-]+: ).+)*)/m;
const MERGE = /^Merge (pull request|branch|remote-tracking branch|tag)\b/;
const REVERT = /^Revert "(.+)"$/;
// Upper-case words followed by a number that are standards, not issue keys.
const NOT_ISSUES = new Set(["UTF", "ISO", "SHA", "AES", "RSA", "CVE", "RFC", "TLS", "SSL", "HTTP", "IEC", "EN", "ECMA", "WCAG", "ES", "IPV", "PDF", "GPT", "WINDOWS"]);

function issues(text: string, issueUrl: string): IssueRef[] {
  const found: string[] = [];
  for (const match of text.matchAll(/(?:^|[\s(,[])#(\d+)\b/g)) found.push(`#${match[1]}`);
  for (const match of text.matchAll(/\b([A-Z][A-Z0-9]{1,9})-(\d+)\b/g)) {
    if (!NOT_ISSUES.has(match[1])) found.push(`${match[1]}-${match[2]}`);
  }
  return [...new Set(found)].map(id => ({ id, url: issueUrl ? issueUrl.replaceAll("{id}", id.replace(/^#/, "")) : "" }));
}

export function parseCommit(commit: GitCommit, template: Pick<ReleaseTemplate, "issueUrl" | "commitUrl"> = { issueUrl: "", commitUrl: "" }): ChangeItem {
  const subject = commit.subject.trim();
  const body = commit.body.replace(/\r\n/g, "\n").trim();
  const merge = commit.parents > 1 || MERGE.test(subject);
  const conventional = merge ? null : CONVENTIONAL.exec(subject);
  const revert = !conventional && REVERT.exec(subject);
  const footer = BREAKING_FOOTER.exec(body);
  const description = conventional ? conventional[4].trim() : revert ? `Revert ${revert[1]}` : subject;
  const breaking = !merge && (!!conventional?.[3] || !!footer);
  const found = issues(`${subject}\n${body}`, template.issueUrl);
  return {
    hash: commit.hash, shortHash: commit.shortHash, author: commit.author, date: commit.date.slice(0, 10),
    type: merge ? "merge" : conventional ? conventional[1].toLowerCase() : revert ? "revert" : "other",
    scope: conventional?.[2]?.trim() ?? "",
    breaking, breakingNote: breaking ? (footer?.[1].replace(/\s*\n\s*/g, " ").trim() || description) : "",
    description, subject, body,
    issues: found, issueList: found.filter(i => !description.includes(i.id)).map(i => i.id).join(", "),
    commitUrl: template.commitUrl ? template.commitUrl.replaceAll("{hash}", commit.hash) : "",
  };
}

// ── Template: front matter ───────────────────────────────────────────────────

type YamlValue = string | string[] | Record<string, string | string[]>[];

function scalar(text: string): string | string[] {
  const value = text.trim();
  if (value.startsWith("[") && value.endsWith("]")) return list(value.slice(1, -1));
  if (value.length >= 2 && (value[0] === '"' || value[0] === "'") && value.endsWith(value[0])) return value.slice(1, -1);
  return value;
}

function list(text: string): string[] {
  return text.split(",").map(part => {
    const value = part.trim();
    return value.length >= 2 && (value[0] === '"' || value[0] === "'") && value.endsWith(value[0]) ? value.slice(1, -1) : value;
  }).filter(Boolean);
}

/** The small YAML subset a template needs: `key: value`, `[a, b]` lists and a list of `- key: value` maps. */
export function parseFrontMatter(text: string): Record<string, YamlValue> {
  const result: Record<string, YamlValue> = {};
  let listKey = "";
  let item: Record<string, string | string[]> | null = null;
  text.split("\n").forEach((raw, index) => {
    const line = raw.replace(/\s+$/, "");
    if (!line.trim() || line.trim().startsWith("#")) return;
    const fail = (): never => { throw new Error(`Template front matter, line ${index + 2}: cannot read "${line.trim()}".`); };
    const indented = /^\s/.test(line);
    if (!indented) {
      const match = /^([A-Za-z][\w-]*):(.*)$/.exec(line) ?? fail();
      item = null;
      if (match[2].trim()) { result[match[1]] = scalar(match[2]); listKey = ""; }
      else { result[match[1]] = []; listKey = match[1]; }
      return;
    }
    if (!listKey) fail();
    const entry = /^\s*-\s*(.*)$/.exec(line);
    const target = result[listKey] as Record<string, string | string[]>[];
    if (entry) {
      const pair = /^([A-Za-z][\w-]*):(.*)$/.exec(entry[1]);
      if (pair) { item = { [pair[1]]: scalar(pair[2]) }; target.push(item); }
      else { item = null; (result[listKey] as unknown as string[]).push(entry[1].trim()); }
      return;
    }
    const pair = /^\s+([A-Za-z][\w-]*):(.*)$/.exec(line);
    if (!pair || !item) fail();
    item![pair![1]] = scalar(pair![2]);
  });
  return result;
}

const asList = (value: YamlValue | undefined): string[] =>
  value === undefined ? [] : typeof value === "string" ? list(value) : (value as unknown[]).filter((v): v is string => typeof v === "string");

const asText = (value: YamlValue | undefined): string => (typeof value === "string" ? value : "");

export function parseTemplate(source: string, fallbackName = "Template"): ReleaseTemplate {
  const text = source.replace(/\r\n/g, "\n").replace(/^﻿/, "");
  let meta: Record<string, YamlValue> = {};
  let body = text;
  const front = /^---\n([\s\S]*?)\n---[ \t]*(?:\n|$)/.exec(text);
  if (front) { meta = parseFrontMatter(front[1]); body = text.slice(front[0].length); }
  const groups = Array.isArray(meta.groups) ? (meta.groups as unknown[]).map((group, i) => {
    if (!group || typeof group !== "object") throw new Error(`Template group ${i + 1} needs a title and types.`);
    const g = group as Record<string, string | string[]>;
    const title = typeof g.title === "string" ? g.title : "";
    const types = (typeof g.types === "string" ? list(g.types) : g.types ?? []).map(t => t.toLowerCase());
    if (!title || !types.length) throw new Error(`Template group ${i + 1} needs a title and types.`);
    return { title, types };
  }) : [];
  const template = {
    name: asText(meta.name) || fallbackName, description: asText(meta.description),
    groups: groups.length ? groups : [{ title: "Changes", types: ["*"] }],
    exclude: asList(meta.exclude).map(t => t.toLowerCase()),
    issueUrl: asText(meta.issueUrl), commitUrl: asText(meta.commitUrl), body,
  };
  compile(template.body); // Report a broken body now, not at the first render.
  return template;
}

// ── Template: body (Mustache subset) ─────────────────────────────────────────

type Node = string | { kind: "var"; name: string } | { kind: "section" | "inverted"; name: string; children: Node[] };

function compile(body: string): Node[] {
  // A section tag alone on its line leaves no empty line behind.
  const source = body.replace(/^[ \t]*(\{\{[#^/!][^{}]*\}\})[ \t]*(?:\n|$)/gm, "$1");
  const root: Node[] = [];
  const stack: { name: string; children: Node[] }[] = [{ name: "", children: root }];
  const tag = /\{\{([#^/!]?)\s*([^{}]*?)\s*\}\}/g;
  let last = 0;
  for (const match of source.matchAll(tag)) {
    const top = stack[stack.length - 1];
    if (match.index! > last) top.children.push(source.slice(last, match.index));
    last = match.index! + match[0].length;
    const [, sigil, name] = match;
    if (sigil === "!") continue;
    if (!name) throw new Error(`Template tag "${match[0]}" has no name.`);
    if (sigil === "#" || sigil === "^") {
      const node = { kind: sigil === "#" ? "section" as const : "inverted" as const, name, children: [] as Node[] };
      top.children.push(node); stack.push(node);
    } else if (sigil === "/") {
      if (stack.length === 1 || top.name !== name) throw new Error(`Template tag {{/${name}}} does not close ${stack.length === 1 ? "any section" : `{{#${top.name}}}`}.`);
      stack.pop();
    } else top.children.push({ kind: "var", name });
  }
  if (stack.length > 1) throw new Error(`Template section {{#${stack[stack.length - 1].name}}} is never closed.`);
  if (last < source.length) root.push(source.slice(last));
  return root;
}

function lookup(stack: unknown[], name: string): unknown {
  if (name === ".") return stack[stack.length - 1];
  const [head, ...rest] = name.split(".");
  for (let i = stack.length - 1; i >= 0; i--) {
    const scope = stack[i];
    if (scope && typeof scope === "object" && head in (scope as object)) {
      return rest.reduce<unknown>((value, key) => value && typeof value === "object" ? (value as Record<string, unknown>)[key] : undefined, (scope as Record<string, unknown>)[head]);
    }
  }
  return undefined;
}

function text(value: unknown): string {
  if (value === undefined || value === null || value === false) return "";
  if (Array.isArray(value)) return value.filter(v => typeof v !== "object").join(", ");
  return typeof value === "object" ? "" : String(value);
}

function renderNodes(nodes: Node[], stack: unknown[]): string {
  let out = "";
  for (const node of nodes) {
    if (typeof node === "string") { out += node; continue; }
    const value = lookup(stack, node.name);
    if (node.kind === "var") { out += text(value); continue; }
    const empty = !value || (Array.isArray(value) && value.length === 0);
    if (node.kind === "inverted") { if (empty) out += renderNodes(node.children, stack); continue; }
    if (empty) continue;
    if (Array.isArray(value)) for (const item of value) out += renderNodes(node.children, [...stack, item]);
    else out += renderNodes(node.children, [...stack, value]);
  }
  return out;
}

export function renderTemplate(body: string, context: Record<string, unknown>): string {
  return renderNodes(compile(body), [context]);
}

// ── Putting it together ──────────────────────────────────────────────────────

export function classify(commits: GitCommit[], template: ReleaseTemplate): { groups: { title: string; items: ChangeItem[] }[]; included: ChangeItem[]; hidden: ChangeItem[] } {
  const groups = template.groups.map(g => ({ title: g.title, types: g.types, items: [] as ChangeItem[] }));
  const included: ChangeItem[] = [];
  const hidden: ChangeItem[] = [];
  for (const commit of commits) {
    const item = parseCommit(commit, template);
    // A breaking change is never hidden, whatever the template excludes.
    const excluded = !item.breaking && template.exclude.includes(item.type);
    const group = excluded ? undefined
      : groups.find(g => g.types.includes(item.type)) ?? groups.find(g => g.types.includes("*"))
        ?? (item.breaking ? groups[groups.length - 1] : undefined);
    if (!group) { hidden.push(item); continue; }
    group.items.push(item); included.push(item);
  }
  return { groups: groups.filter(g => g.items.length).map(({ title, items }) => ({ title, items })), included, hidden };
}

export function releaseContext(range: Pick<GitRange, "commits" | "truncated">, template: ReleaseTemplate, options: ReleaseOptions) {
  const { groups, included, hidden } = classify(range.commits, template);
  const breaking = included.filter(item => item.breaking);
  const counts = new Map<string, number>();
  for (const item of included) counts.set(item.author, (counts.get(item.author) ?? 0) + 1);
  const contributors = [...counts].map(([name, commits]) => ({ name, commits }))
    .sort((a, b) => b.commits - a.commits || a.name.localeCompare(b.name));
  return {
    context: {
      version: options.version, date: options.date, repository: options.repository,
      from: options.from, to: options.to,
      range: options.from ? `${options.from}..${options.to}` : `up to ${options.to}`,
      groups: groups.map(g => ({ ...g, count: g.items.length })), commits: included,
      breaking, hasBreaking: breaking.length > 0,
      contributors, contributorList: contributors.map(c => c.name).join(", "),
      stats: { commits: included.length, hidden: hidden.length, total: range.commits.length, contributors: contributors.length },
      truncated: range.truncated,
    },
    included, hidden,
  };
}

export function renderReleaseNotes(range: Pick<GitRange, "commits" | "truncated">, template: ReleaseTemplate, options: ReleaseOptions): ReleaseResult {
  const { context, included, hidden } = releaseContext(range, template, options);
  const markdown = renderTemplate(template.body, context).replace(/\n{3,}/g, "\n\n").trim() + "\n";
  return { markdown, included, hidden };
}

/**
 * `## Heading` blocks become sections; a leading `# Title` names the document.
 * Headings inside fenced code are text, not structure.
 */
export function markdownToSections(markdown: string, fallbackTitle: string): { title: string | null; sections: Section[] } {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  let title: string | null = null;
  let fence = "";
  const sections: { title: string; lines: string[] }[] = [];
  let current: { title: string; lines: string[] } | null = null;
  const preamble: string[] = [];
  for (const line of lines) {
    const marker = /^\s*(```+|~~~+)/.exec(line);
    if (marker) fence = !fence ? marker[1] : marker[1].startsWith(fence) ? "" : fence;
    const h2: RegExpExecArray | null = !fence && !marker ? /^##[ \t]+(.+?)[ \t#]*$/.exec(line) : null;
    const h1: RegExpExecArray | null = !fence && !marker && !current && title === null ? /^#[ \t]+(.+?)[ \t#]*$/.exec(line) : null;
    if (h2) { current = { title: h2[1], lines: [] }; sections.push(current); }
    else if (h1) title = h1[1];
    else (current ? current.lines : preamble).push(line);
  }
  const content = (block: string[]) => block.join("\n").trim();
  const result: Section[] = sections.map(s => ({ title: s.title, content: content(s.lines), diagrams: [] }));
  if (content(preamble)) result.unshift({ title: title ?? fallbackTitle, content: content(preamble), diagrams: [] });
  if (!result.length) result.push({ title: title ?? fallbackTitle, content: "", diagrams: [] });
  return { title, sections: result };
}

// ── Built-in templates ───────────────────────────────────────────────────────

export const BUILTIN_TEMPLATES: { id: string; source: string }[] = [
  {
    id: "builtin:keep-a-changelog",
    source: `---
name: Keep a Changelog
description: One "## [version] - date" block per release, ready to prepend to CHANGELOG.md.
groups:
  - title: Added
    types: feat
  - title: Fixed
    types: fix
  - title: Changed
    types: perf, refactor
  - title: Removed
    types: revert
  - title: Other
    types: "*"
exclude: chore, ci, build, test, style, docs, merge
---
## [{{version}}] - {{date}}
{{#hasBreaking}}

### Breaking changes
{{#breaking}}
- {{#scope}}**{{scope}}:** {{/scope}}{{breakingNote}}
{{/breaking}}
{{/hasBreaking}}
{{#groups}}

### {{title}}
{{#items}}
- {{#scope}}**{{scope}}:** {{/scope}}{{description}}{{#issueList}} ({{issueList}}){{/issueList}}
{{/items}}
{{/groups}}
{{^commits}}

_No user-visible changes._
{{/commits}}
`,
  },
  {
    id: "builtin:technical",
    source: `---
name: Technical release notes
description: For the team: every change with its commit, author and issues, plus contributors.
groups:
  - title: Features
    types: feat
  - title: Bug fixes
    types: fix
  - title: Performance
    types: perf
  - title: Refactoring
    types: refactor
  - title: Documentation
    types: docs
  - title: Build and CI
    types: build, ci
  - title: Tests
    types: test
  - title: Other changes
    types: "*"
exclude: merge, style
# issueUrl: https://github.com/<org>/<repo>/issues/{id}
# commitUrl: https://github.com/<org>/<repo>/commit/{hash}
---
# Release {{version}}

{{date}} · {{repository}} · {{range}} · {{stats.commits}} changes from {{stats.contributors}} contributors
{{#truncated}}

> Only the newest 2000 commits of this range are listed.
{{/truncated}}
{{#hasBreaking}}

## Breaking changes
{{#breaking}}
- {{#scope}}**{{scope}}:** {{/scope}}{{breakingNote}} ({{shortHash}})
{{/breaking}}
{{/hasBreaking}}
{{#groups}}

## {{title}} ({{count}})
{{#items}}
- {{#scope}}**{{scope}}:** {{/scope}}{{description}} — {{author}}, {{#commitUrl}}[{{shortHash}}]({{commitUrl}}){{/commitUrl}}{{^commitUrl}}\`{{shortHash}}\`{{/commitUrl}}{{#issueList}} · {{issueList}}{{/issueList}}
{{/items}}
{{/groups}}

## Contributors
{{#contributors}}
- {{name}} ({{commits}})
{{/contributors}}
`,
  },
  {
    id: "builtin:customer-sk",
    source: `---
name: Poznámky k vydaniu pre zákazníkov (SK)
description: Novinky, opravy a kroky pri aktualizácii; technické zmeny sú skryté. Texty commitov potom preformuluj cez AI Rework.
groups:
  - title: Novinky
    types: feat
  - title: Opravy
    types: fix
  - title: Vylepšenia výkonu
    types: perf
exclude: chore, ci, build, test, style, docs, refactor, merge, other
---
# Vydanie {{version}}

Dátum vydania: {{date}}

## Zhrnutie

<!-- Dve až tri vety o tom, čo vydanie prináša používateľom. -->
{{#groups}}

## {{title}}
{{#items}}
- {{description}}{{#issueList}} ({{issueList}}){{/issueList}}
{{/items}}
{{/groups}}
{{#hasBreaking}}

## Dôležité zmeny a kroky pri aktualizácii
{{#breaking}}
- {{breakingNote}}
{{/breaking}}
{{/hasBreaking}}

## Známe obmedzenia

- Žiadne známe obmedzenia.
`,
  },
];

/** A starting point for a team's own template, with every field documented. */
export const TEMPLATE_HELP = `Front matter (between the --- lines):
  name, description        shown in the template list
  groups                   "- title: …" with "types: feat, fix"; the first matching group wins, "*" takes the rest
  exclude                  types left out (breaking changes are never left out)
  issueUrl / commitUrl     links, with {id} / {hash}
Body: {{version}} {{date}} {{repository}} {{range}} {{stats.commits}} {{stats.hidden}}
  {{#groups}} {{title}} {{count}} {{#items}} … {{/items}} {{/groups}}
  item: {{description}} {{scope}} {{type}} {{shortHash}} {{author}} {{date}} {{issueList}} {{commitUrl}} {{breakingNote}} {{body}}
  {{#breaking}} … {{/breaking}}  {{#hasBreaking}} … {{/hasBreaking}}  {{#contributors}} {{name}} {{commits}} {{/contributors}}
  {{^list}} … {{/list}} renders when a list is empty; {{! comment }}`;
