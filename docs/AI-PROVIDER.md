# Local AI configuration (E0) — Python engine, no longer used for generation

> **Docs / Diagram / Auto generujú cez rovnaký provider layer ako Rework** (Ollama, OpenAI API,
> Gemini API, Claude API, Claude/Codex/Gemini CLI, nastavené v dialógu **AI settings**) — pozri
> [AI-REWORK.md](AI-REWORK.md). Premenné prostredia na tejto stránke a Python engine, ktorý
> popisujú, generovanie dokumentácie ani diagramov už nepohánia; AI settings sa v Ruste rieši
> úplne inou vrstvou (`src-tauri/src/ai/`), o ktorej je reč v AI-REWORK.md.
>
> Čo z Python engine zostáva: `python-engine/agent.py`, DSPy a jeho testy sú stále v repozitári a
> `python_runtime::initialize()` (inicializácia PyO3) beží pri štarte portable buildu ako offline
> kontrola, že bundlovaný Python interpreter a moduly fungujú — nič viac. Príkazy
> `generate_documentation`/`generate_diagram` v `commands.rs` zostávajú zaregistrované (spätná
> kompatibilita nástrojov, ktoré by ich volali priamo), ale `run_generation_job` (runtime.rs) ich
> už nevolá pre žiadnu úlohu.

The Python engine uses local Ollama only. Importing `agent` no longer lists models,
generates a test prompt, selects another model, or sets a global DSPy LM.
The bundled LiteLLM price metadata is selected before importing DSPy, avoiding its
unnecessary remote metadata request. Ollama remains an external prerequisite for the engine
itself, even though no ArchGen feature currently drives it.

Set these environment variables **before launching the desktop application**:

| Variable | Default | Meaning |
| --- | --- | --- |
| `ARCHGEN_AI_PROVIDER` | `ollama` | Only supported provider; other values fail explicitly. |
| `ARCHGEN_AI_MODEL` | `llama3.2:3b` | Exact requested Ollama model name; no discovery or fallback. |
| `ARCHGEN_OLLAMA_URL` | `http://localhost:11434` | Loopback HTTP(S) origin only, without credentials, path or query. |
| `ARCHGEN_AI_TIMEOUT_SECONDS` | `120` | Positive finite per-provider-request timeout in seconds. |

Example in PowerShell:

```powershell
$env:ARCHGEN_AI_MODEL = 'qwen3:8b'
$env:ARCHGEN_AI_TIMEOUT_SECONDS = '180'
```

Install the selected model separately with `ollama pull <model>` and ensure Ollama
is running (`ollama serve` when needed). Then launch ArchGen from the configured
environment. This change does not add a settings screen or persist credentials.

An explicit Python diagnostic is available from `python-engine`:

```powershell
python -c "from ai_provider import health_check; print(health_check())"
```

It reads the configured model's metadata using Ollama `show`; it never generates
text or downloads a model. `ready` means metadata is available, not that inference
has been tested. Connection, missing-model, timeout and other failures become
actionable `AIProviderError` messages without raw prompts or response bodies.

Each public generation call creates its own LM and enters `dspy.context(lm=...)`.
Entire generation jobs are serialized with a lock because the current predictors
and graph are shared. DSPy's context uses context variables inherited by supported
LangGraph execution; global `dspy.configure()` is not used from Rust worker threads.
Provider transport retries are disabled. The timeout applies to each provider
request, **not the whole document generation or time waiting for the lock**.
Cancellation and end-to-end deadlines remain future work.

Offline verification:

```powershell
python -B -m unittest discover -s python-engine -p 'test_*.py' -v
```

Tests mock AI dependencies, do not require Ollama and do not create dependency disk
caches. They cover import behavior, configuration, error handling, serialized
concurrent jobs, context cleanup and the existing Rust-visible entrypoints.
An additional smoke check against the installed DSPy/LangGraph packages, with LM
and disk cache mocked, verified context propagation through four C4 sections,
three diagrams and validation, followed by restoration of the original context.
Live inference, model quality and native desktop integration require a separate
smoke test. Requirements are currently unpinned; dependency version pinning and
the full A3 provider capability/schema interface remain outside this increment.
