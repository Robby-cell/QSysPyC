use rustc_hash::{FxHashMap, FxHashSet};

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::{ControlId, PropertyId};
use qsys_model::{
    ButtonType, ControlDef, ControlKind, ControlUnit, GenericControlSpec, PageLayout, PinStyle,
    PluginInfo, PropertyDef, TextType,
};
use qsys_parser::ast;

use crate::symbols::ScopeStack;
use crate::ui_layout::{
    UiResolveEnv, collect_inline_ui_pages_from_stmts, extract_declarative_layout_from_method,
    is_ui_layout_expr, parse_ui_layout_expr_with_env,
};

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

pub fn eval_constant_with_env(
    expr: &ast::Expr,
    env: &rustc_hash::FxHashMap<String, ConstValue>,
) -> Option<ConstValue> {
    match &expr.kind {
        ast::ExprKind::Int(i) => Some(ConstValue::Int(*i)),
        ast::ExprKind::Float(f) => Some(ConstValue::Float(*f)),
        ast::ExprKind::String(s) => Some(ConstValue::String(s.clone())),
        ast::ExprKind::Bool(b) => Some(ConstValue::Bool(*b)),
        ast::ExprKind::Ident(name) => env.get(name).cloned(),
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Neg,
            operand,
        } => match eval_constant_with_env(operand, env)? {
            ConstValue::Int(i) => Some(ConstValue::Int(-i)),
            ConstValue::Float(f) => Some(ConstValue::Float(-f)),
            _ => None,
        },
        _ => None,
    }
}

pub fn eval_constant(expr: &ast::Expr) -> Option<ConstValue> {
    match &expr.kind {
        ast::ExprKind::Int(i) => Some(ConstValue::Int(*i)),
        ast::ExprKind::Float(f) => Some(ConstValue::Float(*f)),
        ast::ExprKind::String(s) => Some(ConstValue::String(s.clone())),
        ast::ExprKind::Bool(b) => Some(ConstValue::Bool(*b)),
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
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
        ast::ExprKind::Binary { left, op, right } => {
            let l = eval_constant(left)?;
            let r = eval_constant(right)?;
            match (l, op, r) {
                (ConstValue::Int(a), ast::BinaryOp::Add, ConstValue::Int(b)) => {
                    Some(ConstValue::Int(a + b))
                }
                (ConstValue::Int(a), ast::BinaryOp::Sub, ConstValue::Int(b)) => {
                    Some(ConstValue::Int(a - b))
                }
                (ConstValue::Int(a), ast::BinaryOp::Mul, ConstValue::Int(b)) => {
                    Some(ConstValue::Int(a * b))
                }
                (ConstValue::Float(a), ast::BinaryOp::Add, ConstValue::Float(b)) => {
                    Some(ConstValue::Float(a + b))
                }
                (ConstValue::Float(a), ast::BinaryOp::Sub, ConstValue::Float(b)) => {
                    Some(ConstValue::Float(a - b))
                }
                (ConstValue::Float(a), ast::BinaryOp::Mul, ConstValue::Float(b)) => {
                    Some(ConstValue::Float(a * b))
                }
                (ConstValue::String(a), ast::BinaryOp::Add, ConstValue::String(b)) => {
                    Some(ConstValue::String(format!("{a}{b}")))
                }
                _ => None,
            }
        }
        ast::ExprKind::Call { callee, args, .. } => {
            if let ast::ExprKind::Ident(fn_name) = &callee.kind
                && fn_name == "str"
                && args.len() == 1
            {
                match eval_constant(&args[0])? {
                    ConstValue::Int(i) => Some(ConstValue::String(i.to_string())),
                    ConstValue::Float(f) => Some(ConstValue::String(f.to_string())),
                    ConstValue::String(s) => Some(ConstValue::String(s)),
                    ConstValue::Bool(b) => Some(ConstValue::String(b.to_string())),
                    ConstValue::List(_) => None,
                }
            } else {
                None
            }
        }
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
    pub pages: Vec<PageLayout>,
    pub ui_control_resolutions: Vec<(Span, ControlId)>,
    pub control_property_resolutions: Vec<(Span, PropertyId)>,
    pub remaining_statements: Vec<ast::Stmt>,
}

pub fn extract_class_declarations(
    class: &ast::ClassDef,
    scope_stack: &mut ScopeStack,
    diagnostics: &mut Diagnostics,
) -> ExtractedDeclarations {
    extract_class_declarations_with_module(class, &[], scope_stack, diagnostics)
}

pub fn extract_class_declarations_with_module(
    class: &ast::ClassDef,
    module_statements: &[ast::Stmt],
    scope_stack: &mut ScopeStack,
    diagnostics: &mut Diagnostics,
) -> ExtractedDeclarations {
    let mut declarations = ExtractedDeclarations::default();
    let mut seen_controls = FxHashSet::default();
    let mut seen_properties = FxHashSet::default();
    let mut pending_controls: Vec<(String, ast::Expr, Span)> = Vec::new();
    let mut pending_ui_exprs: Vec<ast::Expr> = Vec::new();

    declarations.plugin_info.name = class.name.clone();

    for stmt in &class.body {
        match &stmt.kind {
            ast::StmtKind::Assign { target, value, .. } => {
                let var_name = match &target.kind {
                    ast::ExprKind::Ident(name) => name.clone(),
                    _ => {
                        declarations.remaining_statements.push(stmt.clone());
                        continue;
                    }
                };

                if var_name == "layout" || var_name == "pages" || is_ui_layout_expr(value) {
                    pending_ui_exprs.push(value.clone());
                    continue;
                }

                if is_control_decl_expr(value) {
                    pending_controls.push((var_name, value.clone(), stmt.span));
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

            ast::StmtKind::FunctionDef(f) if f.name == "get_control_layout" => {
                if let Some(ui_expr) = extract_declarative_layout_from_method(&f.body) {
                    pending_ui_exprs.push(ui_expr);
                } else {
                    declarations.remaining_statements.push(stmt.clone());
                }
            }

            _ => declarations.remaining_statements.push(stmt.clone()),
        }
    }

    // Parse Control.* declarations now that all Property.* declarations are available
    for (var_name, value, span) in pending_controls {
        if let Some(control_def) = try_parse_control(
            &var_name,
            &value,
            span,
            &declarations.properties,
            &mut declarations.control_property_resolutions,
            diagnostics,
        ) {
            if !seen_controls.insert(var_name.clone()) {
                diagnostics.report(
                    DiagnosticKind::DuplicateControlDeclaration {
                        name: var_name.clone(),
                    },
                    span,
                );
            } else {
                let id = scope_stack.alloc_control_id();
                declarations.controls.push(DeclaredControl {
                    id,
                    ident: var_name,
                    def: control_def,
                });
            }
        }
    }

    let ui_env = UiResolveEnv::from_class_and_module(
        &class.body,
        module_statements,
        &declarations.properties,
    );

    // Second pass: resolve declarative UI.* trees now that all controls are registered
    for ui_expr in &pending_ui_exprs {
        let solved_pages = parse_ui_layout_expr_with_env(
            &class.name,
            ui_expr,
            &declarations.controls,
            &ui_env,
            diagnostics,
            &mut declarations.ui_control_resolutions,
        );
        declarations.pages.extend(solved_pages);
    }

    // Also validate and collect any inline UI.* calls inside hybrid `get_control_layout` methods
    for stmt in &declarations.remaining_statements {
        if let ast::StmtKind::FunctionDef(f) = &stmt.kind
            && f.name == "get_control_layout"
        {
            let mut local_ui_vars = FxHashMap::default();
            let mut method_ui_env = ui_env.clone();
            let inline_pages = collect_inline_ui_pages_from_stmts(
                &class.name,
                &f.body,
                &declarations.controls,
                &mut method_ui_env,
                diagnostics,
                &mut declarations.ui_control_resolutions,
                &mut local_ui_vars,
            );
            if declarations.pages.is_empty() {
                declarations.pages.extend(inline_pages);
            }
        }
    }

    declarations
}

fn is_control_decl_expr(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Call { callee, .. } => match &callee.kind {
            ast::ExprKind::Attribute { value, .. } | ast::ExprKind::Subscript { value, .. } => {
                matches!(&value.kind, ast::ExprKind::Ident(obj) if obj == "Control")
            }
            _ => false,
        },
        _ => false,
    }
}

fn resolve_type_property_arg(
    val: &ast::Expr,
    control_name: &str,
    properties: &[DeclaredProperty],
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<(String, Vec<String>, String)> {
    let mut prop_ident_opt: Option<&str> = None;
    let mut prop_key_opt: Option<String> = None;

    let mut target = val;
    if let ast::ExprKind::Attribute { value, attr } = &target.kind
        && attr == "Value"
    {
        target = value;
    }

    match &target.kind {
        ast::ExprKind::Ident(id) => {
            prop_ident_opt = Some(id.as_str());
        }
        ast::ExprKind::Attribute { value, attr } if matches!(&value.kind, ast::ExprKind::Ident(s) if s == "self") =>
        {
            prop_ident_opt = Some(attr.as_str());
        }
        ast::ExprKind::Subscript { value, index } if matches!(&value.kind, ast::ExprKind::Ident(s) if s == "props" || s == "Properties") => {
            if let Some(s) = eval_constant(index).and_then(|c| c.as_str().map(String::from)) {
                prop_key_opt = Some(s);
            }
        }
        _ => {}
    }

    let default_choices = || {
        vec![
            "Integer".to_string(),
            "Float".to_string(),
            "String".to_string(),
            "Boolean".to_string(),
        ]
    };

    if let Some(prop_ident) = prop_ident_opt
        && let Some(prop) = properties
            .iter()
            .find(|p| p.ident == prop_ident || p.def.name == prop_ident)
    {
        prop_resolutions.push((val.span, prop.id));
        match &prop.def.kind {
            qsys_model::PropertyKind::Enum { choices, default } => {
                return Some((prop.def.name.clone(), choices.clone(), default.clone()));
            }
            qsys_model::PropertyKind::String { default } => {
                let def_choice = if default.is_empty() {
                    "String".to_string()
                } else {
                    default.clone()
                };
                return Some((prop.def.name.clone(), default_choices(), def_choice));
            }
            _ => {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.to_string(),
                        message: format!(
                            "Property '{}' used for generic Control[Type] must be a Property.Enum or Property.String",
                            prop.ident
                        ),
                    },
                    val.span,
                );
                return None;
            }
        }
    }

    if let Some(prop_key) = prop_key_opt {
        if let Some(prop) = properties
            .iter()
            .find(|p| p.def.name == prop_key || p.ident == prop_key)
        {
            prop_resolutions.push((val.span, prop.id));
            match &prop.def.kind {
                qsys_model::PropertyKind::Enum { choices, default } => {
                    return Some((prop.def.name.clone(), choices.clone(), default.clone()));
                }
                qsys_model::PropertyKind::String { default } => {
                    let def_choice = if default.is_empty() {
                        "String".to_string()
                    } else {
                        default.clone()
                    };
                    return Some((prop.def.name.clone(), default_choices(), def_choice));
                }
                _ => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: control_name.to_string(),
                            message: format!(
                                "Property '{}' used for generic Control[Type] must be a Property.Enum or Property.String",
                                prop.def.name
                            ),
                        },
                        val.span,
                    );
                    return None;
                }
            }
        }
        return Some((prop_key, default_choices(), "String".to_string()));
    }

    diagnostics.report(
        DiagnosticKind::InvalidDeclarationArgument {
            name: control_name.to_string(),
            message:
                "Generic Control[Type] requires a declared Property.Enum (e.g. Control[Type](...))"
                    .to_string(),
        },
        val.span,
    );
    None
}

fn eval_count_expr(
    expr: &ast::Expr,
    control_name: &str,
    properties: &[DeclaredProperty],
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
    reported_error: &mut bool,
) -> Option<(i64, String, bool)> {
    match &expr.kind {
        ast::ExprKind::Int(i) => return Some((*i, i.to_string(), false)),
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Pos,
            operand,
        } => {
            return eval_count_expr(
                operand,
                control_name,
                properties,
                prop_resolutions,
                diagnostics,
                reported_error,
            );
        }
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Neg,
            operand,
        } => {
            let (v, s, has_p) = eval_count_expr(
                operand,
                control_name,
                properties,
                prop_resolutions,
                diagnostics,
                reported_error,
            )?;
            return Some((v.saturating_neg(), format!("(-{s})"), has_p));
        }
        ast::ExprKind::Binary { left, op, right } => {
            let (lv, ls, lp) = eval_count_expr(
                left,
                control_name,
                properties,
                prop_resolutions,
                diagnostics,
                reported_error,
            )?;
            let (rv, rs, rp) = eval_count_expr(
                right,
                control_name,
                properties,
                prop_resolutions,
                diagnostics,
                reported_error,
            )?;
            let has_p = lp || rp;
            return match op {
                ast::BinaryOp::Add => {
                    Some((lv.saturating_add(rv), format!("({ls} + {rs})"), has_p))
                }
                ast::BinaryOp::Sub => {
                    Some((lv.saturating_sub(rv), format!("({ls} - {rs})"), has_p))
                }
                ast::BinaryOp::Mul => Some((lv.saturating_mul(rv), format!("{ls} * {rs}"), has_p)),
                ast::BinaryOp::FloorDiv | ast::BinaryOp::Div => {
                    let v = if rv != 0 { lv / rv } else { 1 };
                    Some((v, format!("({ls} // {rs})"), has_p))
                }
                ast::BinaryOp::Mod => {
                    let v = if rv != 0 { lv % rv } else { 0 };
                    Some((v, format!("({ls} % {rs})"), has_p))
                }
                ast::BinaryOp::Pow => {
                    let v = lv.saturating_pow(rv.max(0) as u32);
                    Some((v, format!("({ls} ^ {rs})"), has_p))
                }
                _ => None,
            };
        }
        _ => {}
    }

    let mut prop_ident_opt: Option<&str> = None;
    let mut prop_key_opt: Option<String> = None;

    let mut target = expr;
    if let ast::ExprKind::Attribute { value, attr } = &target.kind
        && attr == "Value"
    {
        target = value;
    }

    match &target.kind {
        ast::ExprKind::Ident(id) => {
            prop_ident_opt = Some(id.as_str());
        }
        ast::ExprKind::Attribute { value, attr } if matches!(&value.kind, ast::ExprKind::Ident(s) if s == "self") =>
        {
            prop_ident_opt = Some(attr.as_str());
        }
        ast::ExprKind::Subscript { value, index } if matches!(&value.kind, ast::ExprKind::Ident(s) if s == "props" || s == "Properties") => {
            if let Some(s) = eval_constant(index).and_then(|c| c.as_str().map(String::from)) {
                prop_key_opt = Some(s);
            }
        }
        _ => {}
    }

    if let Some(prop_ident) = prop_ident_opt
        && let Some(prop) = properties
            .iter()
            .find(|p| p.ident == prop_ident || p.def.name == prop_ident)
    {
        prop_resolutions.push((expr.span, prop.id));
        if let qsys_model::PropertyKind::Integer { default, .. } = &prop.def.kind {
            return Some((
                (*default).max(1),
                format!("{{PROP:{}}}", prop.def.name),
                true,
            ));
        }
        *reported_error = true;
        diagnostics.report(
            DiagnosticKind::InvalidDeclarationArgument {
                name: control_name.to_string(),
                message: format!(
                    "Property '{}' used for 'count' must be a Property.Integer",
                    prop.ident
                ),
            },
            expr.span,
        );
        return None;
    }

    if let Some(prop_key) = prop_key_opt {
        if let Some(prop) = properties
            .iter()
            .find(|p| p.def.name == prop_key || p.ident == prop_key)
        {
            prop_resolutions.push((expr.span, prop.id));
            if let qsys_model::PropertyKind::Integer { default, .. } = &prop.def.kind {
                return Some((
                    (*default).max(1),
                    format!("{{PROP:{}}}", prop.def.name),
                    true,
                ));
            }
            *reported_error = true;
            diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: control_name.to_string(),
                    message: format!(
                        "Property '{}' used for 'count' must be a Property.Integer",
                        prop.def.name
                    ),
                },
                expr.span,
            );
            return None;
        }
        return Some((1, format!("{{PROP:{prop_key}}}"), true));
    }

    None
}

fn resolve_count_arg(
    val: &ast::Expr,
    control_name: &str,
    properties: &[DeclaredProperty],
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<(usize, Option<String>)> {
    let mut reported_error = false;
    if let Some((evaluated_val, template, has_prop)) = eval_count_expr(
        val,
        control_name,
        properties,
        prop_resolutions,
        diagnostics,
        &mut reported_error,
    ) {
        if !has_prop {
            if evaluated_val > 0 {
                return Some((evaluated_val as usize, None));
            }
            diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: control_name.to_string(),
                    message: "'count' must be a positive integer or a Property.Integer".to_string(),
                },
                val.span,
            );
            return None;
        }

        let preview_count = evaluated_val.clamp(1, 128) as usize;
        let normalized_spec = if let Some(inner) = template
            .strip_prefix("{PROP:")
            .and_then(|s| s.strip_suffix('}'))
            && !inner.contains("{PROP:")
        {
            inner.to_string()
        } else {
            template
        };
        return Some((preview_count, Some(normalized_spec)));
    }

    if !reported_error {
        diagnostics.report(
            DiagnosticKind::InvalidDeclarationArgument {
                name: control_name.to_string(),
                message: "'count' must be a positive integer or a Property.Integer".to_string(),
            },
            val.span,
        );
    }
    None
}

fn try_parse_control(
    var_name: &str,
    expr: &ast::Expr,
    span: Span,
    properties: &[DeclaredProperty],
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<ControlDef> {
    let (kind_str, subscript_type_expr, args, kwargs) = match &expr.kind {
        ast::ExprKind::Call {
            callee,
            args,
            kwargs,
        } => match &callee.kind {
            ast::ExprKind::Attribute { value, attr } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Control" => {
                    (attr.as_str(), None, args, kwargs)
                }
                _ => return None,
            },
            ast::ExprKind::Subscript { value, index } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Control" => {
                    ("Generic", Some(index.as_ref()), args, kwargs)
                }
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };

    let is_generic_method =
        matches!(kind_str, "Generic" | "Dynamic") && subscript_type_expr.is_none();
    let has_kwarg_type = kwargs.iter().any(|(k, _)| k == "type" || k == "property");
    let positional_name_arg = if is_generic_method && !has_kwarg_type {
        args.get(1)
    } else {
        args.first()
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
        None => match positional_name_arg {
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

    let (count, count_property) = match kwargs.iter().find(|(k, _)| k == "count") {
        Some((_, val)) => resolve_count_arg(
            val,
            &control_name,
            properties,
            prop_resolutions,
            diagnostics,
        )?,
        None => (1, None),
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

    let user_pin = match kwargs.iter().find(|(k, _)| k == "user_pin") {
        Some((_, val)) => match eval_constant(val).and_then(|c| c.as_bool()) {
            Some(b) => b,
            None => {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.clone(),
                        message: "'user_pin' must be a boolean literal".to_string(),
                    },
                    val.span,
                );
                return None;
            }
        },
        None => true,
    };

    let parse_unit = |diagnostics: &mut Diagnostics| -> Option<ControlUnit> {
        match kwargs.iter().find(|(k, _)| k == "unit") {
            Some((_, val)) => match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                Some(ref s) if s == "Float" => Some(ControlUnit::Float),
                Some(ref s) if s == "Integer" => Some(ControlUnit::Integer),
                Some(ref s) if s == "dB" || s == "Decibels" => Some(ControlUnit::Decibels),
                Some(ref s) if s == "Hz" || s == "Hertz" => Some(ControlUnit::Hertz),
                Some(ref s) if s == "Percent" => Some(ControlUnit::Percent),
                Some(ref s) if s == "Seconds" => Some(ControlUnit::Seconds),
                Some(ref s) if s == "Pan" => Some(ControlUnit::Pan),
                _ => {
                    diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "Invalid 'unit'. Expected 'Float', 'Integer', 'dB', 'Hz', 'Percent', 'Seconds', or 'Pan'".to_string(),
                            },
                            val.span,
                        );
                    None
                }
            },
            None => Some(ControlUnit::Float),
        }
    };

    let kind = match kind_str {
        "Float" | "Knob" | "Fader" => {
            let unit = parse_unit(diagnostics)?;

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
                    unit,
                    min,
                    max,
                    default,
                }
            } else {
                ControlKind::Knob {
                    unit,
                    min,
                    max,
                    default,
                }
            }
        }

        "Boolean" | "Button" => {
            let default_btn_type = if kind_str == "Boolean" {
                ButtonType::Toggle
            } else {
                ButtonType::Momentary
            };
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
                None => default_btn_type,
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
            let text_type = match kwargs.iter().find(|(k, _)| k == "type") {
                Some((_, val)) => {
                    match eval_constant(val).and_then(|c| c.as_str().map(String::from)) {
                        Some(ref s) if s == "Text" => TextType::Text,
                        Some(ref s) if s == "ReadOnly" => TextType::ReadOnly,
                        _ => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Invalid text 'type'. Expected 'Text' or 'ReadOnly'"
                                        .to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => TextType::Text,
            };
            ControlKind::Text { text_type, default }
        }

        "Indicator" => {
            let unit = parse_unit(diagnostics)?;
            ControlKind::Indicator { unit }
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

        // Property-typed generic control: `Control[Type](...)`, `Control.[Type](...)`, or `Control.Generic(Type, ...)`
        "Generic" | "Dynamic" => {
            let type_expr = subscript_type_expr
                .or_else(|| {
                    kwargs
                        .iter()
                        .find(|(k, _)| k == "type" || k == "property")
                        .map(|(_, v)| v)
                })
                .or_else(|| args.first());

            let Some(type_expr) = type_expr else {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name,
                        message: "Control.Generic requires a Property reference (e.g. Control[Type]() or Control.Generic(Type))".to_string(),
                    },
                    span,
                );
                return None;
            };

            let (prop_name, choices, default_choice) = resolve_type_property_arg(
                type_expr,
                &control_name,
                properties,
                prop_resolutions,
                diagnostics,
            )?;

            let unit = parse_unit(diagnostics)?;
            let min = kwargs
                .iter()
                .find(|(k, _)| k == "min")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_f64())
                .unwrap_or(0.0);
            let max = kwargs
                .iter()
                .find(|(k, _)| k == "max")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_f64())
                .unwrap_or(100.0);

            let (default_num, default_str) = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => match eval_constant(val) {
                    Some(ConstValue::Float(f)) => (f, f.to_string()),
                    Some(ConstValue::Int(i)) => (i as f64, i.to_string()),
                    Some(ConstValue::String(s)) => {
                        let num = s.parse::<f64>().unwrap_or(min);
                        (num, s)
                    }
                    Some(ConstValue::Bool(b)) => (if b { 1.0 } else { 0.0 }, b.to_string()),
                    _ => (min, String::new()),
                },
                None => (min, String::new()),
            };

            let gspec = GenericControlSpec {
                property_name: prop_name,
                choices,
                default_choice: default_choice.clone(),
                unit,
                min,
                max,
                default_num,
                default_str,
                button_type: ButtonType::Toggle,
                text_type: TextType::Text,
            };
            let resolved_kind = gspec.resolve_choice_kind(&default_choice);

            let mut def = ControlDef::new(control_name, resolved_kind)
                .with_pin_style(pin_style)
                .with_user_pin(user_pin)
                .with_count_property(count_property)
                .with_generic_spec(Some(gspec));
            if count > 1 {
                def = def.with_count(count);
            }
            return Some(def);
        }

        _ => {
            diagnostics.report(
                DiagnosticKind::syntax_error(format!("Unknown Control type 'Control.{kind_str}'")),
                span,
            );
            return None;
        }
    };

    let mut def = ControlDef::new(control_name, kind)
        .with_pin_style(pin_style)
        .with_user_pin(user_pin)
        .with_count_property(count_property);
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

        "Boolean" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| eval_constant(v))
                .and_then(|c| c.as_bool())
                .unwrap_or(false);
            PropertyDef::boolean(prop_name, default)
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
        "manufacturer" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.manufacturer = Some(s);
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
