pub mod ast;
mod lower;

use std::sync::Arc;

use ruff_python_parser::{Mode, ParseOptions, parse_unchecked};
use ruff_text_size::Ranged;

use qsys_diagnostics::{DiagnosticKind, Diagnostics, SourceFile, Span};

pub use crate::ast::*;
use crate::lower::{Lowerer, text_range_to_span};

/// A validated, lowered QSysPy module.
#[derive(Debug, Clone)]
pub struct ParsedModule {
    pub file: Arc<SourceFile>,
    pub module: ast::Module,
}

/// Normalizes `Control.[Type](...)` shorthand into valid Python subscript syntax `Control [Type](...)`
/// while preserving exact byte offsets and spans.
fn normalize_source(src: &str) -> std::borrow::Cow<'_, str> {
    if src.contains("Control.[") {
        std::borrow::Cow::Owned(src.replace("Control.[", "Control ["))
    } else {
        std::borrow::Cow::Borrowed(src)
    }
}

/// Parses Python source text into our clean, curated QSysPy AST.
pub fn parse_module(file: Arc<SourceFile>) -> Result<ParsedModule, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let normalized = normalize_source(&file.src);

    // Parse using Ruff
    let parsed = parse_unchecked(&normalized, ParseOptions::from(Mode::Module));

    // Collect syntax errors
    for err in parsed.errors() {
        let span = text_range_to_span(err.range());
        diagnostics.report(
            DiagnosticKind::SyntaxError {
                message: err.error.to_string(),
            },
            span,
        );
    }

    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    let ruff_suite = match parsed.into_syntax() {
        ruff_python_ast::Mod::Module(m) => m.body,
        _ => {
            diagnostics.report(
                DiagnosticKind::ExpectedModuleBody,
                Span::new(0, file.src.len()),
            );
            return Err(diagnostics);
        }
    };

    // Lower Ruff AST into QSysPy AST while validating language subset
    let mut lowerer = Lowerer {
        diagnostics: &mut diagnostics,
    };

    let statements = lowerer.lower_suite(&ruff_suite);

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        let module = ast::Module {
            statements,
            span: Span::new(0, file.src.len()),
        };
        Ok(ParsedModule { file, module })
    }
}

/// Fault-tolerant parser entry point for IDE / LSP features.
/// Always lowers the recovered Ruff AST even when syntax errors are present.
pub fn parse_module_lossy(file: Arc<SourceFile>) -> (ParsedModule, Diagnostics) {
    let mut diagnostics = Diagnostics::new();
    let normalized = normalize_source(&file.src);
    let parsed = parse_unchecked(&normalized, ParseOptions::from(Mode::Module));

    for err in parsed.errors() {
        let span = text_range_to_span(err.range());
        diagnostics.report(
            DiagnosticKind::SyntaxError {
                message: err.error.to_string(),
            },
            span,
        );
    }

    let ruff_suite = match parsed.into_syntax() {
        ruff_python_ast::Mod::Module(m) => m.body,
        _ => Default::default(),
    };

    let mut lowerer = Lowerer {
        diagnostics: &mut diagnostics,
    };
    let statements = lowerer.lower_suite(&ruff_suite);
    let module = ast::Module {
        statements,
        span: Span::new(0, file.src.len()),
    };

    (ParsedModule { file, module }, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsys_diagnostics::{DiagnosticKind, FileId};

    #[test]
    fn test_parse_valid_plugin() {
        let src = r#"
class Mixer(QSysPlugin):
    Gain = Control.Float()

    @Gain.EventHandler
    def on_gain(self, ctl):
        print(ctl.Value)
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).expect("Should parse cleanly");

        assert_eq!(parsed.module.statements.len(), 1);
        match &parsed.module.statements[0].kind {
            StmtKind::ClassDef(cls) => {
                assert_eq!(cls.name, "Mixer");
                assert_eq!(cls.body.len(), 2); // Gain assignment + on_gain method
            }
            _ => panic!("Expected ClassDef"),
        }
    }

    #[test]
    fn test_pattern_match_unsupported_async() {
        let src = "async def bad(): pass";
        let file = SourceFile::new(FileId(1), "async.py", src);
        let errs = parse_module(file).unwrap_err();

        assert!(matches!(
            &errs.items[0].kind,
            DiagnosticKind::UnsupportedAsync { construct } if construct == "async def"
        ));
        assert_eq!(errs.items[0].code(), "E0101");
    }

    #[test]
    fn test_pattern_match_unsupported_generator() {
        let src = "def gen(): yield 1";
        let file = SourceFile::new(FileId(1), "yield.py", src);
        let errs = parse_module(file).unwrap_err();

        assert!(matches!(
            errs.items[0].kind,
            DiagnosticKind::UnsupportedGenerator
        ));
        assert_eq!(errs.items[0].code(), "E0102");
    }
}
