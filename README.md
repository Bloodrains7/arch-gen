# ArchGen (Architecture & Diagram Generator)

A Rust-based tool for generating, visualizing, and validating UML, BPMN, and Archimate diagrams, with AI-driven workflows powered by LangGraph and DSPy.

## Features
- **AI-Driven Generation:** Utilizes LangGraph for agentic diagram reasoning and DSPy for optimized prompt/logic generation.
- **Multi-Format Support:**
  - UML (Class, Sequence, State, etc.)
  - BPMN (Business Process Model and Notation)
  - Archimate (Enterprise Architecture)
- **Visualisation & Integration:**
  - Native integration with **Microsoft Visio** (via COM).
  - Native integration with **Sparx Enterprise Architect (EA)** (via COM).
  - Export to Mermaid/PlantUML for quick previews.
- **Documentation:** Automatic generation and validation of architectural documentation against the diagrams.

## Working with a project
- A project is a **folder of plain files** (Markdown sections, `.puml`/`.mmd` diagrams, small JSON
  manifests) meant to live in Git: see [docs/PROJECT-FORMAT.md](docs/PROJECT-FORMAT.md).
- **AI rework of selected blocks**: tick sections or diagrams, write an instruction, and review the
  proposal before anything changes. Providers: local Ollama, OpenAI / Gemini / Anthropic APIs, or the
  Claude, Codex and Gemini CLIs with their own login. The UI says who receives the content before it
  is sent; API keys are stored with Windows DPAPI outside the project folder. See
  [docs/AI-REWORK.md](docs/AI-REWORK.md).

## Architecture
The project uses a hybrid approach:
1. **Rust Core:** Manages the CLI, file system, and native Windows COM interfaces for Visio and EA.
2. **Python Engine:** Runs LangGraph and DSPy for complex AI workflows, bridged via pyo3.

## Prerequisites
- Rust (latest stable)
- Python 3.9+ (with langgraph, dspy-ai, pydantic)
- Microsoft Visio / Enterprise Architect (for native visualization)
