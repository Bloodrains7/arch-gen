<script lang="ts">
  import { onMount } from "svelte";
  import { monaco } from "../monaco-setup";

  let { value = $bindable(""), language = "markdown", onChange = (_v: string) => {} } = $props();
  let container: HTMLDivElement;
  let editor: monaco.editor.IStandaloneCodeEditor;
  let syncing = false;

  // Define a dark theme matching our app
  function defineTheme() {
    monaco.editor.defineTheme("archgen-dark", {
      base: "vs-dark",
      inherit: true,
      rules: [
        { token: "comment", foreground: "6b7080", fontStyle: "italic" },
        { token: "keyword", foreground: "6c8cff" },
        { token: "string", foreground: "4cce8a" },
        { token: "number", foreground: "f0b756" },
        { token: "type", foreground: "8ba3ff" },
        // Markdown specific
        { token: "keyword.md", foreground: "6c8cff", fontStyle: "bold" },
        { token: "string.link.md", foreground: "6c8cff" },
        { token: "variable.md", foreground: "4cce8a" },
        { token: "markup.heading", foreground: "e4e6ed", fontStyle: "bold" },
      ],
      colors: {
        "editor.background": "#1a1d27",
        "editor.foreground": "#e4e6ed",
        "editor.lineHighlightBackground": "#242836",
        "editor.selectionBackground": "#333849",
        "editorCursor.foreground": "#6c8cff",
        "editorLineNumber.foreground": "#6b7080",
        "editorLineNumber.activeForeground": "#9398a7",
        "editor.inactiveSelectionBackground": "#2d3244",
        "editorWidget.background": "#1a1d27",
        "editorWidget.border": "#333849",
        "input.background": "#242836",
        "input.border": "#333849",
        "scrollbarSlider.background": "#33384966",
        "scrollbarSlider.hoverBackground": "#6b708066",
      },
    });
  }

  onMount(() => {
    defineTheme();

    editor = monaco.editor.create(container, {
      value: value,
      language: language,
      theme: "archgen-dark",
      minimap: { enabled: false },
      fontSize: 13,
      fontFamily: "'Cascadia Code', 'Fira Code', 'Consolas', monospace",
      lineNumbers: "off",
      wordWrap: "on",
      wrappingStrategy: "advanced",
      padding: { top: 12, bottom: 12 },
      scrollBeyondLastLine: false,
      automaticLayout: true,
      renderLineHighlight: "none",
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      overviewRulerBorder: false,
      scrollbar: {
        vertical: "auto",
        horizontal: "hidden",
        verticalScrollbarSize: 6,
      },
      contextmenu: true,
      suggest: {
        showWords: false,
      },
      quickSuggestions: false,
      folding: true,
      lineDecorationsWidth: 8,
      renderWhitespace: "none",
      tabSize: 2,
    });

    // Sync editor content back to parent
    editor.onDidChangeModelContent(() => {
      if (syncing) return;
      value = editor.getValue();
      onChange(value);
    });

    // Auto-resize editor based on content
    const updateHeight = () => {
      const contentHeight = Math.min(
        Math.max(editor.getContentHeight(), 100),
        600
      );
      container.style.height = `${contentHeight}px`;
      editor.layout();
    };

    editor.onDidContentSizeChange(updateHeight);
    updateHeight();

    return () => {
      editor.dispose();
    };
  });

  // Update editor when value changes externally
  $effect(() => {
    if (editor && value !== editor.getValue()) {
      syncing = true;
      try { editor.setValue(value); } finally { syncing = false; }
    }
  });
</script>

<div class="monaco-wrapper">
  <div bind:this={container} class="monaco-container"></div>
</div>

<style>
  .monaco-wrapper {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    transition: border-color 0.15s;
  }

  .monaco-wrapper:focus-within {
    border-color: var(--accent);
  }

  .monaco-container {
    min-height: 100px;
    max-height: 600px;
  }
</style>
