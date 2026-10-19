import * as wasm from './pkg/qsys_wasm.js';

export function registerLspFeatures(monaco) {
  // 1. Hover Provider
  if (typeof wasm.get_hover === 'function') {
    monaco.languages.registerHoverProvider('python', {
      provideHover(model, position) {
        try {
          const json = wasm.get_hover(model.getValue(), position.lineNumber, position.column);
          if (!json) return null;
          const data = JSON.parse(json);
          return {
            range: new monaco.Range(data.start_line, data.start_col, data.end_line, data.end_col),
            contents: data.contents.map(c => ({ value: c }))
          };
        } catch (_e) {
          return null;
        }
      }
    });
  }

  // 2. Completion Item Provider (Fault-Tolerant & Context-Aware)
  if (typeof wasm.get_completions === 'function') {
    monaco.languages.registerCompletionItemProvider('python', {
      triggerCharacters: ['.', '@'],
      provideCompletionItems(model, position) {
        try {
          const json = wasm.get_completions(model.getValue(), position.lineNumber, position.column);
          const items = JSON.parse(json);

          const word = model.getWordUntilPosition(position);
          const defaultRange = new monaco.Range(
            position.lineNumber,
            word.startColumn,
            position.lineNumber,
            word.endColumn
          );

          const kindMap = {
            Field: monaco.languages.CompletionItemKind.Field,
            Property: monaco.languages.CompletionItemKind.Property,
            Method: monaco.languages.CompletionItemKind.Method,
            Function: monaco.languages.CompletionItemKind.Function,
            Event: monaco.languages.CompletionItemKind.Event,
            Class: monaco.languages.CompletionItemKind.Class,
            Variable: monaco.languages.CompletionItemKind.Variable,
            Snippet: monaco.languages.CompletionItemKind.Snippet
          };

          const suggestions = items.map((item, idx) => ({
            label: item.label,
            kind: kindMap[item.kind] || monaco.languages.CompletionItemKind.Property,
            detail: item.detail,
            documentation: { value: item.documentation },
            insertText: item.insert_text,
            range: defaultRange,
            sortText: String(idx).padStart(3, '0'),
            insertTextRules: item.is_snippet
              ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
              : monaco.languages.CompletionItemInsertTextRule.None
          }));

          return { suggestions };
        } catch (_e) {
          return { suggestions: [] };
        }
      }
    });
  }

  // 3. Signature Help Provider
  if (typeof wasm.get_signature_help === 'function') {
    monaco.languages.registerSignatureHelpProvider('python', {
      signatureHelpTriggerCharacters: ['(', ','],
      signatureHelpRetriggerCharacters: [','],
      provideSignatureHelp(model, position) {
        try {
          const json = wasm.get_signature_help(
            model.getValue(),
            position.lineNumber,
            position.column
          );
          if (!json) return null;
          const sig = JSON.parse(json);
          return {
            value: {
              signatures: [
                {
                  label: sig.label,
                  documentation: { value: sig.documentation },
                  parameters: (sig.parameters || []).map(p => ({
                    label: p.label,
                    documentation: { value: p.documentation }
                  }))
                }
              ],
              activeSignature: 0,
              activeParameter: sig.active_parameter || 0
            },
            dispose: () => {}
          };
        } catch (_e) {
          return null;
        }
      }
    });
  }

  // 4. Quick-Fix Code Action Provider for `did you mean '...'?` Diagnostics
  monaco.languages.registerCodeActionProvider('python', {
    provideCodeActions(model, _range, context) {
      const actions = [];
      for (const marker of context.markers || []) {
        const match = marker.message && marker.message.match(/did you mean '([^']+)'\?/);
        if (!match) continue;
        const replacement = match[1];

        const markerRange = new monaco.Range(
          marker.startLineNumber,
          marker.startColumn,
          marker.endLineNumber,
          marker.endColumn
        );
        const existingText = model.getValueInRange(markerRange);
        const dotIdx = existingText.lastIndexOf('.');
        const editRange =
          dotIdx !== -1
            ? new monaco.Range(
                marker.startLineNumber,
                marker.startColumn + dotIdx + 1,
                marker.endLineNumber,
                marker.endColumn
              )
            : markerRange;

        actions.push({
          title: `Replace with '${replacement}'`,
          diagnostics: [marker],
          kind: 'quickfix',
          isPreferred: true,
          edit: {
            edits: [
              {
                resource: model.uri,
                versionId: model.getVersionId(),
                textEdit: {
                  range: editRange,
                  text: replacement
                }
              }
            ]
          }
        });
      }
      return { actions, dispose: () => {} };
    }
  });

  // 5. Inlay Hints Provider
  if (typeof wasm.get_inlay_hints === 'function') {
    monaco.languages.registerInlayHintsProvider('python', {
      provideInlayHints(model) {
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
        } catch (_e) {
          return { hints: [], dispose: () => {} };
        }
      }
    });
  }
}

