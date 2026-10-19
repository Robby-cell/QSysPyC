use qsys_diagnostics::{FileId, SourceFile};
use qsys_parser::parse_module;
use qsys_sema::analyze;

#[test]
fn test_catches_undefined_variable_typo_for_self() {
    let src = r#"
class MyPlugin(QSysPlugin):
    Gain = Control.Float()

    def foo(self):
        sel.bar = 42
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let diags = analyze(&parsed).unwrap_err();

    assert_eq!(diags.items.len(), 1);
    assert_eq!(diags.items[0].code(), "E0206");
    assert!(
        diags.items[0]
            .message()
            .contains("Undefined variable 'sel'; did you mean 'self'?")
    );
}

#[test]
fn test_catches_undeclared_attribute_on_self() {
    let src = r#"
class MyPlugin(QSysPlugin):
    MasterGain = Control.Float()

    def foo(self):
        self.MusterGain = 42.0
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let diags = analyze(&parsed).unwrap_err();

    assert_eq!(diags.items.len(), 1);
    assert_eq!(diags.items[0].code(), "E0207");
    assert!(diags.items[0].message().contains(
        "Class 'MyPlugin' has no control or property 'MusterGain'; did you mean 'MasterGain'?"
    ));
}
