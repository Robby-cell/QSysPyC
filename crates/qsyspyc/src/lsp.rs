use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use qsys_compiler::{Analyzer, Compiler};
use qsys_diagnostics::{FileId, SourceFile};

pub struct QSysLspBackend {
    client: Client,
    documents: Arc<RwLock<HashMap<Url, String>>>,
}

impl QSysLspBackend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn validate_document(&self, uri: &Url, text: &str) {
        let file_path = uri.path();
        let file = SourceFile::new(FileId(1), file_path, text);

        let diagnostics = match Compiler::check(text, file_path) {
            Ok(_) => Vec::new(),
            Err(diags) => {
                let markers = diags.to_editor_markers(&file);
                markers
                    .into_iter()
                    .map(|m| Diagnostic {
                        range: Range {
                            start: Position::new(
                                (m.start_line_number.saturating_sub(1)) as u32,
                                (m.start_column.saturating_sub(1)) as u32,
                            ),
                            end: Position::new(
                                (m.end_line_number.saturating_sub(1)) as u32,
                                (m.end_column.saturating_sub(1)) as u32,
                            ),
                        },
                        severity: Some(match m.severity {
                            qsys_diagnostics::Severity::Error => DiagnosticSeverity::ERROR,
                            qsys_diagnostics::Severity::Warning => DiagnosticSeverity::WARNING,
                            qsys_diagnostics::Severity::Info => DiagnosticSeverity::INFORMATION,
                        }),
                        code: Some(NumberOrString::String(m.code)),
                        source: Some("qsyspy".to_string()),
                        message: if let Some(help) = m.help {
                            format!("{}\nHelp: {}", m.message, help)
                        } else {
                            m.message
                        },
                        ..Default::default()
                    })
                    .collect()
            }
        };

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for QSysLspBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string(), "@".to_string()]),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: Some(vec![",".to_string()]),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                inlay_hint_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "qsyspy-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "QSysPy Language Server initialized.")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;

        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.validate_document(&uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents
                .write()
                .await
                .insert(uri.clone(), change.text.clone());
            self.validate_document(&uri, &change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let line = (pos.line + 1) as usize;
        let col = (pos.character + 1) as usize;

        // Query the Rust analyzer directly
        if let Some(h) = Analyzer::hover(text, line, col) {
            Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: h.contents.join("\n\n---\n\n"),
                }),
                range: Some(Range {
                    start: Position::new(
                        (h.start_line.saturating_sub(1)) as u32,
                        (h.start_col.saturating_sub(1)) as u32,
                    ),
                    end: Position::new(
                        (h.end_line.saturating_sub(1)) as u32,
                        (h.end_col.saturating_sub(1)) as u32,
                    ),
                }),
            }))
        } else {
            Ok(None)
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let docs = self.documents.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let line = (pos.line + 1) as usize;
        let col = (pos.character + 1) as usize;

        // Query the Rust compiler directly for completions
        let completions = Analyzer::completions(text, line, col);
        let items: Vec<CompletionItem> = completions
            .into_iter()
            .map(|c| CompletionItem {
                label: c.label,
                kind: Some(match c.kind.as_str() {
                    "Field" => CompletionItemKind::FIELD,
                    "Event" => CompletionItemKind::EVENT,
                    "Method" => CompletionItemKind::METHOD,
                    "Property" => CompletionItemKind::PROPERTY,
                    "Class" => CompletionItemKind::CLASS,
                    _ => CompletionItemKind::TEXT,
                }),
                detail: Some(c.detail),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: c.documentation,
                })),
                insert_text: Some(c.insert_text),
                insert_text_format: Some(if c.is_snippet {
                    InsertTextFormat::SNIPPET
                } else {
                    InsertTextFormat::PLAIN_TEXT
                }),
                ..Default::default()
            })
            .collect();

        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let docs = self.documents.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        // Query the Rust TypeMap directly for inlay hints
        let hints = Analyzer::inlay_hints(text);
        let lsp_hints = hints
            .into_iter()
            .map(|h| InlayHint {
                position: Position::new(
                    (h.line.saturating_sub(1)) as u32,
                    (h.col.saturating_sub(1)) as u32,
                ),
                label: InlayHintLabel::String(h.label),
                kind: Some(InlayHintKind::TYPE),
                padding_left: Some(true),
                padding_right: None,
                tooltip: None,
                data: None,
                text_edits: None,
            })
            .collect();

        Ok(Some(lsp_hints))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let text = match docs.get(&uri) {
            Some(t) => t,
            None => return Ok(None),
        };

        let line = (pos.line + 1) as usize;
        let col = (pos.character + 1) as usize;

        if let Some(sig) = Analyzer::signature_help(text, line, col) {
            let parameters = sig
                .parameters
                .into_iter()
                .map(|p| ParameterInformation {
                    label: ParameterLabel::Simple(p.label),
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: p.documentation,
                    })),
                })
                .collect();

            Ok(Some(SignatureHelp {
                signatures: vec![SignatureInformation {
                    label: sig.label,
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: sig.documentation,
                    })),
                    parameters: Some(parameters),
                    active_parameter: Some(sig.active_parameter as u32),
                }],
                active_signature: Some(0),
                active_parameter: Some(sig.active_parameter as u32),
            }))
        } else {
            Ok(None)
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(QSysLspBackend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
