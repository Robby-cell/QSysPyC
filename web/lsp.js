import * as wasm from './pkg/qsys_wasm.js';

export function registerLspFeatures(monaco) {
  // Hover Provider
  if (typeof wasm.get_hover === 'function') {
    monaco.languages.registerHoverProvider('python', {
      provideHover: function (model, position) {
        try {
          const json = wasm.get_hover(model.getValue(), position.lineNumber, position.column);
          if (!json) return null;
          const data = JSON.parse(json);
          return {
            range: new monaco.Range(data.start_line, data.start_col, data.end_line, data.end_col),
            contents: data.contents.map(c => ({ value: c }))
          };
        } catch (e) {
          return null;
        }
      }
    });
  }

  // Completion Provider
  if (typeof wasm.get_completions === 'function') {
    monaco.languages.registerCompletionItemProvider('python', {
      triggerCharacters: ['.', '@', ' '],
      provideCompletionItems: function (model, position) {
        try {
          const json = wasm.get_completions(model.getValue(), position.lineNumber, position.column);
          const items = JSON.parse(json);

          const suggestions = items.map(item => ({
            label: item.label,
            kind: monaco.languages.CompletionItemKind[item.kind] || monaco.languages.CompletionItemKind.Property,
            detail: item.detail,
            documentation: item.documentation,
            insertText: item.insert_text,
            insertTextRules: item.is_snippet 
              ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet 
              : monaco.languages.CompletionItemInsertTextRule.None
          }));

          return { suggestions };
        } catch (e) {
          return { suggestions: [] };
        }
      }
    });
  }

  // Inlay Hints Provider
  if (typeof wasm.get_inlay_hints === 'function') {
    monaco.languages.registerInlayHintsProvider('python', {
      provideInlayHints: function (model) {
        try {
          const json = wasm.get_inlay_hints(model.getValue());
          const rawHints = JSON.parse(json);

          const hints = rawHints.map(h => ({
            position: { lineNumber: h.line, column: h.col },
            label: h.label,
            kind: monaco.languages.InlayHintKind.Type,
            paddingLeft: true
          }));

          return { hints, dispose: () => {} };
        } catch (e) {
          return { hints: [], dispose: () => {} };
        }
      }
    });
  }
}
