use qsys_diagnostics::{DiagnosticKind, FileId, SourceFile};
use qsys_parser::ast::*;
use qsys_parser::parse_module;

fn parse_str(src: &str) -> Result<Module, Vec<qsys_diagnostics::Diagnostic>> {
    let file = SourceFile::new(FileId(1), "test.py", src);
    parse_module(file)
        .map(|parsed| parsed.module)
        .map_err(|diags| diags.items)
}

#[test]
fn test_parse_if_elif_else_exact_nesting() {
    let src = r#"
if a:
    x = 1
elif b:
    x = 2
else:
    x = 3
"#;
    let module = parse_str(src).expect("Should parse");
    assert_eq!(module.statements.len(), 1);

    match &module.statements[0].kind {
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert!(matches!(condition.kind, ExprKind::Ident(ref id) if id == "a"));
            assert_eq!(then_branch.len(), 1);
            // else_branch must contain nested If for `elif b`, NOT flattened body statements!
            assert_eq!(else_branch.len(), 1);
            match &else_branch[0].kind {
                StmtKind::If {
                    condition: elif_cond,
                    then_branch: elif_then,
                    else_branch: final_else,
                } => {
                    assert!(matches!(elif_cond.kind, ExprKind::Ident(ref id) if id == "b"));
                    assert_eq!(elif_then.len(), 1);
                    assert_eq!(final_else.len(), 1);
                }
                _ => panic!("Expected nested StmtKind::If for elif"),
            }
        }
        _ => panic!("Expected StmtKind::If"),
    }
}

#[test]
fn test_rejects_unsupported_operators_without_fallback() {
    // MatMult (@) must not silently turn into Add (+)
    let src = "res = a @ b";
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs[0].code(), "E0107");
    assert!(errs[0].message().contains("MatMult"));
}

#[test]
fn test_rejects_chained_comparison() {
    let src = "valid = a < b < c";
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs[0].code(), "E0108");
}

#[test]
fn test_rejects_chained_assignment() {
    let src = "a = b = 10";
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs[0].code(), "E0109");
}

#[test]
fn test_rejects_complex_number_without_panic() {
    let src = "val = 1 + 2j";
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs[0].code(), "E0111");
}

#[test]
fn test_rejects_tuple_target_in_for_loop() {
    let src = "for (a, b) in items: pass";
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs[0].code(), "E0110");
}

#[test]
fn test_multiple_imports_lowered_completely() {
    let src = "import math, sys";
    let module = parse_str(src).expect("Should parse");
    assert_eq!(module.statements.len(), 2);
    assert!(
        matches!(&module.statements[0].kind, StmtKind::Import { module, .. } if module == "math")
    );
    assert!(
        matches!(&module.statements[1].kind, StmtKind::Import { module, .. } if module == "sys")
    );
}

// Declarations (Classes, Methods, Decorators)

#[test]
fn test_parse_plugin_class_structure() {
    let src = r#"
class MatrixMixer(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0)
    Mute = Control.Boolean()

    @Gain.EventHandler
    def on_gain(self, ctl):
        pass
"#;
    let module = parse_str(src).expect("Plugin should parse cleanly");
    assert_eq!(module.statements.len(), 1);

    match &module.statements[0].kind {
        StmtKind::ClassDef(cls) => {
            assert_eq!(cls.name, "MatrixMixer");
            assert_eq!(cls.bases, vec!["QSysPlugin"]);
            assert_eq!(cls.body.len(), 3); // Gain, Mute, on_gain

            // Verify method and decorator
            match &cls.body[2].kind {
                StmtKind::FunctionDef(f) => {
                    assert_eq!(f.name, "on_gain");
                    assert_eq!(f.params.len(), 2);
                    assert_eq!(f.params[0].name, "self");
                    assert_eq!(f.params[1].name, "ctl");
                    assert_eq!(f.decorators.len(), 1);
                }
                _ => panic!("Expected FunctionDef for on_gain"),
            }
        }
        _ => panic!("Expected ClassDef"),
    }
}

#[test]
fn test_parse_functions_with_types_and_returns() {
    let src = r#"
def calculate_db(linear: float, offset: int) -> float:
    return linear * 20.0 + offset
"#;
    let module = parse_str(src).expect("Function with types should parse");
    match &module.statements[0].kind {
        StmtKind::FunctionDef(f) => {
            assert_eq!(f.name, "calculate_db");
            assert_eq!(f.params[0].type_annotation.as_deref(), Some("float"));
            assert_eq!(f.params[1].type_annotation.as_deref(), Some("int"));
            assert_eq!(f.return_type.as_deref(), Some("float"));
        }
        _ => panic!("Expected FunctionDef"),
    }
}

// Control Flow (If/Elif/Else, Loops)

#[test]
fn test_parse_if_elif_else_branches() {
    let src = r#"
if level > 0:
    mute = False
elif level == -100:
    mute = True
else:
    mute = False
"#;
    let module = parse_str(src).expect("If statement should parse");
    match &module.statements[0].kind {
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert!(matches!(
                condition.kind,
                ExprKind::Binary {
                    op: BinaryOp::Gt,
                    ..
                }
            ));
            assert_eq!(then_branch.len(), 1);
            // Elif + Else are gathered in else_branch
            assert!(!else_branch.is_empty());
        }
        _ => panic!("Expected If statement"),
    }
}

#[test]
fn test_parse_for_and_while_loops() {
    let src = r#"
for channel in channels:
    channel.Value = 0.0

while count > 0:
    count -= 1
"#;
    dbg!(&src[72..82]);
    let module = parse_str(src).expect("Loops should parse");
    assert_eq!(module.statements.len(), 2);

    match &module.statements[0].kind {
        StmtKind::For { target, .. } => assert_eq!(target, "channel"),
        _ => panic!("Expected For loop"),
    }

    match &module.statements[1].kind {
        StmtKind::While { .. } => {}
        _ => panic!("Expected While loop"),
    }
}

// Expressions & Operators

#[test]
fn test_parse_binary_and_unary_expressions() {
    let src = "x = (a + b) * -c";
    let module = parse_str(src).expect("Arithmetic should parse");

    match &module.statements[0].kind {
        StmtKind::Assign { target, value } => {
            assert!(matches!(target.kind, ExprKind::Ident(ref id) if id == "x"));
            match &value.kind {
                ExprKind::Binary { op, right, .. } => {
                    assert_eq!(*op, BinaryOp::Mul);
                    assert!(matches!(
                        right.kind,
                        ExprKind::Unary {
                            op: UnaryOp::Neg,
                            ..
                        }
                    ));
                }
                _ => panic!("Expected Binary expression"),
            }
        }
        _ => panic!("Expected Assign"),
    }
}

#[test]
fn test_parse_calls_with_positional_and_kwargs() {
    let src = "Controls.Gain.SetRamp(5.0, logarithmic=True)";
    let module = parse_str(src).expect("Call should parse");

    match &module.statements[0].kind {
        StmtKind::Expr(expr) => match &expr.kind {
            ExprKind::Call {
                callee,
                args,
                kwargs,
            } => {
                assert!(matches!(callee.kind, ExprKind::Attribute { .. }));
                assert_eq!(args.len(), 1);
                assert_eq!(kwargs.len(), 1);
                assert_eq!(kwargs[0].0, "logarithmic");
                assert!(matches!(kwargs[0].1.kind, ExprKind::Bool(true)));
            }
            _ => panic!("Expected Call"),
        },
        _ => panic!("Expected StmtKind::Expr"),
    }
}

#[test]
fn test_parse_subscripts_and_collections() {
    let src = r#"
items = [1, 2, "three"]
val = items[0]
"#;
    let module = parse_str(src).expect("Collections and subscripts should parse");
    assert_eq!(module.statements.len(), 2);

    match &module.statements[0].kind {
        StmtKind::Assign { value, .. } => match &value.kind {
            ExprKind::List(elements) => assert_eq!(elements.len(), 3),
            _ => panic!("Expected List"),
        },
        _ => panic!("Expected Assign"),
    }

    match &module.statements[1].kind {
        StmtKind::Assign { value, .. } => match &value.kind {
            ExprKind::Subscript { value, index } => {
                assert!(matches!(value.kind, ExprKind::Ident(ref id) if id == "items"));
                assert!(matches!(index.kind, ExprKind::Int(0)));
            }
            _ => panic!("Expected Subscript"),
        },
        _ => panic!("Expected Assign"),
    }
}

// Prohibited Python Features Rejection

#[test]
fn test_rejects_async_def() {
    let errs = parse_str("async def bad_worker(): pass").unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code(), "E0101");
    assert!(matches!(
        &errs[0].kind,
        DiagnosticKind::UnsupportedAsync { construct } if construct == "async def"
    ));
}

#[test]
fn test_rejects_async_for() {
    let errs = parse_str("async for item in queue: pass").unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code(), "E0101");
    assert!(matches!(
        &errs[0].kind,
        DiagnosticKind::UnsupportedAsync { construct } if construct == "async for"
    ));
}

#[test]
fn test_rejects_await_expression() {
    let errs = parse_str("val = await fetch()").unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code(), "E0101");
    assert!(matches!(
        &errs[0].kind,
        DiagnosticKind::UnsupportedAsync { construct } if construct == "await"
    ));
}

#[test]
fn test_rejects_yield_and_yield_from() {
    let errs_yield = parse_str("def gen(): yield 10").unwrap_err();
    assert_eq!(errs_yield[0].code(), "E0102");
    assert!(matches!(
        errs_yield[0].kind,
        DiagnosticKind::UnsupportedGenerator
    ));

    let errs_yield_from = parse_str("def gen(): yield from [1, 2]").unwrap_err();
    assert_eq!(errs_yield_from[0].code(), "E0103");
    assert!(matches!(
        errs_yield_from[0].kind,
        DiagnosticKind::UnsupportedYieldFrom
    ));
}

#[test]
fn test_rejects_match_statement() {
    let src = r#"
match val:
    case 1:
        pass
"#;
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code(), "E0104");
    assert!(matches!(
        errs[0].kind,
        DiagnosticKind::UnsupportedMatchStatement
    ));
}

// Syntax Error Recovery

#[test]
fn test_catches_unclosed_parenthesis() {
    let src = "def broken(:\n    pass";
    let errs = parse_str(src).unwrap_err();
    assert!(!errs.is_empty());
    assert_eq!(errs[0].code(), "E0001");
    assert!(errs[0].message().contains("Syntax error"));
}

#[test]
fn test_catches_indentation_errors() {
    let src = "def test():\npass";
    let errs = parse_str(src).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code(), "E0001");
}
