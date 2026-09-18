// The only place that knows the AI IPC wire shapes. Every Tauri argument name
// here is camelCase and spelled exactly as `ai_configure`/`ai_set_key` expect —
// Tauri silently drops a misspelt optional argument instead of failing, so the
// wrapper functions are the single point that can get this right or wrong.
import type { RuntimeInvoke, GenerationRequest } from "./runtime";
import type { BlockSelection } from "./selection";

export type ProviderId = "ollama" | "openai" | "gemini" | "anthropic" | "claude_cli" | "codex_cli" | "gemini_cli";

export interface ProviderStatus {
  id: ProviderId;
  label: string;
  local: boolean;
  cli: boolean;
  recipient: string;
  model: string;
  defaultModel: string;
  available: boolean;
  detail: string;
  keySource: "env" | "settings" | "none" | "not_needed";
  notice: string;
}

export interface AiStatus {
  provider: ProviderId;
  providers: ProviderStatus[];
  ollamaUrl: string;
  timeoutSeconds: number;
}

export function fetchAiStatus(invoke: RuntimeInvoke): Promise<AiStatus> {
  return invoke<AiStatus>("ai_status", {});
}

export interface ConfigureAiOptions {
  provider?: ProviderId;
  model?: string;
  ollamaUrl?: string;
  timeoutSeconds?: number;
}

// Every field is optional on the wire: an omitted key leaves that setting
// untouched, so `undefined` here must never turn into a sent `null`/`undefined`.
export function configureAi(invoke: RuntimeInvoke, options: ConfigureAiOptions): Promise<AiStatus> {
  const args: Record<string, unknown> = {};
  if (options.provider !== undefined) args.provider = options.provider;
  if (options.model !== undefined) args.model = options.model;
  if (options.ollamaUrl !== undefined) args.ollamaUrl = options.ollamaUrl;
  if (options.timeoutSeconds !== undefined) args.timeoutSeconds = Math.round(options.timeoutSeconds);
  return invoke<AiStatus>("ai_configure", args);
}

// An empty key removes it; there is no separate "remove key" command.
export function setAiKey(invoke: RuntimeInvoke, provider: ProviderId, key: string): Promise<AiStatus> {
  return invoke<AiStatus>("ai_set_key", { provider, key });
}

export function fetchOllamaModels(invoke: RuntimeInvoke): Promise<string[]> {
  return invoke<string[]>("ai_ollama_models", {});
}

export function testProvider(invoke: RuntimeInvoke, provider: ProviderId): Promise<string> {
  return invoke<string>("ai_test_provider", { provider });
}

// The provider the UI is showing right now: the id Rust will be asked to confirm
// at job creation. `undefined` when the configured id is not in `providers`
// (an empty list, or a settings file naming a provider that no longer exists).
export function currentProvider(status: AiStatus): ProviderStatus | undefined {
  return status.providers.find(p => p.id === status.provider);
}

// Document scope always sends empty target lists and includeContext:false:
// whole-document rework must be chosen with nothing selected, never fall out
// of a stale or pruned selection, and "the rest of the document" is meaningless
// once the whole document is already the target.
export function buildReworkRequest(
  selection: BlockSelection,
  scope: "selection" | "document",
  instruction: string,
  language: string,
  includeContext: boolean,
  status: AiStatus,
): GenerationRequest {
  return {
    kind: "rework",
    instruction,
    language,
    scope,
    sectionIds: scope === "selection" ? selection.sectionIds : [],
    diagramIds: scope === "selection" ? selection.diagramIds : [],
    includeContext: scope === "selection" && includeContext,
    provider: status.provider,
    model: currentProvider(status)?.model ?? "",
  };
}

// Only a local provider keeps content on this computer; every other provider
// must be named before anything is sent, plus its `notice` when it has one.
// Scope "none" (Rework mode, nothing ticked, whole document not armed) has
// nothing to consent to yet, so it renders no text at all.
export function consentText(status: AiStatus, scope: "selection" | "document" | "none", includeContext: boolean): string {
  const provider = currentProvider(status);
  if (!provider || provider.local || scope === "none") return "";
  const target = scope === "document" ? "the whole document"
    : includeContext ? "the selected content and the rest of the document (as context)"
    : "the selected content";
  const sentence = `Sends ${target} to ${provider.recipient} via ${provider.label}.`;
  return provider.notice ? `${sentence} ${provider.notice}` : sentence;
}

export function providerChip(provider: ProviderStatus): string {
  return `${provider.label} · ${provider.model || "default model"}`;
}
