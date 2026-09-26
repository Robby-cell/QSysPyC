use qsys_diagnostics::{FileId, SourceFile};
use qsys_ir::*;
use qsys_parser::parse_module;
use qsys_sema::analyze;

#[test]
fn test_self_control_attribute_resolves_to_control_id() {
    let src = r#"
class Mixer(QSysPlugin):
    Master = Control.Float(name="MasterGain")

    def reset_gain(self):
        self.Master.Value = 0.0
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let model = analyze(&parsed).expect("Should analyze cleanly");
    let ir = model.into_ir();

    match ir {
        IRProgram::Plugin(p) => {
            assert_eq!(p.controls.len(), 1);
            let expected_ctl_id = p.controls[0].id;

            // In reset_gain(): self.Master.Value = 0.0
            let method = &p.methods[0];
            match &method.body[0] {
                IRStmt::Assign { target, .. } => match target {
                    IRExpr::Attribute { object, attr } => {
                        assert_eq!(attr, "Value");
                        // object is self.Master, which now lowers to IRExpr::Control(expected_ctl_id)
                        assert_eq!(**object, IRExpr::Control(expected_ctl_id));
                    }
                    _ => panic!("Expected Attribute expression on target"),
                },
                _ => panic!("Expected Assign statement"),
            }
        }
        _ => panic!("Expected Plugin IR"),
    }
}

#[test]
fn test_local_variables_resolve_to_stable_symbol_ids() {
    let src = r#"
def compute():
    x = 10
    y = x + 5
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let model = analyze(&parsed).unwrap();
    let ir = model.into_ir();

    match ir {
        IRProgram::Script(s) => {
            let body = if !s.methods.is_empty() {
                &s.methods[0].body
            } else {
                &s.body
            };

            // First statement: x = 10
            let x_id = match &body[0] {
                IRStmt::Assign { target, .. } => match target {
                    IRExpr::Local(id) => *id,
                    _ => panic!("Expected Local for x target"),
                },
                _ => panic!("Expected Assign"),
            };

            // Second statement: y = x + 5
            match &body[1] {
                IRStmt::Assign { value, .. } => match value {
                    IRExpr::Binary { left, .. } => {
                        assert_eq!(**left, IRExpr::Local(x_id));
                    }
                    _ => panic!("Expected Binary expression"),
                },
                _ => panic!("Expected Assign"),
            }
        }
        _ => panic!("Expected Script IR"),
    }
}

#[test]
fn test_fuzzy_member_diagnosis_on_control_handle() {
    let src = r#"
class Mixer(QSysPlugin):
    Gain = Control.Float()

    @Gain.EventHandler
    def on_gain(self, ctl):
        x = ctl.Valu
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let diags = analyze(&parsed).unwrap_err();

    assert_eq!(diags.items.len(), 1);
    assert_eq!(diags.items[0].code(), "E0201");
    assert!(
        diags.items[0]
            .message()
            .contains("Control 'ctl' has no member 'Valu'; did you mean 'Value'?")
    );
}
