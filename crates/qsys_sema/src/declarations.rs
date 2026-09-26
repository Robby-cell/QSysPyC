use rustc_hash::FxHashSet;

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::{ControlId, PropertyId};
use qsys_model::{
    ButtonType, ControlDef, ControlKind, ControlUnit, PinStyle, PluginInfo, PropertyDef, TextType,
};
use qsys_parser::ast;

use crate::symbols::ScopeStack;

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    List(Vec<ConstValue>),
}

impl ConstValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

pub fn eval_constant(expr: &ast::Expr) -> Option<ConstValue> {
    match &expr.kind {
        ast::ExprKind::Int(i) => Some(ConstValue::Int(*i)),
        ast::ExprKind::Float(f) => Some(ConstValue::Float(*f)),
        ast::ExprKind::String(s) => Some(ConstValue::String(s.clone())),
        ast::ExprKind::Bool(b) => Some(ConstValue::Bool(*b)),
        ast::ExprKind::List(items) => {
            let mut list = Vec::new();
            for item in items {
                list.push(eval_constant(item)?);
            }
            Some(ConstValue::List(list))
        }
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Neg,
            operand,
        } => match eval_constant(operand)? {
            ConstValue::Int(i) => Some(ConstValue::Int(-i)),
            ConstValue::Float(f) => Some(ConstValue::Float(-f)),
            _ => None,
        },
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Pos,
            operand,
        } => eval_constant(operand),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredControl {
    pub id: ControlId,
    pub ident: String,
    pub def: ControlDef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredProperty {
    pub id: PropertyId,
    pub ident: String,
    pub def: PropertyDef,
}

#[derive(Debug, Clone, Default)]
pub struct ExtractedDeclarations {
    pub controls: Vec<DeclaredControl>,
    pub properties: Vec<DeclaredProperty>,
    pub plugin_info: PluginInfo,
    pub remaining_statements: Vec<ast::Stmt>,
}

pub fn extract_class_declarations(
    class: &ast::ClassDef,
    scope_stack: &mut ScopeStack,
    diagnostics: &mut Diagnostics,
) -> ExtractedDeclarations {
    let mut declarations = ExtractedDeclarations::default();
    let mut seen_controls = FxHashSet::default();
    let mut seen_properties = FxHashSet::default();

    declarations.plugin_info.name = class.name.clone();

    for stmt in &class.body {
        match &stmt.kind {
            ast::StmtKind::Assign { target, value } => {
                let var_name = match &target.kind {
                    ast::ExprKind::Ident(name) => name.clone(),
                    _ => {
                        declarations.remaining_statements.push(stmt.clone());
                        continue;
                    }
                };

                if let Some(control_def) =
                    try_parse_control(&var_name, value, stmt.span, diagnostics)
                {
                    if !seen_controls.insert(var_name.clone()) {
                        diagnostics.report(
                            DiagnosticKind::DuplicateControlDeclaration {
                                name: var_name.clone(),
                            },
                            stmt.span,
                        );
                    } else {
                        let id = scope_stack.alloc_control_id();
                        declarations.controls.push(DeclaredControl {
                            id,
                            ident: var_name.clone(),
                            def: control_def,
                        });
                    }
                    continue;
                }

                if let Some(property_def) =
                    try_parse_property(&var_name, value, stmt.span, diagnostics)
                {
                    if !seen_properties.insert(var_name.clone()) {
                        diagnostics.report(
                            DiagnosticKind::syntax_error(format!(
                                "Property '{var_name}' is declared multiple times"
                            )),
                            stmt.span,
                        );
                    } else {
                        let id = scope_stack.alloc_property_id();
                        declarations.properties.push(DeclaredProperty {
                            id,
                            ident: var_name.clone(),
                            def: property_def,
                        });
                    }
                    continue;
                }

                if try_parse_metadata(&var_name, value, &mut declarations.plugin_info) {
                    continue;
                }

                declarations.remaining_statements.push(stmt.clone());
            }

            _ => declarations.remaining_statements.push(stmt.clone()),
        }
    }

    declarations
}

fn try_parse_control(
    var_name: &str,
    expr: &ast::Expr,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Option<ControlDef> {
    let (kind_str, args, kwargs) = match &expr.kind {
        ast::ExprKind::Call {
            callee,
            args,
            kwargs,
        } => match &callee.kind {
            ast::ExprKind::Attribute { value, attr } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Control" => (attr.as_str(), args, kwargs),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };

    // 1. Check keyword: name="..."
    // 2. Check positional: Control.<Type>("...")
    // 3. Fall back to built-in Q-SYS standard names ("IPAddress", "MACAddress")
    // 4. Fall back to variable identifier name (e.g. `Gain = ...` -> "Gain")
    let control_name = match kwargs.iter().find(|(k, _)| k == "name") {
        Some((_, val)) => match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
            Some(s) => s,
            None => {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: var_name.to_string(),
                        message: "Control 'name' argument must be a string".to_string(),
                    },
                    val.span,
                );
                return None;
            }
        },
        None => match args.first() {
            Some(val) => match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                Some(s) => s,
                None => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: var_name.to_string(),
                            message: "Control positional name must be a string".to_string(),
                        },
                        val.span,
                    );
                    return None;
                }
            },
            None => match kind_str {
                "IPAddress" | "IpAddress" => "IPAddress".to_string(),
                "MACAddress" | "MacAddress" => "MACAddress".to_string(),
                _ => var_name.to_string(),
            },
        },
    };

    let count = match kwargs.iter().find(|(k, _)| k == "count") {
        Some((_, val)) => match eval_constant(val).and_then(|c| c.as_i64()) {
            Some(c) if c > 0 => c as usize,
            _ => {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.clone(),
                        message: "'count' must be a positive integer".to_string(),
                    },
                    val.span,
                );
                return None;
            }
        },
        None => 1,
    };

    let pin_style = match kwargs.iter().find(|(k, _)| k == "pins") {
        Some((_, val)) => {
            match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                Some(ref s) if s == "None" => PinStyle::None,
                Some(ref s) if s == "Input" => PinStyle::Input,
                Some(ref s) if s == "Output" => PinStyle::Output,
                Some(ref s) if s == "Both" => PinStyle::Both,
                _ => {
                    diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.clone(),
                        message: "Invalid 'pins' value. Expected 'None', 'Input', 'Output', or 'Both'".to_string(),
                    },
                    val.span,
                );
                    return None;
                }
            }
        }
        None => PinStyle::Both,
    };

    let kind = match kind_str {
        "Float" | "Knob" | "Fader" => {
            let min = match kwargs.iter().find(|(k, _)| k == "min") {
                Some((_, val)) => match eval_constant(val).and_then(|c| c.as_f64()) {
                    Some(m) => m,
                    None => {
                        diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "'min' must be a numeric literal".to_string(),
                            },
                            val.span,
                        );
                        return None;
                    }
                },
                None => 0.0,
            };

            let max = match kwargs.iter().find(|(k, _)| k == "max") {
                Some((_, val)) => match eval_constant(val).and_then(|c| c.as_f64()) {
                    Some(m) => m,
                    None => {
                        diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "'max' must be a numeric literal".to_string(),
                            },
                            val.span,
                        );
                        return None;
                    }
                },
                None => 1.0,
            };

            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => match eval_constant(val).and_then(|c| c.as_f64()) {
                    Some(d) => d,
                    None => {
                        diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "'default' must be a numeric literal".to_string(),
                            },
                            val.span,
                        );
                        return None;
                    }
                },
                None => min,
            };

            if min > max {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.clone(),
                        message: format!("'min' ({min}) cannot be greater than 'max' ({max})"),
                    },
                    span,
                );
                return None;
            }

            if default < min || default > max {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.clone(),
                        message: format!("'default' ({default}) is outside range [{min}, {max}]"),
                    },
                    span,
                );
                return None;
            }

            if kind_str == "Fader" {
                ControlKind::Fader {
                    unit: ControlUnit::Float,
                    min,
                    max,
                    default,
                }
            } else {
                ControlKind::Knob {
                    unit: ControlUnit::Float,
                    min,
                    max,
                    default,
                }
            }
        }

        "Boolean" | "Button" => {
            let btn_type = match kwargs.iter().find(|(k, _)| k == "type") {
                Some((_, val)) => {
                    match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                        Some(ref s) if s == "Momentary" => ButtonType::Momentary,
                        Some(ref s) if s == "Trigger" => ButtonType::Trigger,
                        Some(ref s) if s == "Toggle" => ButtonType::Toggle,
                        _ => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Invalid button 'type'. Expected 'Momentary', 'Toggle', or 'Trigger'".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => ButtonType::Momentary,
            };
            ControlKind::Button {
                button_type: btn_type,
            }
        }

        "Text" => {
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => {
                    match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                        Some(s) => s,
                        None => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Text 'default' must be a string".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => String::new(),
            };
            ControlKind::Text {
                text_type: TextType::Text,
                default,
            }
        }

        // Built-in Q-SYS Reserved Control: IPAddress
        "IPAddress" | "IpAddress" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_str().map(String::from))
                .unwrap_or_else(|| "255.255.255.255".to_string());

            ControlKind::Text {
                text_type: TextType::Text,
                default,
            }
        }

        // Built-in Q-SYS Reserved Control: MACAddress
        "MACAddress" | "MacAddress" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_str().map(String::from))
                .unwrap_or_else(|| "00:00:00:00:00:00".to_string());

            ControlKind::Text {
                text_type: TextType::Text,
                default,
            }
        }

        _ => {
            diagnostics.report(
                DiagnosticKind::syntax_error(format!("Unknown Control type 'Control.{kind_str}'")),
                span,
            );
            return None;
        }
    };

    let mut def = ControlDef::new(control_name, kind).with_pin_style(pin_style);
    if count > 1 {
        def = def.with_count(count);
    }
    Some(def)
}

fn try_parse_property(
    var_name: &str,
    expr: &ast::Expr,
    span: Span,
    diagnostics: &mut Diagnostics,
) -> Option<PropertyDef> {
    let (kind_str, args, kwargs) = match &expr.kind {
        ast::ExprKind::Call {
            callee,
            args,
            kwargs,
        } => match &callee.kind {
            ast::ExprKind::Attribute { value, attr } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Property" => (attr.as_str(), args, kwargs),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };

    let prop_name = kwargs
        .iter()
        .find(|(k, _)| k == "name")
        .and_then(|(_, v)| eval_constant(v))
        .and_then(|c| c.as_str().map(String::from))
        .or_else(|| {
            args.first()
                .and_then(eval_constant)
                .and_then(|c| c.as_str().map(String::from))
        })
        .unwrap_or_else(|| var_name.to_string());

    let description = kwargs
        .iter()
        .find(|(k, _)| k == "description")
        .and_then(|(_, v)| eval_constant(v))
        .and_then(|c| c.as_str().map(String::from));

    let prop = match kind_str {
        "Integer" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_i64())
                .unwrap_or(0);
            let min = kwargs
                .iter()
                .find(|(k, _)| k == "min")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_i64());
            let max = kwargs
                .iter()
                .find(|(k, _)| k == "max")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_i64());
            PropertyDef::integer(prop_name, default, min, max)
        }

        "Double" | "Float" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_f64())
                .unwrap_or(0.0);
            let min = kwargs
                .iter()
                .find(|(k, _)| k == "min")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_f64());
            let max = kwargs
                .iter()
                .find(|(k, _)| k == "max")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_f64());
            PropertyDef::double(prop_name, default, min, max)
        }

        "String" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_str().map(String::from))
                .unwrap_or_default();
            PropertyDef::string(prop_name, default)
        }

        "Enum" => {
            let choices = match kwargs.iter().find(|(k, _)| k == "choices") {
                Some((_, val)) => match eval_constant(val) {
                    Some(ConstValue::List(items)) => {
                        let mut strings = Vec::new();
                        for item in items {
                            if let Some(s) = item.as_str() {
                                strings.push(s.to_string());
                            } else {
                                diagnostics.report(
                                    DiagnosticKind::InvalidDeclarationArgument {
                                        name: prop_name.clone(),
                                        message: "All choices in 'choices' must be string literals"
                                            .to_string(),
                                    },
                                    val.span,
                                );
                                return None;
                            }
                        }
                        if strings.is_empty() {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: prop_name.clone(),
                                    message: "'choices' list cannot be empty".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                        strings
                    }
                    _ => {
                        diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: prop_name.clone(),
                                message: "'choices' must be a list of strings".to_string(),
                            },
                            val.span,
                        );
                        return None;
                    }
                },
                None => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: prop_name.clone(),
                            message: "Property.Enum requires 'choices'".to_string(),
                        },
                        span,
                    );
                    return None;
                }
            };

            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => {
                    match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                        Some(d) => {
                            if !choices.contains(&d) {
                                diagnostics.report(
                                    DiagnosticKind::InvalidDeclarationArgument {
                                        name: prop_name.clone(),
                                        message: format!(
                                            "'default' choice ('{d}') must be one of {choices:?}"
                                        ),
                                    },
                                    val.span,
                                );
                                return None;
                            }
                            d
                        }
                        None => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: prop_name.clone(),
                                    message: "'default' choice must be a string".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => choices.first().cloned().unwrap(),
            };

            PropertyDef::enumeration(prop_name, choices, default)
        }

        _ => {
            diagnostics.report(
                DiagnosticKind::syntax_error(format!(
                    "Unknown Property type 'Property.{kind_str}'"
                )),
                span,
            );
            return None;
        }
    };

    Some(match description {
        Some(d) => prop.with_description(d),
        None => prop,
    })
}

fn try_parse_metadata(var_name: &str, value: &ast::Expr, info: &mut PluginInfo) -> bool {
    let const_val = eval_constant(value);
    match var_name {
        "name" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.name = s;
                return true;
            }
        }
        "version" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.version = s;
                return true;
            }
        }
        "author" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.author = s;
                return true;
            }
        }
        "id" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.id = s;
                return true;
            }
        }
        "description" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.description = Some(s);
                return true;
            }
        }
        "show_debug" => {
            if let Some(b) = const_val.and_then(|c| c.as_bool()) {
                info.show_debug = b;
                return true;
            }
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_parser::parse_module;

    fn parse_class_body(src: &str) -> (ExtractedDeclarations, Diagnostics) {
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).expect("Should parse");
        let cls = match &parsed.module.statements[0].kind {
            ast::StmtKind::ClassDef(c) => c,
            _ => panic!("Expected ClassDef"),
        };
        let mut diags = Diagnostics::new();
        let mut scope_stack = ScopeStack::new();
        let decls = extract_class_declarations(cls, &mut scope_stack, &mut diags);
        (decls, diags)
    }

    #[test]
    fn test_extract_controls_with_negative_ranges() {
        let src = r#"
class Mixer(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)
    Mute = Control.Boolean()
"#;
        let (decls, diags) = parse_class_body(src);
        assert!(!diags.has_errors());
        assert_eq!(decls.controls.len(), 2);

        let gain = &decls.controls[0];
        assert_eq!(gain.ident, "Gain");
        assert_eq!(gain.def.name, "MasterGain");
        match &gain.def.kind {
            ControlKind::Knob {
                min, max, default, ..
            } => {
                assert_eq!(*min, -100.0);
                assert_eq!(*max, 20.0);
                assert_eq!(*default, 0.0);
            }
            _ => panic!("Expected Knob/Float"),
        }

        let mute = &decls.controls[1];
        assert_eq!(mute.ident, "Mute");
        assert_eq!(mute.def.name, "Mute");
        assert!(matches!(mute.def.kind, ControlKind::Button { .. }));
    }
}
