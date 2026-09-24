import { test } from "node:test";
import assert from "node:assert/strict";
import {
  parseCommit, parseTemplate, parseFrontMatter, renderTemplate, renderReleaseNotes, classify,
  markdownToSections, BUILTIN_TEMPLATES,
} from "../src/lib/release-notes.ts";

let n = 0;
const commit = (subject, body = "", extra = {}) => {
  n++;
  const hash = n.toString(16).padStart(40, "a");
  return { hash, shortHash: hash.slice(0, 7), parents: 1, author: "Ján Novák", date: "2026-09-20T10:00:00+02:00", subject, body, ...extra };
};
const options = { version: "1.4.0", date: "2026-09-24", from: "v1.3.0", to: "HEAD", repository: "billing" };
const builtin = id => parseTemplate(BUILTIN_TEMPLATES.find(t => t.id === id).source);

test("conventional commits give type, scope, breaking notes and issues", () => {
  const feat = parseCommit(commit("feat(api)!: remove v1 endpoints (#42)", "Details.\n\nBREAKING CHANGE: clients must call /v2\nand re-authenticate.\nRefs: PAY-17, UTF-8 stays text"));
  assert.equal(feat.type, "feat");
  assert.equal(feat.scope, "api");
  assert.equal(feat.breaking, true);
  assert.equal(feat.breakingNote, "clients must call /v2 and re-authenticate.");
  assert.equal(feat.description, "remove v1 endpoints (#42)");
  assert.deepEqual(feat.issues.map(i => i.id), ["#42", "PAY-17"]);
  assert.equal(feat.issueList, "PAY-17");
  assert.equal(feat.date, "2026-09-20");

  const bang = parseCommit(commit("refactor!: drop Node 18"));
  assert.equal(bang.breakingNote, "drop Node 18");
  const plain = parseCommit(commit("Update readme"));
  assert.deepEqual([plain.type, plain.scope, plain.breaking, plain.description], ["other", "", false, "Update readme"]);
  assert.equal(parseCommit(commit("Merge pull request #7 from x/y")).type, "merge");
  assert.equal(parseCommit(commit("feat: merged work", "", { parents: 2 })).type, "merge");
  const revert = parseCommit(commit('Revert "feat: risky"'));
  assert.deepEqual([revert.type, revert.description], ["revert", "Revert feat: risky"]);
  const linked = parseCommit(commit("fix: x #9"), { issueUrl: "https://tracker/issues/{id}", commitUrl: "https://git/commit/{hash}" });
  assert.deepEqual(linked.issues, [{ id: "#9", url: "https://tracker/issues/9" }]);
  assert.match(linked.commitUrl, /^https:\/\/git\/commit\/[0-9a]{40}$/);
});

test("front matter reads scalars, inline lists, list of maps and comments", () => {
  const meta = parseFrontMatter(`name: "Team notes"
# a comment
groups:
  - title: Features
    types: [feat, feature]
  - title: Fixes
    types: fix
exclude:
  - chore
  - ci
issueUrl: https://jira.example/browse/{id}`);
  assert.equal(meta.name, "Team notes");
  assert.deepEqual(meta.groups, [{ title: "Features", types: ["feat", "feature"] }, { title: "Fixes", types: "fix" }]);
  assert.deepEqual(meta.exclude, ["chore", "ci"]);
  assert.equal(meta.issueUrl, "https://jira.example/browse/{id}");
  assert.throws(() => parseFrontMatter("groups:\n  types: feat"), /line 3/);
  assert.throws(() => parseFrontMatter("not yaml at all"), /line 2/);
});

test("templates without front matter still render and broken bodies are reported", () => {
  const template = parseTemplate("{{#groups}}{{#items}}- {{description}}\n{{/items}}{{/groups}}", "Plain");
  assert.equal(template.name, "Plain");
  assert.deepEqual(template.groups, [{ title: "Changes", types: ["*"] }]);
  assert.throws(() => parseTemplate("{{#groups}}x"), /never closed/);
  assert.throws(() => parseTemplate("{{#a}}x{{/b}}"), /does not close/);
  assert.throws(() => parseTemplate("---\ngroups:\n  - title: Only title\n---\nx"), /needs a title and types/);
});

test("mustache subset: variables, sections, inverted sections, dotted names and standalone lines", () => {
  const body = "# {{title}}\n{{#items}}\n- {{name}} ({{meta.count}}){{#tag}} [{{.}}]{{/tag}}\n{{/items}}\n{{^items}}\nnone\n{{/items}}\n{{! hidden }}\nend {{missing}}\n";
  assert.equal(renderTemplate(body, { title: "T", items: [{ name: "a", meta: { count: 2 }, tag: "x" }, { name: "b", meta: { count: 0 } }] }),
    "# T\n- a (2) [x]\n- b (0)\nend \n");
  assert.equal(renderTemplate(body, { title: "T", items: [] }), "# T\nnone\nend \n");
  // Outer names stay visible inside a section.
  assert.equal(renderTemplate("{{#list}}{{prefix}}{{.}} {{/list}}", { prefix: "#", list: [1, 2] }), "#1 #2 ");
});

test("classification hides excluded types but never a breaking change", () => {
  const template = builtin("builtin:keep-a-changelog");
  const { groups, included, hidden } = classify([
    commit("feat: invoices"), commit("fix(pdf): margins"), commit("chore: bump"), commit("chore!: drop Windows 7"),
    commit("perf: cache"), commit("Tweak build script"), commit("Merge branch 'x'", "", { parents: 2 }),
  ], template);
  assert.deepEqual(groups.map(g => [g.title, g.items.map(i => i.description)]), [
    ["Added", ["invoices"]], ["Fixed", ["margins"]], ["Changed", ["cache"]], ["Other", ["drop Windows 7", "Tweak build script"]],
  ]);
  assert.equal(included.length, 5);
  assert.deepEqual(hidden.map(h => h.type), ["chore", "merge"]);
});

test("Keep a Changelog output is clean Markdown and becomes one section per release", () => {
  const range = { truncated: false, commits: [
    commit("feat(api): add refunds (#12)"), commit("fix: rounding in totals"), commit("feat!: new tax model", "BREAKING CHANGE: VAT is now per line."),
  ] };
  const { markdown, hidden } = renderReleaseNotes(range, builtin("builtin:keep-a-changelog"), options);
  assert.equal(markdown, `## [1.4.0] - 2026-09-24

### Breaking changes
- VAT is now per line.

### Added
- **api:** add refunds (#12)
- new tax model

### Fixed
- rounding in totals
`);
  assert.equal(hidden.length, 0);
  const { title, sections } = markdownToSections(markdown, "Release notes");
  assert.equal(title, null);
  assert.deepEqual(sections.map(s => s.title), ["[1.4.0] - 2026-09-24"]);
  assert.match(sections[0].content, /^### Breaking changes/);
  assert.equal(renderReleaseNotes({ truncated: false, commits: [commit("chore: x")] }, builtin("builtin:keep-a-changelog"), options).markdown,
    "## [1.4.0] - 2026-09-24\n\n_No user-visible changes._\n");
});

test("technical and Slovak customer templates render every placeholder they use", () => {
  const range = { truncated: true, commits: [commit("feat: export PDF"), commit("docs: guide", "", { author: "Eva" }), commit("chore: deps")] };
  const technical = renderReleaseNotes(range, builtin("builtin:technical"), options).markdown;
  assert.match(technical, /^# Release 1\.4\.0\n\n2026-09-24 · billing · v1\.3\.0\.\.HEAD · 3 changes from 2 contributors/);
  assert.match(technical, /Only the newest 2000 commits/);
  assert.match(technical, /## Features \(1\)\n- export PDF — Ján Novák, `[0-9a]{7}`/);
  assert.match(technical, /## Contributors\n- Ján Novák \(2\)\n- Eva \(1\)/);
  assert.doesNotMatch(technical, /\{\{|\}\}/);

  const customer = renderReleaseNotes(range, builtin("builtin:customer-sk"), options);
  assert.match(customer.markdown, /^# Vydanie 1\.4\.0\n\nDátum vydania: 2026-09-24/);
  assert.match(customer.markdown, /## Novinky\n- export PDF/);
  assert.doesNotMatch(customer.markdown, /guide|deps|Dôležité/);
  assert.equal(customer.hidden.length, 2);
  const doc = markdownToSections(customer.markdown, "Release notes");
  assert.equal(doc.title, "Vydanie 1.4.0");
  assert.deepEqual(doc.sections.map(s => s.title), ["Vydanie 1.4.0", "Zhrnutie", "Novinky", "Známe obmedzenia"]);
  assert.equal(doc.sections[0].content, "Dátum vydania: 2026-09-24");
});

test("headings inside code fences are not sections", () => {
  const { sections } = markdownToSections("## Real\n\n```md\n## Not a heading\n```\n~~~\n# nor this\n~~~\n## Next\ntext", "X");
  assert.deepEqual(sections.map(s => s.title), ["Real", "Next"]);
  assert.match(sections[0].content, /## Not a heading/);
  assert.deepEqual(markdownToSections("", "Fallback").sections, [{ title: "Fallback", content: "", diagrams: [] }]);
});
