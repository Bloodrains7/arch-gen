import * as monaco from "monaco-editor";

// Configure Monaco workers using blob URLs (no separate worker files needed)
self.MonacoEnvironment = {
  getWorker(_workerId: string, label: string) {
    const getWorkerModule = (moduleUrl: string) => {
      return new Worker(
        new URL(moduleUrl, import.meta.url),
        { type: "module" }
      );
    };

    switch (label) {
      case "json":
        return getWorkerModule(
          "monaco-editor/esm/vs/language/json/json.worker?worker"
        );
      case "css":
      case "scss":
      case "less":
        return getWorkerModule(
          "monaco-editor/esm/vs/language/css/css.worker?worker"
        );
      case "html":
      case "handlebars":
      case "razor":
        return getWorkerModule(
          "monaco-editor/esm/vs/language/html/html.worker?worker"
        );
      case "typescript":
      case "javascript":
        return getWorkerModule(
          "monaco-editor/esm/vs/language/typescript/ts.worker?worker"
        );
      default:
        return getWorkerModule(
          "monaco-editor/esm/vs/editor/editor.worker?worker"
        );
    }
  },
};

// Register custom Markdown snippets for architects
monaco.languages.registerCompletionItemProvider("markdown", {
  provideCompletionItems(model, position) {
    const word = model.getWordUntilPosition(position);
    const range = {
      startLineNumber: position.lineNumber,
      endLineNumber: position.lineNumber,
      startColumn: word.startColumn,
      endColumn: word.endColumn,
    };

    return {
      suggestions: [
        {
          label: "adr",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "## ADR-${1:001}: ${2:Title}",
            "",
            "**Status:** ${3|Proposed,Accepted,Deprecated,Superseded|}",
            "**Date:** ${4:YYYY-MM-DD}",
            "",
            "### Context",
            "${5:What is the issue?}",
            "",
            "### Decision",
            "${6:What was decided?}",
            "",
            "### Consequences",
            "${7:What are the results?}",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "Architecture Decision Record",
          range,
        },
        {
          label: "table",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "| ${1:Header 1} | ${2:Header 2} | ${3:Header 3} |",
            "|${1/./\\-/g}|${2/./\\-/g}|${3/./\\-/g}|",
            "| ${4} | ${5} | ${6} |",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "Markdown table",
          range,
        },
        {
          label: "checklist",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "- [ ] ${1:Task 1}",
            "- [ ] ${2:Task 2}",
            "- [ ] ${3:Task 3}",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "Checklist",
          range,
        },
        {
          label: "component",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "### ${1:Component Name}",
            "",
            "**Responsibility:** ${2:What does it do?}",
            "**Technology:** ${3:Tech stack}",
            "**Interfaces:**",
            "- Provides: ${4:API/Interface}",
            "- Requires: ${5:Dependencies}",
            "",
            "**Quality Attributes:**",
            "- ${6:Attribute}: ${7:How it's achieved}",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "Architecture component description",
          range,
        },
        {
          label: "risk",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "### Risk: ${1:Risk Title}",
            "",
            "**Probability:** ${2|Low,Medium,High,Critical|}",
            "**Impact:** ${3|Low,Medium,High,Critical|}",
            "**Mitigation:** ${4:Strategy}",
            "**Owner:** ${5:Person}",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "Risk assessment entry",
          range,
        },
        {
          label: "nfr",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "### NFR: ${1:Requirement Name}",
            "",
            "**Category:** ${2|Performance,Security,Availability,Scalability,Maintainability|}",
            "**Metric:** ${3:Measurable target}",
            "**Priority:** ${4|Must,Should,Could,Won't|}",
            "**Verification:** ${5:How to test/verify}",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "Non-functional requirement",
          range,
        },
        {
          label: "plantuml",
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: [
            "```plantuml",
            "@startuml",
            "${1:actor User}",
            "${2:participant System}",
            "",
            "${3:User -> System : Request}",
            "${4:System --> User : Response}",
            "@enduml",
            "```",
          ].join("\n"),
          insertTextRules:
            monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          documentation: "PlantUML diagram block",
          range,
        },
      ],
    };
  },
});

export { monaco };
