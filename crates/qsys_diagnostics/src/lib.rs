mod editor;
pub mod kind;
pub mod source;
pub mod span;

pub use editor::EditorMarker;
pub use kind::{DiagnosticKind, Severity};
pub use source::SourceFile;
pub use span::{FileId, Span};

use serde::{Deserialize, Serialize};

/// A compiler diagnostic bound to an exact source span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Span,
    pub custom_help: Option<String>,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, span: Span) -> Self {
        Self {
            kind,
            span,
            custom_help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.custom_help = Some(help.into());
        self
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    pub fn message(&self) -> String {
        self.kind.message()
    }

    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }

    pub fn help(&self) -> Option<String> {
        self.custom_help.clone().or_else(|| self.kind.help())
    }

    /// Converts into a representation ready for Monaco / CodeMirror editor squiggles.
    pub fn to_editor_marker(&self, file: &SourceFile) -> EditorMarker {
        let (start_line, start_col) = file.location(self.span.start);
        let (end_line, end_col) = file.location(self.span.end);

        EditorMarker {
            code: self.code().to_string(),
            message: self.message(),
            severity: self.severity(),
            start_line_number: start_line,
            start_column: start_col,
            end_line_number: end_line,
            end_column: end_col,
            help: self.help(),
        }
    }
}

/// Collection of diagnostics produced during compilation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn report(&mut self, kind: DiagnosticKind, span: Span) {
        self.add(Diagnostic::new(kind, span));
    }

    pub fn add(&mut self, diag: Diagnostic) {
        self.items.push(diag);
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity() == Severity::Error)
    }

    pub fn to_editor_markers(&self, file: &SourceFile) -> Vec<EditorMarker> {
        self.items
            .iter()
            .map(|d| d.to_editor_marker(file))
            .collect()
    }
}
