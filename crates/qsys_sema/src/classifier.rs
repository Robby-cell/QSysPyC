use qsys_diagnostics::{DiagnosticKind, Diagnostics};
use qsys_parser::ast;

/// The classified compilation target of a QSysPy file.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetClassification<'a> {
    /// Full Q-SYS plugin (.qplug) with lifecycle methods
    Plugin { class: &'a ast::ClassDef },
    /// Text Controller / Script Component tied to existing pins
    TextController { class: &'a ast::ClassDef },
    /// Lightweight procedural script without a containing class
    ProceduralScript,
}

/// Inspects top-level statements to classify the program target.
pub fn classify_module<'a>(
    module: &'a ast::Module,
    diagnostics: &mut Diagnostics,
) -> Option<TargetClassification<'a>> {
    let mut plugin_class: Option<&'a ast::ClassDef> = None;
    let mut controller_class: Option<&'a ast::ClassDef> = None;

    for stmt in &module.statements {
        if let ast::StmtKind::ClassDef(cls) = &stmt.kind {
            if cls.bases.iter().any(|b| b == "QSysPlugin") {
                if let Some(prev) = plugin_class {
                    diagnostics.report(
                        DiagnosticKind::syntax_error(format!(
                            "Multiple QSysPlugin classes found ('{}' and '{}'). A file can only define one plugin.",
                            prev.name, cls.name
                        )),
                        cls.span,
                    );
                    return None;
                }
                plugin_class = Some(cls);
            } else if cls.bases.iter().any(|b| b == "TextController") {
                controller_class = Some(cls);
            }
        }
    }

    if let Some(class) = plugin_class {
        Some(TargetClassification::Plugin { class })
    } else if let Some(class) = controller_class {
        Some(TargetClassification::TextController { class })
    } else {
        Some(TargetClassification::ProceduralScript)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_parser::parse_module;

    fn parse_and_classify(src: &str) -> (Option<TargetClassification<'_>>, Diagnostics) {
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).expect("Should parse");
        // Leak parsed to satisfy lifetime in tests
        let parsed_ref = Box::leak(Box::new(parsed));
        let mut diags = Diagnostics::new();
        let target = classify_module(&parsed_ref.module, &mut diags);
        (target, diags)
    }

    #[test]
    fn test_classifies_plugin() {
        let src = "class Mixer(QSysPlugin): pass";
        let (target, diags) = parse_and_classify(src);
        assert!(!diags.has_errors());
        match target.unwrap() {
            TargetClassification::Plugin { class } => assert_eq!(class.name, "Mixer"),
            _ => panic!("Expected Plugin"),
        }
    }

    #[test]
    fn test_classifies_text_controller() {
        let src = "class Room(TextController): pass";
        let (target, diags) = parse_and_classify(src);
        assert!(!diags.has_errors());
        match target.unwrap() {
            TargetClassification::TextController { class } => assert_eq!(class.name, "Room"),
            _ => panic!("Expected TextController"),
        }
    }

    #[test]
    fn test_classifies_procedural_script() {
        let src = "print('Hello Q-SYS')";
        let (target, diags) = parse_and_classify(src);
        assert!(!diags.has_errors());
        assert_eq!(target.unwrap(), TargetClassification::ProceduralScript);
    }

    #[test]
    fn test_rejects_multiple_plugins() {
        let src = r#"
class PluginA(QSysPlugin): pass
class PluginB(QSysPlugin): pass
"#;
        let (target, diags) = parse_and_classify(src);
        assert!(target.is_none());
        assert!(diags.has_errors());
        assert!(
            diags.items[0]
                .message()
                .contains("Multiple QSysPlugin classes")
        );
    }
}
