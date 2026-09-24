import { test } from "node:test";
import assert from "node:assert/strict";
import { MAX_MERMAID_SOURCE_BYTES, mermaidErrorMessage, mermaidSourceProblem, renderMermaidSvg } from "../src/lib/mermaid.ts";

// Pure logic only: actually rendering needs a browser DOM (mermaid.render() and our
// own computed-style inlining both touch `document`), so that path is covered by the
// Playwright e2e suite (tests/e2e/mermaid.spec.mjs) instead.

test("MAX_MERMAID_SOURCE_BYTES mirrors the Rust local renderer's 256 KiB limit", () => {
  assert.equal(MAX_MERMAID_SOURCE_BYTES, 256 * 1024);
});

test("renderMermaidSvg rejects oversized input before ever loading mermaid or touching the DOM", async () => {
  // The size guard is a synchronous check ahead of the dynamic import() of mermaid
  // (which needs a browser DOM), so this must reject cleanly under plain Node too.
  const oversized = "a".repeat(MAX_MERMAID_SOURCE_BYTES + 1);
  await assert.rejects(renderMermaidSvg(oversized), /Diagram exceeds the 256 KiB render limit\./);
});

test("renderMermaidSvg measures UTF-8 bytes, not UTF-16 code units", async () => {
  // "😀" is 2 UTF-16 code units but 4 UTF-8 bytes; a string of these must be judged
  // by byte length the same way the Rust renderer judges `source.len()`.
  const perEmojiBytes = 4;
  const count = Math.floor(MAX_MERMAID_SOURCE_BYTES / perEmojiBytes) + 1;
  const oversized = "\u{1F600}".repeat(count);
  await assert.rejects(renderMermaidSvg(oversized), /256 KiB render limit/);
});

test("mermaidErrorMessage keeps only the first non-blank line, prefixed for the UI", () => {
  const err = new Error("Parse error on line 2:\nflowchart TD A --> \n------------------^\nExpecting 'NEWLINE', got 'EOF'");
  assert.equal(mermaidErrorMessage(err), "Invalid Mermaid diagram: Parse error on line 2:");
});

test("mermaidErrorMessage truncates a very long first line", () => {
  const err = new Error(`${"x".repeat(250)}\nsecond line`);
  const message = mermaidErrorMessage(err);
  assert.ok(message.startsWith("Invalid Mermaid diagram: "));
  assert.ok(message.endsWith("…"));
  assert.ok(message.length < 230);
});

test("mermaidErrorMessage handles a non-Error rejection and an empty message", () => {
  assert.equal(mermaidErrorMessage("boom"), "Invalid Mermaid diagram: boom");
  assert.equal(mermaidErrorMessage(new Error("")), "Invalid Mermaid diagram.");
  assert.equal(mermaidErrorMessage(new Error("   \n  \n")), "Invalid Mermaid diagram.");
});

test("sources that could make Mermaid load something are refused before rendering, plain styling is not", async () => {
  const body = "flowchart LR\n  A[Client] --> B[Server]";
  for (const risky of [
    `%%{init: {"theme": "dark"}}%%\n${body}`,
    `---\ntitle: T\nconfig:\n  themeCSS: ".x{}"\n---\n${body}`,
    `${body}\n  style A fill:url(https://example.invalid/a)`,
    `${body}\n  classDef c fill:u\\rl(https://example.invalid/b)`,
    `${body}\n  linkStyle 0 stroke:rgb(url(https://example.invalid/c))`,
    `${body}\n  style A background:image-set("https:example.invalid" 1x)`,
    `${body}\n  classDef c @import`,
    "C4Context\n  Person(a, \"A\")\n  UpdateElementStyle(a, $bgColor=\"url(https://example.invalid/d)\")",
  ]) {
    assert.ok(mermaidSourceProblem(risky), risky);
    await assert.rejects(renderMermaidSvg(risky), /not supported|plain values/);
  }
  for (const plain of [
    body,
    `---\ntitle: Checkout\n---\n${body}`,
    `${body}\n  style A fill:#f9f,stroke:rgb(51, 51, 51),stroke-width:4px\n  classDef curly fill:hsl(210 50% 50%),color:#fff\n  linkStyle default stroke:rgba(0,0,0,0.5)`,
    "C4Context\n  Person(a, \"A\")\n  UpdateElementStyle(a, $bgColor=\"grey\", $borderColor=\"rgb(1,2,3)\")",
    "flowchart LR\n  A[\"Label with url(x) and C:\\\\path is just text\"] --> B",
  ]) assert.equal(mermaidSourceProblem(plain), null, plain);
});
