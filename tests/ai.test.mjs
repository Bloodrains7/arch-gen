import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  fetchAiStatus, configureAi, setAiKey, fetchOllamaModels, testProvider,
  buildReworkRequest, currentProvider, consentText, providerChip,
} from "../src/lib/ai.ts";

const fixture = path => JSON.parse(readFileSync(new URL(`./fixtures/${path}`, import.meta.url)));

function recordingInvoke(returns) {
  const calls = [];
  const invoke = async (command, args) => { calls.push({ command, args }); return returns; };
  return { calls, invoke };
}

test("fetchAiStatus sends no arguments", async () => {
  const { calls, invoke } = recordingInvoke("status");
  assert.equal(await fetchAiStatus(invoke), "status");
  assert.deepEqual(calls, [{ command: "ai_status", args: {} }]);
});

test("configureAi omits undefined options and rounds timeoutSeconds to an integer", async () => {
  const { calls, invoke } = recordingInvoke("status");
  await configureAi(invoke, { model: "gpt-5.6" });
  await configureAi(invoke, { provider: "ollama", ollamaUrl: "http://localhost:11434", timeoutSeconds: 42.6 });
  await configureAi(invoke, { timeoutSeconds: 30.4 });
  assert.deepEqual(calls, [
    { command: "ai_configure", args: { model: "gpt-5.6" } },
    { command: "ai_configure", args: { provider: "ollama", ollamaUrl: "http://localhost:11434", timeoutSeconds: 43 } },
    { command: "ai_configure", args: { timeoutSeconds: 30 } },
  ]);
});

test("setAiKey sends the provider and key exactly, including an empty key", async () => {
  const { calls, invoke } = recordingInvoke("status");
  await setAiKey(invoke, "openai", "sk-abc");
  await setAiKey(invoke, "openai", "");
  assert.deepEqual(calls, [
    { command: "ai_set_key", args: { provider: "openai", key: "sk-abc" } },
    { command: "ai_set_key", args: { provider: "openai", key: "" } },
  ]);
});

test("fetchOllamaModels sends no arguments", async () => {
  const { calls, invoke } = recordingInvoke(["qwen3.8:27b"]);
  assert.deepEqual(await fetchOllamaModels(invoke), ["qwen3.8:27b"]);
  assert.deepEqual(calls, [{ command: "ai_ollama_models", args: {} }]);
});

test("testProvider sends only the provider id", async () => {
  const { calls, invoke } = recordingInvoke("Uses gpt-5.6.");
  assert.equal(await testProvider(invoke, "openai"), "Uses gpt-5.6.");
  assert.deepEqual(calls, [{ command: "ai_test_provider", args: { provider: "openai" } }]);
});

test("buildReworkRequest matches the wire fixture", () => {
  const status = {
    provider: "claude_cli", ollamaUrl: "http://localhost:11434", timeoutSeconds: 600,
    providers: [{ id: "claude_cli", label: "Claude CLI", local: false, cli: true, recipient: "Anthropic",
      model: "", defaultModel: "", available: true, detail: "Uses the Claude Code login on this computer.",
      keySource: "not_needed", notice: "" }],
  };
  const selection = { sectionIds: ["section-a", "section-b"], diagramIds: ["diagram-c"] };
  const request = buildReworkRequest(selection, "selection", "Prepíš formálnejšie.", "sk", false, status);
  assert.deepEqual(request, fixture("rework-request.json"));
});

test("buildReworkRequest sends empty target lists for document scope, never the ticked selection", () => {
  const status = {
    provider: "ollama", ollamaUrl: "http://localhost:11434", timeoutSeconds: 600,
    providers: [{ id: "ollama", label: "Ollama (local)", local: true, cli: false, recipient: "this computer",
      model: "qwen3.8:27b", defaultModel: "", available: true, detail: "", keySource: "not_needed", notice: "" }],
  };
  const selection = { sectionIds: ["a"], diagramIds: ["b"] };
  const request = buildReworkRequest(selection, "document", "Rewrite it.", "en", true, status);
  assert.deepEqual(request, {
    kind: "rework", instruction: "Rewrite it.", language: "en", scope: "document",
    sectionIds: [], diagramIds: [], includeContext: false, provider: "ollama", model: "qwen3.8:27b",
  });
});

test("buildReworkRequest keeps includeContext for selection scope", () => {
  const status = {
    provider: "ollama", ollamaUrl: "http://localhost:11434", timeoutSeconds: 600,
    providers: [{ id: "ollama", label: "Ollama (local)", local: true, cli: false, recipient: "this computer",
      model: "qwen3.8:27b", defaultModel: "", available: true, detail: "", keySource: "not_needed", notice: "" }],
  };
  const selection = { sectionIds: ["a"], diagramIds: [] };
  const request = buildReworkRequest(selection, "selection", "Rewrite it.", "en", true, status);
  assert.equal(request.includeContext, true);
});

test("currentProvider finds the configured provider on the fixture and is undefined for an empty list", () => {
  const status = fixture("ai-status.json");
  assert.equal(currentProvider(status)?.id, "ollama");
  assert.equal(currentProvider({ ...status, providers: [] }), undefined);
});

test("consentText is empty for the local provider in every scope", () => {
  const status = fixture("ai-status.json");
  assert.equal(consentText(status, "selection", false), "");
  assert.equal(consentText(status, "document", false), "");
  assert.equal(consentText(status, "none", false), "");
});

test("consentText is empty for scope \"none\" even for a cloud provider: nothing is armed to consent to yet", () => {
  const status = { ...fixture("ai-status.json"), provider: "anthropic" };
  assert.equal(consentText(status, "none", false), "");
});

test("consentText names the recipient and label for a cloud provider in both scopes", () => {
  const status = { ...fixture("ai-status.json"), provider: "anthropic" };
  assert.equal(consentText(status, "selection", false), "Sends the selected content to Anthropic via Claude API.");
  assert.equal(consentText(status, "document", false), "Sends the whole document to Anthropic via Claude API.");
});

test("consentText names the read-only context when includeContext is ticked for selection scope", () => {
  const status = { ...fixture("ai-status.json"), provider: "anthropic" };
  assert.equal(
    consentText(status, "selection", true),
    "Sends the selected content and the rest of the document (as context) to Anthropic via Claude API.",
  );
  // Document scope already sends everything; includeContext must not change its sentence.
  assert.equal(consentText(status, "document", true), "Sends the whole document to Anthropic via Claude API.");
});

test("consentText appends the provider notice when it has one", () => {
  const base = fixture("ai-status.json");
  const status = {
    ...base, provider: "anthropic",
    providers: base.providers.map(p => p.id === "anthropic" ? { ...p, notice: "Uses the paid API, billed per request." } : p),
  };
  assert.equal(consentText(status, "selection", false), "Sends the selected content to Anthropic via Claude API. Uses the paid API, billed per request.");
});

test("providerChip shows the model, or falls back to \"default model\" when none is set", () => {
  const withModel = { id: "anthropic", label: "Claude API", local: false, cli: false, recipient: "Anthropic",
    model: "claude-opus-5", defaultModel: "claude-opus-5", available: true, detail: "", keySource: "env", notice: "" };
  const withoutModel = { ...withModel, id: "claude_cli", label: "Claude CLI", model: "" };
  assert.equal(providerChip(withModel), "Claude API · claude-opus-5");
  assert.equal(providerChip(withoutModel), "Claude CLI · default model");
});
