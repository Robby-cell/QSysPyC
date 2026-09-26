use qsys_diagnostics::{FileId, SourceFile};
use qsys_ir::*;
use qsys_parser::parse_module;
use qsys_sema::analyze;

#[test]
fn test_string_concatenation_typed_to_ir() {
    let src = r#"
def greet(name: str):
    msg = "Hello, " + name
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let model = analyze(&parsed).unwrap();
    let ir = model.into_ir();

    match ir {
        IRProgram::Script(s) => {
            assert_eq!(s.methods.len(), 1);
            assert_eq!(s.methods[0].name, "greet");
            match &s.methods[0].body[0] {
                IRStmt::Assign { value, .. } => match value {
                    IRExpr::Binary { op, .. } => {
                        assert_eq!(*op, IRBinaryOp::StringConcat);
                    }
                    _ => panic!("Expected Binary expression"),
                },
                _ => panic!("Expected Assign"),
            }
        }
        _ => panic!("Expected Script"),
    }
}

#[test]
fn test_call_kwargs_preserved_in_ir() {
    let src = r#"
def configure(client):
    client.setup(rate=9600, active=True)
"#;
    let file = SourceFile::new(FileId(1), "test.py", src);
    let parsed = parse_module(file).unwrap();
    let model = analyze(&parsed).unwrap();
    let ir = model.into_ir();

    match ir {
        IRProgram::Script(s) => {
            assert_eq!(s.methods.len(), 1);
            assert_eq!(s.methods[0].name, "configure");
            match &s.methods[0].body[0] {
                IRStmt::Expr { expr, .. } => match expr {
                    IRExpr::Call { kwargs, .. } => {
                        assert_eq!(kwargs.len(), 2);
                        assert_eq!(kwargs[0].0, "rate");
                        assert_eq!(kwargs[1].0, "active");
                    }
                    _ => panic!("Expected Call expression"),
                },
                _ => panic!("Expected Expr statement"),
            }
        }
        _ => panic!("Expected Script"),
    }
}

#[test]
fn test_strict_escape_hatch_rejections() {
    let bad1 = "def run(): qsys.lua()";
    let file1 = SourceFile::new(FileId(1), "test.py", bad1);
    assert!(analyze(&parse_module(file1).unwrap()).is_err());

    let bad2 = "def run(): qsys.lua('one', 'two')";
    let file2 = SourceFile::new(FileId(2), "test.py", bad2);
    assert!(analyze(&parse_module(file2).unwrap()).is_err());

    let bad3 = "def run(): qsys.lua(code='print()')";
    let file3 = SourceFile::new(FileId(3), "test.py", bad3);
    assert!(analyze(&parse_module(file3).unwrap()).is_err());
}

#[test]
fn test_strict_declaration_bounds_rejection() {
    let bad_range = r#"
class BadMixer(QSysPlugin):
    Gain = Control.Float(min=10.0, max=0.0)
"#;
    let file1 = SourceFile::new(FileId(1), "test.py", bad_range);
    let diags1 = analyze(&parse_module(file1).unwrap()).unwrap_err();
    assert_eq!(diags1.items[0].code(), "E0204");
    assert!(diags1.items[0].message().contains("cannot be greater than"));

    let bad_btn = r#"
class BadButton(QSysPlugin):
    Mute = Control.Button(type="InvalidType")
"#;
    let file2 = SourceFile::new(FileId(2), "test.py", bad_btn);
    let diags2 = analyze(&parse_module(file2).unwrap()).unwrap_err();
    assert_eq!(diags2.items[0].code(), "E0204");
    assert!(diags2.items[0].message().contains("Invalid button 'type'"));
}
