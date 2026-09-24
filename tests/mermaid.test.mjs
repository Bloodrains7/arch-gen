import { test } from "node:test";
import assert from "node:assert/strict";
import { MAX_MERMAID_SOURCE_BYTES, mermaidErrorMessage, renderMermaidSvg } from "../src/lib/mermaid.ts";

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
