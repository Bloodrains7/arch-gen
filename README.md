# ArchGen (Architecture & Diagram Generator)

A desktop tool (Tauri 2 + Svelte 5 + Rust, with a Python AI engine) for architecture documentation
and design: templates, Markdown sections, PlantUML/Mermaid diagrams, AI assistance with review before
anything changes, Git integration and release notes.

## Features
- **Documentation templates** with guidance per section: arc42, C4, TOGAF, Solution Design (structured
  along the Azure Well-Architected pillars), Decision Record (ADR, MADR), Well-Architected Review and
  Release Notes. Applying a template adds its structure and never discards content you wrote.
- **Diagrams**: PlantUML and Mermaid sources per section; PlantUML renders with a private local
  renderer (no upload), the public PlantUML server only on an explicit click.
- **AI**: generate documentation or diagrams (Python engine, local Ollama), and rework selected blocks
  with a provider of your choice — local Ollama, OpenAI / Gemini / Anthropic APIs, or the Claude, Codex
  and Gemini CLIs. Every result is a proposal you review; see [docs/AI-REWORK.md](docs/AI-REWORK.md).
- **Git**: projects are plain files in a folder ([docs/PROJECT-FORMAT.md](docs/PROJECT-FORMAT.md)); the
  app shows the Git status of the project folder, browses and restores its history, commits only the
  project folder and pushes the current branch on request.
- **Release notes from Git**: pick a repository and a commit range (tags, branches), choose a template
  (Keep a Changelog, technical, Slovak customer notes, or your own Markdown template shared in the
  project), and insert the result as a document or save it as a file; see
  [docs/GIT-A-RELEASE-NOTES.md](docs/GIT-A-RELEASE-NOTES.md).
- **Export**: Markdown with front matter (title, project, template, language, date) and a
  self-contained HTML page (opens in Word, prints to PDF) with locally rendered PlantUML diagrams.
- Microsoft Visio and Sparx Enterprise Architect exports are **planned, not implemented**; the buttons
  are disabled and the backend reports that nothing was exported.

## Documentation
- [docs/PRODUCT-DESIGN.md](docs/PRODUCT-DESIGN.md) – product design and roadmap (Slovak)
- [docs/IMPLEMENTATION-PROGRESS.md](docs/IMPLEMENTATION-PROGRESS.md) – what is implemented and verified
- [docs/ZLEPSENIA.md](docs/ZLEPSENIA.md) – review and prioritised next improvements
- [docs/RUNTIME-DISTRIBUTION.md](docs/RUNTIME-DISTRIBUTION.md) – portable Windows build, private Python and renderer

## Architecture
1. **Svelte frontend** (`src/`): editor, templates, review dialogs; pure domain logic in `src/lib/*.ts`.
2. **Rust core** (`src-tauri/`): project files, edit sessions and jobs (SQLite registry), Git, local
   renderer, AI providers.
3. **Python engine** (`python-engine/`): LangGraph + DSPy generation, embedded through PyO3.

## Development
- Prerequisites: Node 22, Rust stable, Python 3 (dev build), Git; Windows is the target platform.
- `npm run tauri dev` – desktop app; `npm run dev` – frontend only (no native commands).
- `npm test`, `npm run check`, `npm run test:e2e` (mocked IPC), `cargo test --manifest-path src-tauri/Cargo.toml`.
