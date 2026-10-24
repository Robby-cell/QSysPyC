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

    let statements = group_bundled_modules(&file.src, lowerer.lower_suite(&ruff_suite));

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
    let statements = group_bundled_modules(&file.src, lowerer.lower_suite(&ruff_suite));
    let module = ast::Module {
        statements,
        span: Span::new(0, file.src.len()),
    };

    (ParsedModule { file, module }, diagnostics)
}

fn module_name_from_path(raw_path: &str) -> String {
    let normalized = raw_path.trim().replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    if let Some(&last) = parts.last() {
        let stem = last.strip_suffix(".py").unwrap_or(last);
        if stem == "__init__" && parts.len() >= 2 {
            return parts[parts.len() - 2].to_string();
        }
        return stem.to_string();
    }
    "module".to_string()
}

/// Groups statements preceded by `# --- Bundled module: <path> ---` comments into
/// `StmtKind::ModuleDef` nodes so imported modules get isolated namespaces.
fn group_bundled_modules(src: &str, statements: Vec<Stmt>) -> Vec<Stmt> {
    const BUNDLED_PREFIX: &str = "# --- Bundled module:";
    const END_BUNDLED_MARKER: &str = "# --- End bundled modules ---";

    if !src.contains(BUNDLED_PREFIX) {
        return statements;
    }

    struct BundledHeader {
        start_byte: usize,
        name: String,
        path: String,
    }

    let mut headers: Vec<BundledHeader> = Vec::new();
    let mut explicit_end_byte: Option<usize> = None;

    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if let Some(after) = trimmed.strip_prefix(BUNDLED_PREFIX) {
            let path = after.trim().trim_end_matches("---").trim().to_string();
            let name = module_name_from_path(&path);
            headers.push(BundledHeader {
                start_byte: offset,
                name,
                path,
            });
        } else if trimmed.starts_with(END_BUNDLED_MARKER) {
            explicit_end_byte = Some(offset);
        }
        offset += line.len();
    }

    if headers.is_empty() {
        return statements;
    }

    let last_header_byte = headers.last().unwrap().start_byte;
    let effective_end_byte = explicit_end_byte.unwrap_or_else(|| {
        // Fallback if no explicit end marker: stop at the first import of a bundled module
        // or the main QSysPlugin / TextController class after the last header.
        for stmt in &statements {
            if stmt.span.start <= last_header_byte {
                continue;
            }
            match &stmt.kind {
                StmtKind::Import { module, .. } | StmtKind::ImportFrom { module, .. } => {
                    let mod_tail = module
                        .trim_start_matches('.')
                        .rsplit('.')
                        .next()
                        .unwrap_or(module.as_str());
                    if headers.iter().any(|h| h.name == mod_tail) {
                        return stmt.span.start;
                    }
                }
                StmtKind::ClassDef(cls)
                    if cls
                        .bases
                        .iter()
                        .any(|b| b == "QSysPlugin" || b == "TextController") =>
                {
                    return stmt.span.start;
                }
                _ => {}
            }
        }
        src.len()
    });

    let first_header_byte = headers[0].start_byte;
    let mut pre_stmts = Vec::new();
    let mut mod_bodies: Vec<Vec<Stmt>> = vec![Vec::new(); headers.len()];
    let mut post_stmts = Vec::new();

    for stmt in statements {
        if stmt.span.start < first_header_byte {
            pre_stmts.push(stmt);
        } else if stmt.span.start >= effective_end_byte {
            post_stmts.push(stmt);
        } else {
            // Find which bundled header owns this statement
            let mut owner = 0;
            for (i, h) in headers.iter().enumerate() {
                if stmt.span.start >= h.start_byte {
                    owner = i;
                } else {
                    break;
                }
            }
            mod_bodies[owner].push(stmt);
        }
    }

    let mut out = pre_stmts;
    for (i, h) in headers.into_iter().enumerate() {
        let end_byte = mod_bodies[i]
            .last()
            .map(|s| s.span.end)
            .unwrap_or(h.start_byte);
        let span = Span::new(h.start_byte, end_byte);
        out.push(Stmt::new(
            StmtKind::ModuleDef {
                name: h.name,
                path: h.path,
                body: std::mem::take(&mut mod_bodies[i]),
            },
            span,
        ));
    }
    out.extend(post_stmts);
    out
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
