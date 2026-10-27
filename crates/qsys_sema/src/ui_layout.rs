use rustc_hash::FxHashMap;
use strsim::levenshtein;

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::{ControlId, PropertyId};
use qsys_model::{
    Color, ControlStyle, FlexDirection, GridCellPreview, LabelPlacement, PageLayout, PropertyKind,
    UiControlSpec, UiGridSpec, UiNode, UiPageSpec,
};
use qsys_parser::ast;

use crate::declarations::{
    ConstValue, DeclaredControl, DeclaredProperty, eval_constant, eval_constant_with_env,
};

#[derive(Debug, Clone, Default)]
pub struct UiResolveEnv<'a> {
    pub properties: &'a [DeclaredProperty],
    pub class_methods: FxHashMap<String, ast::FunctionDef>,
    pub module_functions: FxHashMap<String, ast::FunctionDef>,
    pub local_functions: FxHashMap<String, ast::FunctionDef>,
    pub constants: FxHashMap<String, ConstValue>,
}

impl<'a> UiResolveEnv<'a> {
    pub fn empty() -> Self {
        Self {
            properties: &[],
            class_methods: FxHashMap::default(),
            module_functions: FxHashMap::default(),
            local_functions: FxHashMap::default(),
            constants: FxHashMap::default(),
        }
    }

    pub fn from_class_and_module(
        class_body: &[ast::Stmt],
        module_stmts: &[ast::Stmt],
        properties: &'a [DeclaredProperty],
    ) -> Self {
        let mut env = Self {
            properties,
            class_methods: FxHashMap::default(),
            module_functions: FxHashMap::default(),
            local_functions: FxHashMap::default(),
            constants: FxHashMap::default(),
        };

        for stmt in module_stmts {
            match &stmt.kind {
                ast::StmtKind::FunctionDef(f) => {
                    env.module_functions.insert(f.name.clone(), f.clone());
                }
                ast::StmtKind::Assign { target, value, .. } => {
                    if let ast::ExprKind::Ident(name) = &target.kind
                        && let Some(val) = eval_constant_with_env(value, &env.constants)
                            .or_else(|| eval_constant(value))
                    {
                        env.constants.insert(name.clone(), val);
                    }
                }
                _ => {}
            }
        }

        for stmt in class_body {
            match &stmt.kind {
                ast::StmtKind::FunctionDef(f) => {
                    env.class_methods.insert(f.name.clone(), f.clone());
                }
                ast::StmtKind::Assign { target, value, .. } => {
                    if let ast::ExprKind::Ident(name) = &target.kind
                        && let Some(val) = eval_constant_with_env(value, &env.constants)
                            .or_else(|| eval_constant(value))
                    {
                        env.constants.insert(name.clone(), val);
                    }
                }
                _ => {}
            }
        }

        env
    }

    pub(crate) fn collect_property_resolutions(
        &self,
        class_name: &str,
        expr: &ast::Expr,
        out: &mut Vec<(Span, PropertyId)>,
    ) {
        match &expr.kind {
            ast::ExprKind::Ident(name) => {
                if let Some(prop) = self
                    .properties
                    .iter()
                    .find(|p| p.ident == *name || p.def.name == *name)
                {
                    out.push((expr.span, prop.id));
                }
            }
            ast::ExprKind::Attribute { value, attr } => {
                if attr == "Value" {
                    if let ast::ExprKind::Attribute {
                        value: inner_obj,
                        attr: prop_ident,
                    } = &value.kind
                        && let ast::ExprKind::Ident(obj_name) = &inner_obj.kind
                        && (obj_name == "self" || obj_name == class_name)
                        && let Some(prop) = self
                            .properties
                            .iter()
                            .find(|p| p.ident == *prop_ident || p.def.name == *prop_ident)
                    {
                        out.push((expr.span, prop.id));
                        return;
                    }
                    if let ast::ExprKind::Ident(prop_ident) = &value.kind
                        && let Some(prop) = self
                            .properties
                            .iter()
                            .find(|p| p.ident == *prop_ident || p.def.name == *prop_ident)
                    {
                        out.push((expr.span, prop.id));
                        return;
                    }
                    if let ast::ExprKind::Subscript {
                        value: inner_obj,
                        index,
                    } = &value.kind
                        && matches!(
                            &inner_obj.kind,
                            ast::ExprKind::Ident(s) if s == "props" || s == "Properties"
                        )
                        && let Some(key) = eval_constant_with_env(index, &self.constants)
                            .and_then(|c| c.as_str().map(String::from))
                        && let Some(prop) = self
                            .properties
                            .iter()
                            .find(|p| p.def.name == key || p.ident == key)
                    {
                        out.push((expr.span, prop.id));
                        return;
                    }
                }
                if let ast::ExprKind::Ident(obj_name) = &value.kind
                    && (obj_name == "self" || obj_name == class_name)
                    && let Some(prop) = self
                        .properties
                        .iter()
                        .find(|p| p.ident == *attr || p.def.name == *attr)
                {
                    out.push((expr.span, prop.id));
                    return;
                }
                self.collect_property_resolutions(class_name, value, out);
            }
            ast::ExprKind::Subscript { value, index } => {
                if matches!(
                    &value.kind,
                    ast::ExprKind::Ident(s) if s == "props" || s == "Properties"
                ) && let Some(key) = eval_constant_with_env(index, &self.constants)
                    .and_then(|c| c.as_str().map(String::from))
                    && let Some(prop) = self
                        .properties
                        .iter()
                        .find(|p| p.def.name == key || p.ident == key)
                {
                    out.push((expr.span, prop.id));
                    return;
                }
                self.collect_property_resolutions(class_name, value, out);
                self.collect_property_resolutions(class_name, index, out);
            }
            ast::ExprKind::Unary { operand, .. } => {
                self.collect_property_resolutions(class_name, operand, out);
            }
            ast::ExprKind::Binary { left, right, .. }
            | ast::ExprKind::BoolOp { left, right, .. } => {
                self.collect_property_resolutions(class_name, left, out);
                self.collect_property_resolutions(class_name, right, out);
            }
            ast::ExprKind::IfExp { test, body, orelse } => {
                self.collect_property_resolutions(class_name, test, out);
                self.collect_property_resolutions(class_name, body, out);
                self.collect_property_resolutions(class_name, orelse, out);
            }
            ast::ExprKind::Call {
                callee,
                args,
                kwargs,
            } => {
                self.collect_property_resolutions(class_name, callee, out);
                for arg in args {
                    self.collect_property_resolutions(class_name, arg, out);
                }
                for (_, kw_val) in kwargs {
                    self.collect_property_resolutions(class_name, kw_val, out);
                }
            }
            ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
                for item in items {
                    self.collect_property_resolutions(class_name, item, out);
                }
            }
            ast::ExprKind::Lambda { body, .. } => {
                self.collect_property_resolutions(class_name, body, out);
            }
            _ => {}
        }
    }

    pub(crate) fn is_dynamic_design_expr(&self, class_name: &str, expr: &ast::Expr) -> bool {
        match &expr.kind {
            ast::ExprKind::Int(_)
            | ast::ExprKind::Float(_)
            | ast::ExprKind::String(_)
            | ast::ExprKind::Bool(_)
            | ast::ExprKind::None => false,
            ast::ExprKind::Ident(name) => self
                .properties
                .iter()
                .any(|p| p.ident == *name || p.def.name == *name),
            ast::ExprKind::Attribute { value, attr } => {
                if attr == "Value" {
                    return true;
                }
                if let ast::ExprKind::Ident(obj_name) = &value.kind
                    && (obj_name == "self" || obj_name == class_name)
                {
                    if self
                        .properties
                        .iter()
                        .any(|p| p.ident == *attr || p.def.name == *attr)
                    {
                        return true;
                    }
                    if self.constants.contains_key(attr) {
                        return false;
                    }
                }
                self.is_dynamic_design_expr(class_name, value)
            }
            ast::ExprKind::Subscript { value, index } => {
                if matches!(
                    &value.kind,
                    ast::ExprKind::Ident(s) if s == "props" || s == "Properties"
                ) {
                    return true;
                }
                self.is_dynamic_design_expr(class_name, value)
                    || self.is_dynamic_design_expr(class_name, index)
            }
            ast::ExprKind::Unary { operand, .. } => {
                self.is_dynamic_design_expr(class_name, operand)
            }
            ast::ExprKind::Binary { left, right, .. }
            | ast::ExprKind::BoolOp { left, right, .. } => {
                self.is_dynamic_design_expr(class_name, left)
                    || self.is_dynamic_design_expr(class_name, right)
            }
            ast::ExprKind::IfExp { test, body, orelse } => {
                self.is_dynamic_design_expr(class_name, test)
                    || self.is_dynamic_design_expr(class_name, body)
                    || self.is_dynamic_design_expr(class_name, orelse)
            }
            ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => items
                .iter()
                .any(|i| self.is_dynamic_design_expr(class_name, i)),
            ast::ExprKind::Call { callee, args, .. } => {
                if let ast::ExprKind::Ident(fn_name) = &callee.kind
                    && matches!(
                        fn_name.as_str(),
                        "str" | "int" | "float" | "bool" | "abs" | "min" | "max" | "round"
                    )
                    && !self.local_functions.contains_key(fn_name)
                    && !self.class_methods.contains_key(fn_name)
                    && !self.module_functions.contains_key(fn_name)
                {
                    return args
                        .iter()
                        .any(|a| self.is_dynamic_design_expr(class_name, a));
                }
                true
            }
            ast::ExprKind::Lambda { .. } => true,
            _ => false,
        }
    }

    pub(crate) fn eval_design_expr_preview(
        &self,
        class_name: &str,
        expr: &ast::Expr,
    ) -> Option<PreviewVal> {
        let mut vars = FxHashMap::default();
        for (name, val) in &self.constants {
            vars.insert(name.clone(), PreviewVal::from_const(val));
        }
        self.eval_preview_expr_in_env(class_name, expr, &vars, 0)
    }

    pub(crate) fn lower_design_expr_to_lua(&self, class_name: &str, expr: &ast::Expr) -> String {
        self.lower_ui_expr_to_lua_inner(class_name, expr, &FxHashMap::default(), false)
    }

    pub(crate) fn lower_design_expr_to_prop_template(
        &self,
        class_name: &str,
        expr: &ast::Expr,
    ) -> String {
        self.lower_ui_expr_to_lua_inner(class_name, expr, &FxHashMap::default(), true)
    }

    fn format_prop_value_lua(prop_name: &str, prop_template_mode: bool) -> String {
        if prop_template_mode {
            format!("{{PROP:{prop_name}}}")
        } else {
            format!("props[\"{}\"].Value", escape_lua_str_content(prop_name))
        }
    }

    fn const_val_to_lua(cval: &ConstValue) -> String {
        match cval {
            ConstValue::Int(i) => {
                if *i < 0 {
                    format!("({i})")
                } else {
                    i.to_string()
                }
            }
            ConstValue::Float(f) => {
                if *f < 0.0 {
                    format!("({f})")
                } else {
                    f.to_string()
                }
            }
            ConstValue::String(s) => format!("\"{}\"", escape_lua_str_content(s)),
            ConstValue::Bool(b) => b.to_string(),
            ConstValue::List(items) => {
                let parts: Vec<String> = items.iter().map(Self::const_val_to_lua).collect();
                format!("{{ {} }}", parts.join(", "))
            }
        }
    }

    pub(crate) fn lower_ui_expr_to_lua_inner(
        &self,
        class_name: &str,
        expr: &ast::Expr,
        local_map: &FxHashMap<String, String>,
        prop_template_mode: bool,
    ) -> String {
        match &expr.kind {
            ast::ExprKind::Int(i) => i.to_string(),
            ast::ExprKind::Float(f) => f.to_string(),
            ast::ExprKind::String(s) => format!("\"{}\"", escape_lua_str_content(s)),
            ast::ExprKind::Bool(b) => b.to_string(),
            ast::ExprKind::None => "nil".to_string(),
            ast::ExprKind::Ident(name) => {
                if let Some(mapped) = local_map.get(name) {
                    return mapped.clone();
                }
                if let Some(prop) = self
                    .properties
                    .iter()
                    .find(|p| p.ident == *name || p.def.name == *name)
                {
                    return Self::format_prop_value_lua(&prop.def.name, prop_template_mode);
                }
                if let Some(cval) = self.constants.get(name) {
                    return Self::const_val_to_lua(cval);
                }
                name.clone()
            }
            ast::ExprKind::Attribute { value, attr } => {
                if attr == "Value" {
                    if let ast::ExprKind::Attribute {
                        value: inner_obj,
                        attr: prop_ident,
                    } = &value.kind
                        && let ast::ExprKind::Ident(obj_name) = &inner_obj.kind
                        && (obj_name == "self" || obj_name == class_name)
                        && let Some(prop) = self
                            .properties
                            .iter()
                            .find(|p| p.ident == *prop_ident || p.def.name == *prop_ident)
                    {
                        return Self::format_prop_value_lua(&prop.def.name, prop_template_mode);
                    }
                    if let ast::ExprKind::Ident(prop_ident) = &value.kind
                        && let Some(prop) = self
                            .properties
                            .iter()
                            .find(|p| p.ident == *prop_ident || p.def.name == *prop_ident)
                    {
                        return Self::format_prop_value_lua(&prop.def.name, prop_template_mode);
                    }
                    if let ast::ExprKind::Subscript {
                        value: inner_obj,
                        index,
                    } = &value.kind
                        && let ast::ExprKind::Ident(obj) = &inner_obj.kind
                        && (obj == "props" || obj == "Properties")
                    {
                        if prop_template_mode
                            && let Some(s) = eval_constant_with_env(index, &self.constants)
                                .and_then(|c| c.as_str().map(String::from))
                        {
                            return format!("{{PROP:{s}}}");
                        }
                        let idx_lua = self.lower_ui_expr_to_lua_inner(
                            class_name,
                            index,
                            local_map,
                            prop_template_mode,
                        );
                        return format!("props[{idx_lua}].Value");
                    }
                }
                if let ast::ExprKind::Ident(obj_name) = &value.kind
                    && (obj_name == "self" || obj_name == class_name)
                {
                    if let Some(prop) = self
                        .properties
                        .iter()
                        .find(|p| p.ident == *attr || p.def.name == *attr)
                    {
                        return Self::format_prop_value_lua(&prop.def.name, prop_template_mode);
                    }
                    if let Some(cval) = self.constants.get(attr) {
                        return Self::const_val_to_lua(cval);
                    }
                }
                if let ast::ExprKind::Ident(prop_ident) = &value.kind
                    && let Some(prop) = self
                        .properties
                        .iter()
                        .find(|p| p.ident == *prop_ident || p.def.name == *prop_ident)
                {
                    return format!(
                        "props[\"{}\"].{attr}",
                        escape_lua_str_content(&prop.def.name)
                    );
                }
                if let ast::ExprKind::Attribute {
                    value: inner_obj,
                    attr: prop_ident,
                } = &value.kind
                    && let ast::ExprKind::Ident(obj_name) = &inner_obj.kind
                    && (obj_name == "self" || obj_name == class_name)
                    && let Some(prop) = self
                        .properties
                        .iter()
                        .find(|p| p.ident == *prop_ident || p.def.name == *prop_ident)
                {
                    return format!(
                        "props[\"{}\"].{attr}",
                        escape_lua_str_content(&prop.def.name)
                    );
                }
                if let ast::ExprKind::Subscript {
                    value: inner_obj,
                    index,
                } = &value.kind
                    && let ast::ExprKind::Ident(obj) = &inner_obj.kind
                    && (obj == "props" || obj == "Properties")
                {
                    let idx_lua = self.lower_ui_expr_to_lua_inner(
                        class_name,
                        index,
                        local_map,
                        prop_template_mode,
                    );
                    return format!("props[{idx_lua}].{attr}");
                }
                let base = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    value,
                    local_map,
                    prop_template_mode,
                );
                format!("{base}.{attr}")
            }
            ast::ExprKind::Subscript { value, index } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "props" || obj == "Properties")
                {
                    if prop_template_mode
                        && let Some(s) = eval_constant_with_env(index, &self.constants)
                            .and_then(|c| c.as_str().map(String::from))
                    {
                        return format!("{{PROP:{s}}}");
                    }
                    let idx_lua = self.lower_ui_expr_to_lua_inner(
                        class_name,
                        index,
                        local_map,
                        prop_template_mode,
                    );
                    return format!("props[{idx_lua}].Value");
                }
                let base = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    value,
                    local_map,
                    prop_template_mode,
                );
                let idx_lua = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    index,
                    local_map,
                    prop_template_mode,
                );
                format!("{base}[({idx_lua}) + 1]")
            }
            ast::ExprKind::Unary { op, operand } => {
                let inner = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    operand,
                    local_map,
                    prop_template_mode,
                );
                match op {
                    ast::UnaryOp::Neg => format!("(-{inner})"),
                    ast::UnaryOp::Pos => inner,
                    ast::UnaryOp::Not => format!("(not {inner})"),
                    ast::UnaryOp::BitNot => format!("(~{inner})"),
                }
            }
            ast::ExprKind::Binary { left, op, right } => {
                let l = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    left,
                    local_map,
                    prop_template_mode,
                );
                let r = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    right,
                    local_map,
                    prop_template_mode,
                );
                let is_str = |e: &ast::Expr| -> bool {
                    matches!(e.kind, ast::ExprKind::String(_))
                        || matches!(
                            &e.kind,
                            ast::ExprKind::Call { callee, .. }
                                if matches!(&callee.kind, ast::ExprKind::Ident(n) if n == "str" || n == "tostring")
                        )
                        || matches!(
                            self.eval_design_expr_preview(class_name, e),
                            Some(PreviewVal::Str(_))
                        )
                };
                if *op == ast::BinaryOp::Add && (is_str(left) || is_str(right)) {
                    return format!("({l} .. {r})");
                }
                let op_str = match op {
                    ast::BinaryOp::Add => "+",
                    ast::BinaryOp::Sub => "-",
                    ast::BinaryOp::Mul => "*",
                    ast::BinaryOp::Div => "/",
                    ast::BinaryOp::FloorDiv => "//",
                    ast::BinaryOp::Mod => "%",
                    ast::BinaryOp::Pow => "^",
                    ast::BinaryOp::BitAnd => "&",
                    ast::BinaryOp::BitOr => "|",
                    ast::BinaryOp::BitXor => "~",
                    ast::BinaryOp::LShift => "<<",
                    ast::BinaryOp::RShift => ">>",
                    ast::BinaryOp::Eq => "==",
                    ast::BinaryOp::NotEq => "~=",
                    ast::BinaryOp::Lt => "<",
                    ast::BinaryOp::LtEq => "<=",
                    ast::BinaryOp::Gt => ">",
                    ast::BinaryOp::GtEq => ">=",
                };
                format!("({l} {op_str} {r})")
            }
            ast::ExprKind::BoolOp { op, left, right } => {
                let op_str = match op {
                    ast::LogicalOp::And => "and",
                    ast::LogicalOp::Or => "or",
                };
                let l = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    left,
                    local_map,
                    prop_template_mode,
                );
                let r = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    right,
                    local_map,
                    prop_template_mode,
                );
                format!("({l} {op_str} {r})")
            }
            ast::ExprKind::IfExp { test, body, orelse } => {
                let t = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    test,
                    local_map,
                    prop_template_mode,
                );
                let b = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    body,
                    local_map,
                    prop_template_mode,
                );
                let o = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    orelse,
                    local_map,
                    prop_template_mode,
                );
                if matches!(body.kind, ast::ExprKind::Bool(false) | ast::ExprKind::None) {
                    format!("(function() if {t} then return {b} else return {o} end end)()")
                } else {
                    format!("({t} and {b} or {o})")
                }
            }
            ast::ExprKind::Call { callee, args, .. } => {
                let rendered_args: Vec<String> = args
                    .iter()
                    .map(|a| {
                        self.lower_ui_expr_to_lua_inner(
                            class_name,
                            a,
                            local_map,
                            prop_template_mode,
                        )
                    })
                    .collect();
                if let ast::ExprKind::Ident(fn_name) = &callee.kind {
                    if !self.local_functions.contains_key(fn_name)
                        && !self.module_functions.contains_key(fn_name)
                        && !self.class_methods.contains_key(fn_name)
                    {
                        match fn_name.as_str() {
                            "str" if rendered_args.len() == 1 => {
                                return format!("tostring({})", rendered_args[0]);
                            }
                            "int" if rendered_args.len() == 1 => {
                                return format!("math.floor(tonumber({}) or 0)", rendered_args[0]);
                            }
                            "float" if rendered_args.len() == 1 => {
                                return format!("(tonumber({}) or 0.0)", rendered_args[0]);
                            }
                            "bool" if rendered_args.len() == 1 => {
                                return format!("__py_truthy({})", rendered_args[0]);
                            }
                            "abs" if rendered_args.len() == 1 => {
                                return format!("math.abs({})", rendered_args[0]);
                            }
                            "round" if rendered_args.len() == 1 => {
                                return format!("math.floor(({}) + 0.5)", rendered_args[0]);
                            }
                            "min" if !rendered_args.is_empty() => {
                                return format!("math.min({})", rendered_args.join(", "));
                            }
                            "max" if !rendered_args.is_empty() => {
                                return format!("math.max({})", rendered_args.join(", "));
                            }
                            _ => {}
                        }
                    }
                    if !self.local_functions.contains_key(fn_name)
                        && !self.module_functions.contains_key(fn_name)
                        && self.class_methods.contains_key(fn_name)
                    {
                        let mut all_args = vec!["nil".to_string()];
                        all_args.extend(rendered_args);
                        return format!("{class_name}_{fn_name}({})", all_args.join(", "));
                    }
                    return format!("{fn_name}({})", rendered_args.join(", "));
                }
                if let ast::ExprKind::Attribute { value, attr } = &callee.kind
                    && let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == class_name)
                {
                    let mut all_args = vec!["nil".to_string()];
                    all_args.extend(rendered_args);
                    return format!("{class_name}_{attr}({})", all_args.join(", "));
                }
                let callee_lua = self.lower_ui_expr_to_lua_inner(
                    class_name,
                    callee,
                    local_map,
                    prop_template_mode,
                );
                format!("{callee_lua}({})", rendered_args.join(", "))
            }
            _ => "\"\"".to_string(),
        }
    }

    pub(crate) fn eval_preview_function_in_env(
        &self,
        class_name: &str,
        func: &ast::FunctionDef,
        args: &[PreviewVal],
        depth: usize,
    ) -> Option<PreviewVal> {
        if depth > 8 {
            return None;
        }
        let mut vars = FxHashMap::default();
        for (name, val) in &self.constants {
            vars.insert(name.clone(), PreviewVal::from_const(val));
        }
        let skip_self = usize::from(func.params.first().is_some_and(|p| p.name == "self"));
        for (param, arg_val) in func.params.iter().skip(skip_self).zip(args.iter()) {
            vars.insert(param.name.clone(), arg_val.clone());
        }
        for param in func.params.iter().skip(skip_self + args.len()) {
            if let Some(ref def_expr) = param.default
                && let Some(def_val) =
                    self.eval_preview_expr_in_env(class_name, def_expr, &vars, depth + 1)
            {
                vars.insert(param.name.clone(), def_val);
            }
        }
        self.eval_preview_stmts_in_env(class_name, &func.body, &mut vars, depth)
    }

    fn eval_preview_stmts_in_env(
        &self,
        class_name: &str,
        stmts: &[ast::Stmt],
        vars: &mut FxHashMap<String, PreviewVal>,
        depth: usize,
    ) -> Option<PreviewVal> {
        for stmt in stmts {
            match &stmt.kind {
                ast::StmtKind::Pass | ast::StmtKind::Global(_) | ast::StmtKind::Nonlocal(_) => {}
                ast::StmtKind::Expr(e) if matches!(e.kind, ast::ExprKind::String(_)) => {}
                ast::StmtKind::Assign { target, value, .. } => {
                    let val = self.eval_preview_expr_in_env(class_name, value, vars, depth)?;
                    match &target.kind {
                        ast::ExprKind::Ident(name) => {
                            vars.insert(name.clone(), val);
                        }
                        ast::ExprKind::Tuple(targets) | ast::ExprKind::List(targets) => {
                            if let PreviewVal::Tuple(items) = val {
                                for (t, item) in targets.iter().zip(items) {
                                    if let ast::ExprKind::Ident(name) = &t.kind {
                                        vars.insert(name.clone(), item);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                ast::StmtKind::If {
                    condition,
                    then_branch,
                    else_branch,
                } => {
                    let cond = self.eval_preview_expr_in_env(class_name, condition, vars, depth)?;
                    let branch = if cond.is_truthy() {
                        then_branch
                    } else {
                        else_branch
                    };
                    if let Some(ret) =
                        self.eval_preview_stmts_in_env(class_name, branch, vars, depth)
                    {
                        return Some(ret);
                    }
                }
                ast::StmtKind::Return(Some(ret_expr)) => {
                    return self.eval_preview_expr_in_env(class_name, ret_expr, vars, depth);
                }
                ast::StmtKind::Return(None) => return None,
                _ => {}
            }
        }
        None
    }

    pub(crate) fn lookup_property_default(&self, ident_or_name: &str) -> Option<PreviewVal> {
        let prop = self
            .properties
            .iter()
            .find(|p| p.ident == ident_or_name || p.def.name == ident_or_name)?;
        match &prop.def.kind {
            PropertyKind::Integer { default, .. } => Some(PreviewVal::Int(*default)),
            PropertyKind::Double { default, .. } => Some(PreviewVal::Float(*default)),
            PropertyKind::String { default } | PropertyKind::Enum { default, .. } => {
                Some(PreviewVal::Str(default.clone()))
            }
            PropertyKind::Boolean { default } => Some(PreviewVal::Bool(*default)),
        }
    }

    pub(crate) fn eval_preview_expr_in_env(
        &self,
        class_name: &str,
        expr: &ast::Expr,
        vars: &FxHashMap<String, PreviewVal>,
        depth: usize,
    ) -> Option<PreviewVal> {
        match &expr.kind {
            ast::ExprKind::Int(i) => Some(PreviewVal::Int(*i)),
            ast::ExprKind::Float(f) => Some(PreviewVal::Float(*f)),
            ast::ExprKind::String(s) => Some(PreviewVal::Str(s.clone())),
            ast::ExprKind::Bool(b) => Some(PreviewVal::Bool(*b)),
            ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval_preview_expr_in_env(class_name, item, vars, depth)?);
                }
                Some(PreviewVal::Tuple(out))
            }
            ast::ExprKind::Ident(name) => {
                if let Some(v) = vars.get(name) {
                    return Some(v.clone());
                }
                if let Some(cval) = self.constants.get(name) {
                    return Some(PreviewVal::from_const(cval));
                }
                if let Some(prop_val) = self.lookup_property_default(name) {
                    return Some(prop_val);
                }
                None
            }
            ast::ExprKind::Attribute { value, attr } => {
                if attr == "Value" {
                    if let ast::ExprKind::Attribute {
                        value: inner_obj,
                        attr: prop_ident,
                    } = &value.kind
                        && let ast::ExprKind::Ident(obj_name) = &inner_obj.kind
                        && (obj_name == "self" || obj_name == class_name)
                    {
                        return self.lookup_property_default(prop_ident);
                    }
                    if let ast::ExprKind::Ident(prop_ident) = &value.kind
                        && let Some(val) = self.lookup_property_default(prop_ident)
                    {
                        return Some(val);
                    }
                    if let Some(PreviewVal::Prop(prop_name)) =
                        self.eval_preview_expr_in_env(class_name, value, vars, depth)
                    {
                        return self.lookup_property_default(&prop_name);
                    }
                }
                if let ast::ExprKind::Ident(obj_name) = &value.kind
                    && (obj_name == "self" || obj_name == class_name)
                {
                    if let Some(val) = self.lookup_property_default(attr) {
                        return Some(val);
                    }
                    if let Some(cval) = self.constants.get(attr) {
                        return Some(PreviewVal::from_const(cval));
                    }
                }
                None
            }
            ast::ExprKind::Subscript { value, index } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "props" || obj == "Properties")
                    && let Some(PreviewVal::Str(prop_name)) =
                        self.eval_preview_expr_in_env(class_name, index, vars, depth)
                {
                    return self
                        .lookup_property_default(&prop_name)
                        .or(Some(PreviewVal::Prop(prop_name)));
                }
                let container = self.eval_preview_expr_in_env(class_name, value, vars, depth)?;
                let idx = self
                    .eval_preview_expr_in_env(class_name, index, vars, depth)?
                    .as_i64()?;
                if let PreviewVal::Tuple(items) = container
                    && idx >= 0
                {
                    return items.get(idx as usize).cloned();
                }
                None
            }
            ast::ExprKind::Unary { op, operand } => {
                let val = self.eval_preview_expr_in_env(class_name, operand, vars, depth)?;
                match op {
                    ast::UnaryOp::Neg => match val {
                        PreviewVal::Int(i) => Some(PreviewVal::Int(-i)),
                        PreviewVal::Float(f) => Some(PreviewVal::Float(-f)),
                        _ => None,
                    },
                    ast::UnaryOp::Pos => Some(val),
                    ast::UnaryOp::Not => Some(PreviewVal::Bool(!val.is_truthy())),
                    ast::UnaryOp::BitNot => match val {
                        PreviewVal::Int(i) => Some(PreviewVal::Int(!i)),
                        _ => None,
                    },
                }
            }
            ast::ExprKind::Binary { left, op, right } => {
                let l = self.eval_preview_expr_in_env(class_name, left, vars, depth)?;
                let r = self.eval_preview_expr_in_env(class_name, right, vars, depth)?;
                match (l, op, r) {
                    (PreviewVal::Str(a), ast::BinaryOp::Add, b) => {
                        Some(PreviewVal::Str(format!("{a}{}", b.to_display_string())))
                    }
                    (a, ast::BinaryOp::Add, PreviewVal::Str(b)) => {
                        Some(PreviewVal::Str(format!("{}{b}", a.to_display_string())))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Add, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Int(a + b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Sub, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Int(a - b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Mul, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Int(a * b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::FloorDiv, PreviewVal::Int(b)) if b != 0 => {
                        Some(PreviewVal::Int(a.div_euclid(b)))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Mod, PreviewVal::Int(b)) if b != 0 => {
                        Some(PreviewVal::Int(a.rem_euclid(b)))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Pow, PreviewVal::Int(b)) if b >= 0 => {
                        Some(PreviewVal::Int(a.saturating_pow(b as u32)))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Div, PreviewVal::Int(b)) if b != 0 => {
                        Some(PreviewVal::Float(a as f64 / b as f64))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Eq, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a == b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::NotEq, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a != b))
                    }
                    (PreviewVal::Str(a), ast::BinaryOp::Eq, PreviewVal::Str(b)) => {
                        Some(PreviewVal::Bool(a == b))
                    }
                    (PreviewVal::Str(a), ast::BinaryOp::NotEq, PreviewVal::Str(b)) => {
                        Some(PreviewVal::Bool(a != b))
                    }
                    (PreviewVal::Bool(a), ast::BinaryOp::Eq, PreviewVal::Bool(b)) => {
                        Some(PreviewVal::Bool(a == b))
                    }
                    (PreviewVal::Bool(a), ast::BinaryOp::NotEq, PreviewVal::Bool(b)) => {
                        Some(PreviewVal::Bool(a != b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Lt, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a < b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::LtEq, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a <= b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Gt, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a > b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::GtEq, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a >= b))
                    }
                    (a, ast::BinaryOp::Eq, b) => Some(PreviewVal::Bool(a.as_f64()? == b.as_f64()?)),
                    (a, ast::BinaryOp::NotEq, b) => {
                        Some(PreviewVal::Bool(a.as_f64()? != b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Lt, b) => Some(PreviewVal::Bool(a.as_f64()? < b.as_f64()?)),
                    (a, ast::BinaryOp::LtEq, b) => {
                        Some(PreviewVal::Bool(a.as_f64()? <= b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Gt, b) => Some(PreviewVal::Bool(a.as_f64()? > b.as_f64()?)),
                    (a, ast::BinaryOp::GtEq, b) => {
                        Some(PreviewVal::Bool(a.as_f64()? >= b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Add, b) => {
                        Some(PreviewVal::Float(a.as_f64()? + b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Sub, b) => {
                        Some(PreviewVal::Float(a.as_f64()? - b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Mul, b) => {
                        Some(PreviewVal::Float(a.as_f64()? * b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Pow, b) => {
                        Some(PreviewVal::Float(a.as_f64()?.powf(b.as_f64()?)))
                    }
                    (a, ast::BinaryOp::FloorDiv, b) => {
                        let denom = b.as_f64()?;
                        if denom == 0.0 {
                            None
                        } else {
                            Some(PreviewVal::Float((a.as_f64()? / denom).floor()))
                        }
                    }
                    (a, ast::BinaryOp::Div, b) => {
                        let denom = b.as_f64()?;
                        if denom == 0.0 {
                            None
                        } else {
                            Some(PreviewVal::Float(a.as_f64()? / denom))
                        }
                    }
                    _ => None,
                }
            }
            ast::ExprKind::BoolOp { op, left, right } => {
                let l = self.eval_preview_expr_in_env(class_name, left, vars, depth)?;
                match op {
                    ast::LogicalOp::And => {
                        if !l.is_truthy() {
                            Some(l)
                        } else {
                            self.eval_preview_expr_in_env(class_name, right, vars, depth)
                        }
                    }
                    ast::LogicalOp::Or => {
                        if l.is_truthy() {
                            Some(l)
                        } else {
                            self.eval_preview_expr_in_env(class_name, right, vars, depth)
                        }
                    }
                }
            }
            ast::ExprKind::IfExp { test, body, orelse } => {
                let cond = self.eval_preview_expr_in_env(class_name, test, vars, depth)?;
                if cond.is_truthy() {
                    self.eval_preview_expr_in_env(class_name, body, vars, depth)
                } else {
                    self.eval_preview_expr_in_env(class_name, orelse, vars, depth)
                }
            }
            ast::ExprKind::Call { callee, args, .. } => {
                let mut eval_args = Vec::with_capacity(args.len());
                for arg in args {
                    eval_args.push(self.eval_preview_expr_in_env(class_name, arg, vars, depth)?);
                }
                if let ast::ExprKind::Ident(fn_name) = &callee.kind {
                    if !self.local_functions.contains_key(fn_name)
                        && !self.class_methods.contains_key(fn_name)
                        && !self.module_functions.contains_key(fn_name)
                    {
                        match fn_name.as_str() {
                            "str" if eval_args.len() == 1 => {
                                return Some(PreviewVal::Str(eval_args[0].to_display_string()));
                            }
                            "int" if eval_args.len() == 1 => {
                                return match &eval_args[0] {
                                    PreviewVal::Int(i) => Some(PreviewVal::Int(*i)),
                                    PreviewVal::Float(f) => Some(PreviewVal::Int(*f as i64)),
                                    PreviewVal::Str(s) => {
                                        s.trim().parse::<i64>().ok().map(PreviewVal::Int)
                                    }
                                    PreviewVal::Bool(b) => Some(PreviewVal::Int(i64::from(*b))),
                                    _ => None,
                                };
                            }
                            "float" if eval_args.len() == 1 => {
                                return match &eval_args[0] {
                                    PreviewVal::Float(f) => Some(PreviewVal::Float(*f)),
                                    PreviewVal::Int(i) => Some(PreviewVal::Float(*i as f64)),
                                    PreviewVal::Str(s) => {
                                        s.trim().parse::<f64>().ok().map(PreviewVal::Float)
                                    }
                                    PreviewVal::Bool(b) => {
                                        Some(PreviewVal::Float(if *b { 1.0 } else { 0.0 }))
                                    }
                                    _ => None,
                                };
                            }
                            "bool" if eval_args.len() == 1 => {
                                return Some(PreviewVal::Bool(eval_args[0].is_truthy()));
                            }
                            "abs" if eval_args.len() == 1 => {
                                return match &eval_args[0] {
                                    PreviewVal::Int(i) => Some(PreviewVal::Int(i.abs())),
                                    PreviewVal::Float(f) => Some(PreviewVal::Float(f.abs())),
                                    _ => None,
                                };
                            }
                            "round" if eval_args.len() == 1 => {
                                return eval_args[0]
                                    .as_f64()
                                    .map(|f| PreviewVal::Int(f.round() as i64));
                            }
                            "min" if !eval_args.is_empty() => {
                                if eval_args.iter().all(|a| matches!(a, PreviewVal::Int(_))) {
                                    return eval_args
                                        .iter()
                                        .filter_map(PreviewVal::as_i64)
                                        .min()
                                        .map(PreviewVal::Int);
                                }
                                let mut best: Option<f64> = None;
                                for a in &eval_args {
                                    let f = a.as_f64()?;
                                    best = Some(best.map_or(f, |b| b.min(f)));
                                }
                                return best.map(PreviewVal::Float);
                            }
                            "max" if !eval_args.is_empty() => {
                                if eval_args.iter().all(|a| matches!(a, PreviewVal::Int(_))) {
                                    return eval_args
                                        .iter()
                                        .filter_map(PreviewVal::as_i64)
                                        .max()
                                        .map(PreviewVal::Int);
                                }
                                let mut best: Option<f64> = None;
                                for a in &eval_args {
                                    let f = a.as_f64()?;
                                    best = Some(best.map_or(f, |b| b.max(f)));
                                }
                                return best.map(PreviewVal::Float);
                            }
                            _ => {}
                        }
                    }
                    let func = self
                        .local_functions
                        .get(fn_name)
                        .or_else(|| self.class_methods.get(fn_name))
                        .or_else(|| self.module_functions.get(fn_name))?;
                    return self.eval_preview_function_in_env(
                        class_name,
                        func,
                        &eval_args,
                        depth + 1,
                    );
                }
                if let ast::ExprKind::Attribute { value, attr } = &callee.kind
                    && let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == class_name)
                {
                    let method = self.class_methods.get(attr)?;
                    return self.eval_preview_function_in_env(
                        class_name,
                        method,
                        &eval_args,
                        depth + 1,
                    );
                }
                None
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
enum LabelSpec {
    Text(String),
    Callable(ast::Expr),
}

#[derive(Debug, Clone)]
enum StyleSpec {
    Static(ControlStyle),
    DynamicExpr(ast::Expr),
    Callable(ast::Expr),
}

#[derive(Debug, Clone)]
pub(crate) enum PreviewVal {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Tuple(Vec<PreviewVal>),
    Prop(String),
}

impl PreviewVal {
    pub(crate) fn to_display_string(&self) -> String {
        match self {
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::Str(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::Tuple(items) => {
                let parts: Vec<String> = items.iter().map(Self::to_display_string).collect();
                format!("({})", parts.join(", "))
            }
            Self::Prop(name) => name.clone(),
        }
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub(crate) fn is_truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Int(i) => *i != 0,
            Self::Float(f) => *f != 0.0,
            Self::Str(s) => !s.is_empty(),
            Self::Tuple(items) => !items.is_empty(),
            Self::Prop(_) => true,
        }
    }

    pub(crate) fn from_const(c: &ConstValue) -> Self {
        match c {
            ConstValue::Int(i) => Self::Int(*i),
            ConstValue::Float(f) => Self::Float(*f),
            ConstValue::String(s) => Self::Str(s.clone()),
            ConstValue::Bool(b) => Self::Bool(*b),
            ConstValue::List(items) => Self::Tuple(items.iter().map(Self::from_const).collect()),
        }
    }
}

fn find_closest_control(target: &str, controls: &[DeclaredControl]) -> Vec<String> {
    let mut best: Option<(usize, String)> = None;
    for ctl in controls {
        let dist = levenshtein(target, &ctl.ident);
        if dist > 0 && dist <= 2 {
            match &best {
                Some((best_d, _)) if dist >= *best_d => {}
                _ => best = Some((dist, ctl.ident.clone())),
            }
        }
    }
    best.into_iter().map(|(_, s)| s).collect()
}

/// Parses a declarative `UI.*` expression (from `layout = ...`, `pages = ...`, or
/// `def get_control_layout(self, props): ...`) into solved [`PageLayout`]s.
pub fn parse_ui_layout_expr(
    class_name: &str,
    expr: &ast::Expr,
    controls: &[DeclaredControl],
    diagnostics: &mut Diagnostics,
    control_resolutions: &mut Vec<(Span, ControlId)>,
) -> Vec<PageLayout> {
    let env = UiResolveEnv::empty();
    parse_ui_layout_expr_with_env(
        class_name,
        expr,
        controls,
        &env,
        diagnostics,
        control_resolutions,
    )
}

pub fn parse_ui_layout_expr_with_env(
    class_name: &str,
    expr: &ast::Expr,
    controls: &[DeclaredControl],
    env: &UiResolveEnv<'_>,
    diagnostics: &mut Diagnostics,
    control_resolutions: &mut Vec<(Span, ControlId)>,
) -> Vec<PageLayout> {
    let mut ctx = UiParserCtx {
        class_name,
        controls,
        env,
        diagnostics,
        control_resolutions,
    };

    let page_specs = ctx.parse_top_level_pages(expr);
    page_specs.iter().map(UiPageSpec::solve).collect()
}

/// Checks whether an expression is a declarative `UI.*` call or list of `UI.*` calls.
pub fn is_ui_layout_expr(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Call { callee, .. } => is_ui_callee(callee).is_some(),
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
            !items.is_empty() && items.iter().all(is_ui_layout_expr)
        }
        _ => false,
    }
}

/// Substitutes local `var = UI.*(...)` variable bindings inside an expression tree.
pub fn substitute_ui_vars(expr: &ast::Expr, vars: &FxHashMap<String, ast::Expr>) -> ast::Expr {
    if vars.is_empty() {
        return expr.clone();
    }
    match &expr.kind {
        ast::ExprKind::Ident(name) => {
            if let Some(bound) = vars.get(name) {
                bound.clone()
            } else {
                expr.clone()
            }
        }
        ast::ExprKind::Call {
            callee,
            args,
            kwargs,
        } => ast::Expr {
            kind: ast::ExprKind::Call {
                callee: Box::new(substitute_ui_vars(callee, vars)),
                args: args.iter().map(|a| substitute_ui_vars(a, vars)).collect(),
                kwargs: kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), substitute_ui_vars(v, vars)))
                    .collect(),
            },
            span: expr.span,
        },
        ast::ExprKind::List(items) => ast::Expr {
            kind: ast::ExprKind::List(items.iter().map(|i| substitute_ui_vars(i, vars)).collect()),
            span: expr.span,
        },
        ast::ExprKind::Tuple(items) => ast::Expr {
            kind: ast::ExprKind::Tuple(items.iter().map(|i| substitute_ui_vars(i, vars)).collect()),
            span: expr.span,
        },
        ast::ExprKind::BoolOp { op, left, right } => ast::Expr {
            kind: ast::ExprKind::BoolOp {
                op: *op,
                left: Box::new(substitute_ui_vars(left, vars)),
                right: Box::new(substitute_ui_vars(right, vars)),
            },
            span: expr.span,
        },
        ast::ExprKind::IfExp { test, body, orelse } => ast::Expr {
            kind: ast::ExprKind::IfExp {
                test: Box::new(substitute_ui_vars(test, vars)),
                body: Box::new(substitute_ui_vars(body, vars)),
                orelse: Box::new(substitute_ui_vars(orelse, vars)),
            },
            span: expr.span,
        },
        _ => expr.clone(),
    }
}

/// Extracts a declarative `UI.*` expression from a `def get_control_layout(self, props):` method body.
///
/// Supports all common idioms:
/// - `return UI.Page(...)`
/// - `layout = {}; graphics = []; layout = UI.Page(...); return layout, graphics`
/// - `layout, graphics = UI.Page(...); return layout, graphics`
/// - Local variable decomposition (`hdr = UI.Header(...); grp = UI.GroupBox(...); return UI.Page("Main", hdr, grp)`)
/// - Bare `UI.Header(...)` / `UI.GroupBox(...)` statements or `graphics.append(UI.Header(...))`
pub fn extract_declarative_layout_from_method(body: &[ast::Stmt]) -> Option<ast::Expr> {
    let mut local_ui_vars: FxHashMap<String, ast::Expr> = FxHashMap::default();
    let mut top_ui_exprs: Vec<ast::Expr> = Vec::new();

    for stmt in body {
        match &stmt.kind {
            ast::StmtKind::Pass | ast::StmtKind::Global(_) | ast::StmtKind::Nonlocal(_) => {}
            ast::StmtKind::Expr(e) if matches!(e.kind, ast::ExprKind::String(_)) => {}
            ast::StmtKind::Expr(e) => {
                let sub = substitute_ui_vars(e, &local_ui_vars);
                if is_ui_layout_expr(&sub) {
                    top_ui_exprs.push(sub);
                } else {
                    let appended = extract_graphics_append(&sub)?;
                    top_ui_exprs.push(appended);
                }
            }
            ast::StmtKind::Assign { target, value, .. } => {
                if is_empty_layout_init(target, value) {
                    continue;
                }
                let sub_val = substitute_ui_vars(value, &local_ui_vars);
                if is_ui_layout_expr(&sub_val) {
                    if is_layout_target(target) {
                        top_ui_exprs.push(sub_val);
                    } else if let ast::ExprKind::Ident(var_name) = &target.kind {
                        local_ui_vars.insert(var_name.clone(), sub_val);
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            ast::StmtKind::Return(Some(ret_expr)) => {
                let sub_ret = substitute_ui_vars(ret_expr, &local_ui_vars);
                if is_ui_layout_expr(&sub_ret) {
                    top_ui_exprs.push(sub_ret);
                } else if is_layout_return_expr(ret_expr) {
                    // `return layout, graphics` after `layout = UI.Page(...)`
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }

    if top_ui_exprs.is_empty() {
        None
    } else if top_ui_exprs.len() == 1 {
        Some(top_ui_exprs.remove(0))
    } else {
        let span = top_ui_exprs[0].span;
        Some(ast::Expr {
            kind: ast::ExprKind::List(top_ui_exprs),
            span,
        })
    }
}

/// Scans statements inside a hybrid `get_control_layout` method (e.g., one with `if` branches or
/// procedural table assignments alongside `UI.*`) so `UI.*` nodes are validated and solved for preview.
pub fn collect_inline_ui_pages_from_stmts(
    class_name: &str,
    stmts: &[ast::Stmt],
    controls: &[DeclaredControl],
    env: &mut UiResolveEnv<'_>,
    diagnostics: &mut Diagnostics,
    control_resolutions: &mut Vec<(Span, ControlId)>,
    local_ui_vars: &mut FxHashMap<String, ast::Expr>,
) -> Vec<PageLayout> {
    let mut pages = Vec::new();
    for stmt in stmts {
        match &stmt.kind {
            ast::StmtKind::FunctionDef(f) => {
                env.local_functions.insert(f.name.clone(), f.clone());
            }
            ast::StmtKind::Assign { target, value, .. } => {
                let sub_val = substitute_ui_vars(value, local_ui_vars);
                if is_ui_layout_expr(&sub_val) {
                    if !is_layout_target(target)
                        && let ast::ExprKind::Ident(var_name) = &target.kind
                    {
                        local_ui_vars.insert(var_name.clone(), sub_val.clone());
                    }
                    pages.extend(parse_ui_layout_expr_with_env(
                        class_name,
                        &sub_val,
                        controls,
                        env,
                        diagnostics,
                        control_resolutions,
                    ));
                } else if let ast::ExprKind::Ident(var_name) = &target.kind
                    && let Some(cval) = eval_constant_with_env(value, &env.constants)
                        .or_else(|| eval_constant(value))
                {
                    env.constants.insert(var_name.clone(), cval);
                }
            }
            ast::StmtKind::Expr(e) => {
                let sub = substitute_ui_vars(e, local_ui_vars);
                if is_ui_layout_expr(&sub) {
                    pages.extend(parse_ui_layout_expr_with_env(
                        class_name,
                        &sub,
                        controls,
                        env,
                        diagnostics,
                        control_resolutions,
                    ));
                } else if let Some(appended) = extract_graphics_append(&sub) {
                    pages.extend(parse_ui_layout_expr_with_env(
                        class_name,
                        &appended,
                        controls,
                        env,
                        diagnostics,
                        control_resolutions,
                    ));
                }
            }
            ast::StmtKind::Return(Some(ret_expr)) => {
                let sub_ret = substitute_ui_vars(ret_expr, local_ui_vars);
                if is_ui_layout_expr(&sub_ret) {
                    pages.extend(parse_ui_layout_expr_with_env(
                        class_name,
                        &sub_ret,
                        controls,
                        env,
                        diagnostics,
                        control_resolutions,
                    ));
                }
            }
            ast::StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                pages.extend(collect_inline_ui_pages_from_stmts(
                    class_name,
                    then_branch,
                    controls,
                    env,
                    diagnostics,
                    control_resolutions,
                    local_ui_vars,
                ));
                pages.extend(collect_inline_ui_pages_from_stmts(
                    class_name,
                    else_branch,
                    controls,
                    env,
                    diagnostics,
                    control_resolutions,
                    local_ui_vars,
                ));
            }
            _ => {}
        }
    }
    pages
}

pub fn extract_graphics_append(expr: &ast::Expr) -> Option<ast::Expr> {
    if let ast::ExprKind::Call { callee, args, .. } = &expr.kind
        && let ast::ExprKind::Attribute { value, attr } = &callee.kind
        && let ast::ExprKind::Ident(obj) = &value.kind
        && obj == "graphics"
        && matches!(attr.as_str(), "append" | "extend")
        && let Some(first) = args.first()
        && is_ui_layout_expr(first)
    {
        return Some(first.clone());
    }
    None
}

fn is_empty_layout_init(target: &ast::Expr, value: &ast::Expr) -> bool {
    let is_empty_collection = match &value.kind {
        ast::ExprKind::Dict(pairs) => pairs.is_empty(),
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => items.is_empty(),
        _ => false,
    };
    is_empty_collection && is_layout_target(target)
}

pub fn is_layout_target(target: &ast::Expr) -> bool {
    match &target.kind {
        ast::ExprKind::Ident(name) => matches!(name.as_str(), "layout" | "graphics" | "pages"),
        ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
            !items.is_empty() && items.iter().all(is_layout_target)
        }
        _ => false,
    }
}

fn is_layout_return_expr(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Ident(name) => matches!(name.as_str(), "layout" | "graphics" | "pages"),
        ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
            !items.is_empty() && items.iter().all(is_layout_return_expr)
        }
        _ => false,
    }
}

fn is_page_or_pages_expr(expr: &ast::Expr) -> bool {
    if let ast::ExprKind::Call { callee, .. } = &expr.kind
        && let Some(tag) = is_ui_callee(callee)
    {
        return matches!(tag, "Page" | "Pages");
    }
    false
}

fn is_ui_callee(callee: &ast::Expr) -> Option<&str> {
    if let ast::ExprKind::Attribute { value, attr } = &callee.kind
        && let ast::ExprKind::Ident(obj) = &value.kind
        && obj == "UI"
    {
        return Some(attr.as_str());
    }
    None
}

struct UiParserCtx<'a> {
    class_name: &'a str,
    controls: &'a [DeclaredControl],
    env: &'a UiResolveEnv<'a>,
    diagnostics: &'a mut Diagnostics,
    control_resolutions: &'a mut Vec<(Span, ControlId)>,
}

impl<'a> UiParserCtx<'a> {
    fn parse_top_level_pages(&mut self, expr: &ast::Expr) -> Vec<UiPageSpec> {
        match &expr.kind {
            ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
                let mut pages = Vec::new();
                let mut loose_children = Vec::new();
                for item in items {
                    if is_page_or_pages_expr(item) {
                        if !loose_children.is_empty() {
                            pages.push(UiPageSpec {
                                name: "Main".to_string(),
                                children: std::mem::take(&mut loose_children),
                                padding: 16.0,
                                gap: 12.0,
                                width: None,
                            });
                        }
                        pages.extend(self.parse_top_level_pages(item));
                    } else {
                        loose_children.extend(self.parse_node_expr(item));
                    }
                }
                if !loose_children.is_empty() {
                    pages.push(UiPageSpec {
                        name: "Main".to_string(),
                        children: loose_children,
                        padding: 16.0,
                        gap: 12.0,
                        width: None,
                    });
                }
                pages
            }
            ast::ExprKind::Call {
                callee,
                args,
                kwargs,
            } => {
                if let Some(tag) = is_ui_callee(callee) {
                    match tag {
                        "Pages" => {
                            let mut pages = Vec::new();
                            for arg in args {
                                pages.extend(self.parse_top_level_pages(arg));
                            }
                            if let Some((_, ch_expr)) =
                                kwargs.iter().find(|(k, _)| k == "children" || k == "pages")
                            {
                                pages.extend(self.parse_top_level_pages(ch_expr));
                            }
                            pages
                        }
                        "Page" => vec![self.parse_page_call(args, kwargs, expr.span)],
                        _ => {
                            // A top-level UI.GroupBox / UI.Col / UI.Row without an outer UI.Page
                            // is automatically wrapped in a default "Main" page.
                            let children = self.parse_node_expr(expr);
                            vec![UiPageSpec {
                                name: "Main".to_string(),
                                children,
                                padding: 16.0,
                                gap: 12.0,
                                width: None,
                            }]
                        }
                    }
                } else {
                    self.diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: "layout".to_string(),
                            message: "Expected a 'UI.Page(...)', 'UI.GroupBox(...)', 'UI.Row(...)', or 'UI.Col(...)' expression".to_string(),
                        },
                        expr.span,
                    );
                    Vec::new()
                }
            }
            _ => {
                self.diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: "layout".to_string(),
                        message: "Expected a 'UI.*' component tree".to_string(),
                    },
                    expr.span,
                );
                Vec::new()
            }
        }
    }

    fn parse_page_call(
        &mut self,
        args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
        _span: Span,
    ) -> UiPageSpec {
        let mut name = self
            .kwarg_str(kwargs, "title")
            .or_else(|| self.kwarg_str(kwargs, "name"))
            .unwrap_or_else(|| "Main".to_string());

        let mut child_start_idx = 0;
        if let Some(first) = args.first()
            && !is_ui_node_or_control_expr(first, self.controls, self.class_name)
            && let Some(s) = self.eval_str(first)
        {
            name = s;
            child_start_idx = 1;
        }

        let padding = self.kwarg_f64(kwargs, "padding").unwrap_or(16.0);
        let gap = self.kwarg_f64(kwargs, "gap").unwrap_or(12.0);
        let width = self.kwarg_f64(kwargs, "width");

        let children = self.collect_children(&args[child_start_idx..], kwargs);

        UiPageSpec {
            name,
            children,
            padding,
            gap,
            width,
        }
    }

    fn collect_children(
        &mut self,
        positional_args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
    ) -> Vec<UiNode> {
        let mut nodes = Vec::new();
        for arg in positional_args {
            nodes.extend(self.parse_node_expr(arg));
        }
        if let Some((_, children_expr)) = kwargs.iter().find(|(k, _)| k == "children") {
            nodes.extend(self.parse_node_expr(children_expr));
        }
        nodes
    }

    fn parse_node_expr(&mut self, expr: &ast::Expr) -> Vec<UiNode> {
        match &expr.kind {
            ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
                let mut out = Vec::new();
                for item in items {
                    out.extend(self.parse_node_expr(item));
                }
                out
            }

            ast::ExprKind::Call {
                callee,
                args,
                kwargs,
            } => {
                let Some(tag) = is_ui_callee(callee) else {
                    self.diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: "UI".to_string(),
                            message: "Expected a 'UI.*' component or declared control reference"
                                .to_string(),
                        },
                        expr.span,
                    );
                    return Vec::new();
                };

                match tag {
                    "Row" | "HStack" => {
                        let mut label = self
                            .kwarg_str(kwargs, "label")
                            .or_else(|| self.kwarg_str(kwargs, "title"))
                            .or_else(|| self.kwarg_str(kwargs, "text"));
                        let mut child_start = 0;
                        if let Some(first) = args.first()
                            && !is_ui_node_or_control_expr(first, self.controls, self.class_name)
                            && let Some(s) = self.eval_str(first)
                        {
                            label = Some(s);
                            child_start = 1;
                        }
                        let gap = self.kwarg_f64(kwargs, "gap").unwrap_or(8.0);
                        let padding = self.kwarg_f64(kwargs, "padding").unwrap_or(0.0);
                        let mut children = Vec::new();
                        if let Some(lbl_text) = label {
                            let lbl_w = self.kwarg_f64(kwargs, "label_width");
                            let lbl_fs = self.kwarg_u32(kwargs, "label_font_size").unwrap_or(11);
                            children.push(UiNode::Label {
                                text: lbl_text,
                                width: lbl_w,
                                height: 22.0,
                                font_size: lbl_fs,
                                h_align: "Left".to_string(),
                                color: None,
                            });
                        }
                        children.extend(self.collect_children(&args[child_start..], kwargs));
                        vec![UiNode::Row {
                            children,
                            gap,
                            padding,
                        }]
                    }

                    "Col" | "Column" | "VStack" | "Div" => {
                        let mut label = self
                            .kwarg_str(kwargs, "label")
                            .or_else(|| self.kwarg_str(kwargs, "title"))
                            .or_else(|| self.kwarg_str(kwargs, "text"));
                        let mut child_start = 0;
                        if let Some(first) = args.first()
                            && !is_ui_node_or_control_expr(first, self.controls, self.class_name)
                            && let Some(s) = self.eval_str(first)
                        {
                            label = Some(s);
                            child_start = 1;
                        }
                        let gap = self.kwarg_f64(kwargs, "gap").unwrap_or(8.0);
                        let padding = self.kwarg_f64(kwargs, "padding").unwrap_or(0.0);
                        let mut children = Vec::new();
                        if let Some(lbl_text) = label {
                            let lbl_w = self.kwarg_f64(kwargs, "label_width");
                            let lbl_fs = self.kwarg_u32(kwargs, "label_font_size").unwrap_or(11);
                            children.push(UiNode::Label {
                                text: lbl_text,
                                width: lbl_w,
                                height: 18.0,
                                font_size: lbl_fs,
                                h_align: "Left".to_string(),
                                color: None,
                            });
                        }
                        children.extend(self.collect_children(&args[child_start..], kwargs));
                        vec![UiNode::Col {
                            children,
                            gap,
                            padding,
                        }]
                    }

                    "GroupBox" | "Section" | "Card" | "Panel" => {
                        let mut title = self
                            .kwarg_str(kwargs, "title")
                            .or_else(|| self.kwarg_str(kwargs, "text"))
                            .unwrap_or_default();
                        let mut child_start = 0;
                        if let Some(first) = args.first()
                            && !is_ui_node_or_control_expr(first, self.controls, self.class_name)
                            && let Some(s) = self.eval_str(first)
                        {
                            title = s;
                            child_start = 1;
                        }

                        let dir_str = self
                            .kwarg_str(kwargs, "direction")
                            .unwrap_or_else(|| "col".to_string());
                        let direction = match dir_str.to_ascii_lowercase().as_str() {
                            "row" | "horizontal" | "hstack" => FlexDirection::Row,
                            _ => FlexDirection::Column,
                        };
                        let gap = self.kwarg_f64(kwargs, "gap").unwrap_or(10.0);
                        let padding = self.kwarg_f64(kwargs, "padding").unwrap_or(12.0);
                        let width = self.kwarg_f64(kwargs, "width");
                        let height = self.kwarg_f64(kwargs, "height");
                        let color = self
                            .kwarg_color(kwargs, "color")
                            .or_else(|| self.kwarg_color(kwargs, "fill"));
                        let stroke = self
                            .kwarg_u32(kwargs, "stroke")
                            .or_else(|| self.kwarg_u32(kwargs, "stroke_width"))
                            .unwrap_or(1);

                        let children = self.collect_children(&args[child_start..], kwargs);
                        vec![UiNode::GroupBox {
                            title,
                            children,
                            direction,
                            gap,
                            padding,
                            width,
                            height,
                            color,
                            stroke,
                        }]
                    }

                    "Header" | "H1" | "H2" => {
                        let text = args
                            .first()
                            .and_then(|e| self.eval_str(e))
                            .or_else(|| self.kwarg_str(kwargs, "text"))
                            .or_else(|| self.kwarg_str(kwargs, "title"))
                            .unwrap_or_default();
                        let width = self.kwarg_f64(kwargs, "width");
                        let height = self.kwarg_f64(kwargs, "height").unwrap_or(28.0);
                        let font_size = self.kwarg_u32(kwargs, "font_size").unwrap_or(14);
                        let color = self.kwarg_color(kwargs, "color");
                        vec![UiNode::Header {
                            text,
                            width,
                            height,
                            font_size,
                            color,
                        }]
                    }

                    "Label" | "Text" => {
                        let text = args
                            .first()
                            .and_then(|e| self.eval_str(e))
                            .or_else(|| self.kwarg_str(kwargs, "text"))
                            .unwrap_or_default();
                        let width = self.kwarg_f64(kwargs, "width");
                        let height = self.kwarg_f64(kwargs, "height").unwrap_or(22.0);
                        let font_size = self.kwarg_u32(kwargs, "font_size").unwrap_or(12);
                        let h_align = self
                            .kwarg_str(kwargs, "align")
                            .unwrap_or_else(|| "Left".to_string());
                        let color = self.kwarg_color(kwargs, "color");
                        vec![UiNode::Label {
                            text,
                            width,
                            height,
                            font_size,
                            h_align,
                            color,
                        }]
                    }

                    "Spacer" => {
                        let width = args
                            .first()
                            .and_then(|e| self.eval_f64(e))
                            .or_else(|| self.kwarg_f64(kwargs, "width"))
                            .or_else(|| self.kwarg_f64(kwargs, "size"))
                            .unwrap_or(12.0);
                        let height = args
                            .get(1)
                            .and_then(|e| self.eval_f64(e))
                            .or_else(|| self.kwarg_f64(kwargs, "height"))
                            .or_else(|| self.kwarg_f64(kwargs, "size"))
                            .unwrap_or(12.0);
                        vec![UiNode::Spacer { width, height }]
                    }

                    "Field" => self.parse_field_call(args, kwargs, expr.span, false),

                    "Control" => self.parse_control_call(args, kwargs, expr.span, false),

                    "Grid" => self.parse_grid_call(args, kwargs, expr.span),

                    other => {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: format!("UI.{other}"),
                                message: format!(
                                    "Unknown UI component 'UI.{other}'. Expected Page, GroupBox, Row, Col, Grid, Header, Label, Field, Control, or Spacer"
                                ),
                            },
                            expr.span,
                        );
                        Vec::new()
                    }
                }
            }

            // Direct control reference passed as a child node: e.g. `UI.Row(self.Gain, self.Mute)` or `UI.Row(Gain, Mute)`
            _ => {
                if let Some((ctl, sub_idx)) = self.resolve_control_ref(expr) {
                    self.build_control_nodes(ctl, sub_idx, &[], None, expr.span, false)
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn parse_label_spec(&mut self, expr: &ast::Expr, arg_name: &str) -> Option<LabelSpec> {
        if let Some(s) = eval_constant_with_env(expr, &self.env.constants)
            .or_else(|| eval_constant(expr))
            .and_then(|c| c.as_str().map(String::from))
        {
            return Some(LabelSpec::Text(s));
        }

        match &expr.kind {
            ast::ExprKind::Lambda { .. } => Some(LabelSpec::Callable(expr.clone())),
            ast::ExprKind::Ident(_) | ast::ExprKind::Attribute { .. } => {
                Some(LabelSpec::Callable(expr.clone()))
            }
            _ => {
                self.diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: arg_name.to_string(),
                        message: format!(
                            "'{arg_name}' must be a string literal (e.g. \"Input {{i}}\") or a function/lambda taking the 0-based index"
                        ),
                    },
                    expr.span,
                );
                None
            }
        }
    }

    fn parse_style_kwarg(&self, kwargs: &[(String, ast::Expr)]) -> Option<StyleSpec> {
        let (_, expr) = kwargs.iter().find(|(k, _)| k == "style")?;
        if let Some(s) = eval_constant_with_env(expr, &self.env.constants)
            .or_else(|| eval_constant(expr))
            .and_then(|c| c.as_str().map(String::from))
        {
            return Some(StyleSpec::Static(ControlStyle::from_name(&s)));
        }

        let norm_expr = normalize_style_string_literals(expr);
        match &norm_expr.kind {
            ast::ExprKind::Lambda { .. } => Some(StyleSpec::Callable(norm_expr)),
            ast::ExprKind::Ident(name)
                if self.env.local_functions.contains_key(name)
                    || self.env.class_methods.contains_key(name)
                    || self.env.module_functions.contains_key(name) =>
            {
                Some(StyleSpec::Callable(norm_expr))
            }
            ast::ExprKind::Attribute { value, attr }
                if matches!(
                    &value.kind,
                    ast::ExprKind::Ident(obj) if obj == "self" || obj == self.class_name
                ) && self.env.class_methods.contains_key(attr) =>
            {
                Some(StyleSpec::Callable(norm_expr))
            }
            _ => Some(StyleSpec::DynamicExpr(norm_expr)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_style_for_item(
        &self,
        style_spec: &Option<StyleSpec>,
        ctl: &DeclaredControl,
        r_idx: Option<i64>,
        c_idx: Option<i64>,
        flat_idx: i64,
        r_lua: Option<&str>,
        c_lua: Option<&str>,
        flat_lua: &str,
    ) -> (ControlStyle, Option<String>) {
        let default_style = ControlStyle::default_for_kind(&ctl.def.kind);
        match style_spec {
            None => (default_style, None),
            Some(StyleSpec::Static(s)) => (s.clone(), None),
            Some(StyleSpec::DynamicExpr(expr)) => {
                let mut vars = FxHashMap::default();
                for (name, val) in &self.env.constants {
                    vars.insert(name.clone(), PreviewVal::from_const(val));
                }
                vars.insert("i".to_string(), PreviewVal::Int(flat_idx));
                if let Some(r) = r_idx {
                    vars.insert("r".to_string(), PreviewVal::Int(r));
                }
                if let Some(c) = c_idx {
                    vars.insert("c".to_string(), PreviewVal::Int(c));
                }
                let preview_style = self
                    .eval_preview_expr(expr, &vars, 0)
                    .map(|v| ControlStyle::from_name(&v.to_display_string()))
                    .unwrap_or(default_style);
                let mut local_map = FxHashMap::default();
                local_map.insert("i".to_string(), format!("({flat_lua})"));
                if let Some(rl) = r_lua {
                    local_map.insert("r".to_string(), format!("({rl})"));
                }
                if let Some(cl) = c_lua {
                    local_map.insert("c".to_string(), format!("({cl})"));
                }
                let lua = self.lower_ui_expr_to_lua(expr, &local_map);
                (preview_style, Some(lua))
            }
            Some(StyleSpec::Callable(fn_expr)) => {
                let arity = self.callable_arity(fn_expr);
                let preview_str = if arity == 0 {
                    self.eval_callable_0_preview(fn_expr)
                } else if arity >= 2
                    && let (Some(r), Some(c)) = (r_idx, c_idx)
                {
                    self.eval_callable_2d_preview(fn_expr, r, c)
                } else {
                    self.eval_callable_preview(fn_expr, flat_idx)
                };
                let preview_style = preview_str
                    .map(|s| ControlStyle::from_name(&s))
                    .unwrap_or(default_style);
                let lua = if arity == 0 {
                    self.lower_callable_0_lua(fn_expr)
                } else if arity >= 2
                    && let (Some(rl), Some(cl)) = (r_lua, c_lua)
                {
                    self.lower_callable_2d_lua(fn_expr, rl, cl)
                } else {
                    self.lower_callable_lua(fn_expr, flat_lua)
                };
                (preview_style, Some(lua))
            }
        }
    }

    fn parse_grid_call(
        &mut self,
        args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
        span: Span,
    ) -> Vec<UiNode> {
        if args.len() >= 2 {
            self.parse_field_call(args, kwargs, span, true)
        } else {
            self.parse_control_call(args, kwargs, span, true)
        }
    }

    fn parse_field_call(
        &mut self,
        args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
        span: Span,
        force_grid: bool,
    ) -> Vec<UiNode> {
        // UI.Field("Master Gain", self.Gain, ...) or UI.Field(fn, self.Gain, ...) or UI.Field(label=..., control=self.Gain, ...)
        let label_expr = args
            .first()
            .or_else(|| kwargs.iter().find(|(k, _)| k == "label").map(|(_, v)| v));
        let label_spec = label_expr.and_then(|e| self.parse_label_spec(e, "UI.Field label"));

        let ctl_expr = args
            .get(1)
            .or_else(|| kwargs.iter().find(|(k, _)| k == "control").map(|(_, v)| v));

        let Some(ctl_expr) = ctl_expr else {
            self.diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: "UI.Field".to_string(),
                    message:
                        "UI.Field requires a label (string or function) and a declared control: UI.Field(\"Label\", self.ControlName)"
                            .to_string(),
                },
                span,
            );
            return Vec::new();
        };

        if let Some((ctl, sub_idx)) = self.resolve_control_ref(ctl_expr) {
            self.build_control_nodes(ctl, sub_idx, kwargs, label_spec, span, force_grid)
        } else {
            Vec::new()
        }
    }

    fn parse_control_call(
        &mut self,
        args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
        span: Span,
        force_grid: bool,
    ) -> Vec<UiNode> {
        let ctl_expr = args
            .first()
            .or_else(|| kwargs.iter().find(|(k, _)| k == "control").map(|(_, v)| v));

        let Some(ctl_expr) = ctl_expr else {
            self.diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: "UI.Control".to_string(),
                    message:
                        "UI.Control requires a declared control reference: UI.Control(self.ControlName)"
                            .to_string(),
                },
                span,
            );
            return Vec::new();
        };

        let explicit_label = kwargs
            .iter()
            .find(|(k, _)| k == "label")
            .and_then(|(_, v)| self.parse_label_spec(v, "UI.Control label"));
        if let Some((ctl, sub_idx)) = self.resolve_control_ref(ctl_expr) {
            self.build_control_nodes(ctl, sub_idx, kwargs, explicit_label, span, force_grid)
        } else {
            Vec::new()
        }
    }

    fn build_control_nodes(
        &mut self,
        ctl: &DeclaredControl,
        subscript_idx: Option<usize>,
        kwargs: &[(String, ast::Expr)],
        label_override: Option<LabelSpec>,
        span: Span,
        force_grid: bool,
    ) -> Vec<UiNode> {
        let style_spec = self.parse_style_kwarg(kwargs);
        let type_property = if style_spec.is_none() {
            ctl.def
                .generic_spec
                .as_ref()
                .map(|g| g.property_name.clone())
        } else {
            None
        };
        let (style, base_style_lua_expr) = self.resolve_style_for_item(
            &style_spec,
            ctl,
            Some(0),
            Some(0),
            0,
            Some("0"),
            Some("0"),
            "0",
        );

        let label_pos_str = self
            .kwarg_str(kwargs, "label_pos")
            .unwrap_or_else(|| "top".to_string());
        let label_width = self.kwarg_f64(kwargs, "label_width").unwrap_or(88.0);
        let label_placement = match label_pos_str.to_ascii_lowercase().as_str() {
            "left" | "inline" | "side" => LabelPlacement::Left { width: label_width },
            _ => LabelPlacement::Top,
        };

        let pretty_spec = kwargs
            .iter()
            .find(|(k, _)| k == "pretty_name")
            .and_then(|(_, v)| self.parse_label_spec(v, "pretty_name"));
        let legend = self.kwarg_str(kwargs, "legend");
        let size_tuple = self.kwarg_size(kwargs, "size");
        let width = self
            .kwarg_f64(kwargs, "width")
            .or_else(|| size_tuple.map(|(w, _)| w));
        let height = self
            .kwarg_f64(kwargs, "height")
            .or_else(|| size_tuple.map(|(_, h)| h));
        let color = self.kwarg_color(kwargs, "color");
        let margin = self.kwarg_f64(kwargs, "margin");
        let read_only = self.kwarg_bool(kwargs, "read_only").unwrap_or(false);
        let invisible = self.kwarg_bool(kwargs, "invisible").unwrap_or(false);

        let kwarg_idx = kwargs
            .iter()
            .find(|(k, _)| k == "index")
            .and_then(|(_, v)| self.eval_i64(v));

        let effective_idx = if let Some(i) = kwarg_idx {
            if i < 0 || (ctl.def.count_property.is_none() && (i as usize) >= ctl.def.count) {
                self.diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: ctl.ident.clone(),
                        message: format!(
                            "Control '{}' has count {}; index {} is out of bounds (expected 0..{})",
                            ctl.ident,
                            ctl.def.count,
                            i,
                            ctl.def.count.saturating_sub(1)
                        ),
                    },
                    span,
                );
                return Vec::new();
            }
            Some(i as usize)
        } else {
            subscript_idx
        };

        // Static single-channel control (`count == 1` and not property-driven)
        if ctl.def.count == 1 && ctl.def.count_property.is_none() {
            let (resolved_label, label_lua_expr) = match &label_override {
                Some(LabelSpec::Text(s)) => (Some(s.clone()), None),
                Some(LabelSpec::Callable(fn_expr)) => {
                    let preview = self
                        .eval_callable_preview(fn_expr, 0)
                        .unwrap_or_else(|| ctl.def.name.clone());
                    let lua = self.lower_callable_lua(fn_expr, "0");
                    (Some(preview), Some(lua))
                }
                None => (None, None),
            };
            let (resolved_pretty, pretty_lua_expr) = match &pretty_spec {
                Some(LabelSpec::Text(s)) => (Some(s.clone()), None),
                Some(LabelSpec::Callable(fn_expr)) => {
                    let preview = self
                        .eval_callable_preview(fn_expr, 0)
                        .unwrap_or_else(|| ctl.def.name.clone());
                    let lua = self.lower_callable_lua(fn_expr, "0");
                    (Some(preview), Some(lua))
                }
                None => (
                    resolved_label
                        .clone()
                        .or_else(|| Some(ctl.def.name.clone())),
                    label_lua_expr.clone(),
                ),
            };
            return vec![UiNode::Control(UiControlSpec {
                control_name: ctl.def.name.clone(),
                style,
                label: resolved_label,
                label_placement,
                pretty_name: resolved_pretty,
                legend,
                width,
                height,
                color,
                margin,
                read_only,
                invisible,
                type_property: type_property.clone(),
                count_property: None,
                base_control_name: None,
                channel_index: None,
                dynamic_label_template: None,
                dynamic_pretty_template: None,
                dynamic_legend_template: None,
                label_lua_expr,
                pretty_lua_expr,
                style_lua_expr: base_style_lua_expr,
                grid: None,
                is_dynamic_repeat_head: false,
                skip_lua_emit: false,
            })];
        }

        // Multi-channel or property-counted control with a specific 0-based channel index (`ChGain[0]`)
        if let Some(idx) = effective_idx {
            let ch = idx + 1;
            let idx_str = idx.to_string();
            let qsys_key = format!("{} {}", ctl.def.name, ch);
            let (idx_style, idx_style_lua) = self.resolve_style_for_item(
                &style_spec,
                ctl,
                None,
                None,
                idx as i64,
                None,
                None,
                &idx_str,
            );
            let (ch_label, label_lua_expr) = match &label_override {
                Some(LabelSpec::Text(lbl)) => (Some(format_channel_template(lbl, ch)), None),
                Some(LabelSpec::Callable(fn_expr)) => {
                    let preview = self
                        .eval_callable_preview(fn_expr, idx as i64)
                        .unwrap_or_else(|| qsys_key.clone());
                    let lua = self.lower_callable_lua(fn_expr, &idx_str);
                    (Some(preview), Some(lua))
                }
                None => (None, None),
            };
            let (resolved_pretty, pretty_lua_expr) = match &pretty_spec {
                Some(LabelSpec::Text(p)) => (Some(format_channel_template(p, ch)), None),
                Some(LabelSpec::Callable(fn_expr)) => {
                    let preview = self
                        .eval_callable_preview(fn_expr, idx as i64)
                        .unwrap_or_else(|| qsys_key.clone());
                    let lua = self.lower_callable_lua(fn_expr, &idx_str);
                    (Some(preview), Some(lua))
                }
                None => (
                    ch_label.clone().or_else(|| Some(qsys_key.clone())),
                    label_lua_expr.clone(),
                ),
            };
            return vec![UiNode::Control(UiControlSpec {
                control_name: qsys_key,
                style: idx_style,
                label: ch_label,
                label_placement,
                pretty_name: resolved_pretty,
                legend,
                width,
                height,
                color,
                margin,
                read_only,
                invisible,
                type_property: type_property.clone(),
                count_property: ctl.def.count_property.clone(),
                base_control_name: Some(ctl.def.name.clone()),
                channel_index: Some(ch),
                dynamic_label_template: None,
                dynamic_pretty_template: None,
                dynamic_legend_template: None,
                label_lua_expr,
                pretty_lua_expr,
                style_lua_expr: idx_style_lua,
                grid: None,
                is_dynamic_repeat_head: false,
                skip_lua_emit: false,
            })];
        }

        // Check if 2D grid layout is requested or inferred for this multi-channel control
        let dir_kwarg = self
            .kwarg_str(kwargs, "direction")
            .map(|s| s.to_ascii_lowercase());
        let is_dir_grid = matches!(dir_kwarg.as_deref(), Some("grid" | "2d" | "matrix"));
        let is_dir_1d = matches!(
            dir_kwarg.as_deref(),
            Some("row" | "horizontal" | "hstack" | "col" | "column" | "vertical" | "vstack")
        );
        let ui_rows = kwargs
            .iter()
            .find(|(k, _)| k == "rows")
            .and_then(|(_, v)| self.parse_ui_dim_kwarg(v));
        let ui_cols = kwargs
            .iter()
            .find(|(k, _)| k == "cols" || k == "columns")
            .and_then(|(_, v)| self.parse_ui_dim_kwarg(v));
        let explicit_row_label_spec = kwargs
            .iter()
            .find(|(k, _)| matches!(k.as_str(), "row_label" | "track_label" | "y_label"))
            .and_then(|(_, v)| self.parse_label_spec(v, "row_label"));
        let explicit_row_labels = self
            .kwarg_str_list(kwargs, "row_labels")
            .or_else(|| self.kwarg_str_list(kwargs, "y_labels"));
        let explicit_col_label_spec = kwargs
            .iter()
            .find(|(k, _)| matches!(k.as_str(), "col_label" | "column_label" | "x_label"))
            .and_then(|(_, v)| self.parse_label_spec(v, "col_label"));
        let explicit_col_labels = self
            .kwarg_str_list(kwargs, "col_labels")
            .or_else(|| self.kwarg_str_list(kwargs, "column_labels"))
            .or_else(|| self.kwarg_str_list(kwargs, "x_labels"));
        let explicit_cell_label_spec = kwargs
            .iter()
            .find(|(k, _)| k == "cell_label")
            .and_then(|(_, v)| self.parse_label_spec(v, "cell_label"));
        let label_has_2d_vars = matches!(
            &label_override,
            Some(LabelSpec::Text(s)) if has_2d_template_vars(s)
        ) || matches!(
            &pretty_spec,
            Some(LabelSpec::Text(s)) if has_2d_template_vars(s)
        );
        let label_is_2d_callable = matches!(
            &label_override,
            Some(LabelSpec::Callable(fn_expr)) if self.callable_arity(fn_expr) >= 2
        ) || matches!(
            &pretty_spec,
            Some(LabelSpec::Callable(fn_expr)) if self.callable_arity(fn_expr) >= 2
        );
        let has_explicit_2d_ui_opts = force_grid
            || is_dir_grid
            || ui_rows.is_some()
            || ui_cols.is_some()
            || explicit_row_label_spec.is_some()
            || explicit_row_labels.is_some()
            || explicit_col_label_spec.is_some()
            || explicit_col_labels.is_some()
            || label_has_2d_vars
            || label_is_2d_callable;
        let use_2d_grid = has_explicit_2d_ui_opts || (ctl.def.explicit_2d && !is_dir_1d);

        if use_2d_grid {
            let (preview_rows, rows_property) = if let Some((r, rp)) = &ui_rows {
                ((*r).max(1), rp.clone())
            } else if let Some(r) = ctl.def.rows {
                (r.max(1), ctl.def.rows_property.clone())
            } else if let Some((c, None)) = &ui_cols {
                let c_max = (*c).max(1);
                (ctl.def.count.max(1).div_ceil(c_max), None)
            } else {
                (1, None)
            };

            let (preview_cols, cols_property) = if let Some((c, cp)) = &ui_cols {
                ((*c).max(1), cp.clone())
            } else if let Some(c) = ctl.def.cols {
                (c.max(1), ctl.def.cols_property.clone())
            } else if let Some((r, None)) = &ui_rows {
                let r_max = (*r).max(1);
                (ctl.def.count.max(1).div_ceil(r_max), None)
            } else {
                (ctl.def.count.max(1), ctl.def.count_property.clone())
            };

            // Determine `effective_row_label_spec`, `effective_col_label_spec`, and `effective_cell_label_spec`:
            // - If `row_label` is given and `col_label` is not given, a 1D/column `label_override` (e.g.
            //   `UI.Field("Input {i}", self.Inputs, row_label="Track {i}")` or `UI.Field("Input {c}", ..., row_label=...)`)
            //   is promoted to `effective_col_label_spec` (top X-axis column header) instead of repeating on every cell!
            // - If neither `row_label` nor `col_label` is given and `label_override` has only row vars (`"Track {r}:"`),
            //   it is promoted to `effective_row_label_spec`.
            // - If neither `row_label` nor `col_label` is given and `label_override` has only col vars (`"Input {c}"`),
            //   it is promoted to `effective_col_label_spec`.
            let has_explicit_row_lbl =
                explicit_row_label_spec.is_some() || explicit_row_labels.is_some();
            let has_explicit_col_lbl =
                explicit_col_label_spec.is_some() || explicit_col_labels.is_some();
            let label_override_is_1d_or_col = match &label_override {
                Some(LabelSpec::Text(s)) => !has_row_template_vars(s),
                Some(LabelSpec::Callable(fn_expr)) => self.callable_arity(fn_expr) <= 1,
                None => false,
            };

            let (effective_row_label_spec, effective_col_label_spec, effective_cell_label_spec) =
                if !has_explicit_row_lbl
                    && let Some(LabelSpec::Text(ref s)) = label_override
                    && has_only_row_template_vars(s)
                {
                    (
                        Some(LabelSpec::Text(s.clone())),
                        explicit_col_label_spec,
                        explicit_cell_label_spec,
                    )
                } else if has_explicit_row_lbl
                    && !has_explicit_col_lbl
                    && label_override_is_1d_or_col
                {
                    (
                        explicit_row_label_spec,
                        label_override.clone(),
                        explicit_cell_label_spec,
                    )
                } else if !has_explicit_row_lbl
                    && !has_explicit_col_lbl
                    && let Some(LabelSpec::Text(ref s)) = label_override
                    && has_only_col_template_vars(s)
                {
                    (
                        None,
                        Some(LabelSpec::Text(s.clone())),
                        explicit_cell_label_spec,
                    )
                } else {
                    (
                        explicit_row_label_spec,
                        explicit_col_label_spec,
                        explicit_cell_label_spec.or_else(|| label_override.clone()),
                    )
                };

            let has_row_label = effective_row_label_spec.is_some() || explicit_row_labels.is_some();
            let mut preview_row_labels = Vec::with_capacity(preview_rows);
            let mut preview_row_label_lua_exprs = Vec::with_capacity(preview_rows);
            for r_idx in 0..preview_rows {
                let r1 = r_idx + 1;
                let r_idx_i64 = r_idx as i64;
                let r_idx_str = r_idx.to_string();
                if let Some(ref list) = explicit_row_labels {
                    preview_row_labels.push(
                        list.get(r_idx)
                            .cloned()
                            .unwrap_or_else(|| format!("Track {r1}")),
                    );
                    preview_row_label_lua_exprs.push(None);
                } else if let Some(ref rspec) = effective_row_label_spec {
                    match rspec {
                        LabelSpec::Text(tmpl) => {
                            preview_row_labels.push(format_row_label_surface(tmpl, r1));
                            preview_row_label_lua_exprs.push(None);
                        }
                        LabelSpec::Callable(fn_expr) => {
                            let p = self
                                .eval_callable_preview(fn_expr, r_idx_i64)
                                .unwrap_or_else(|| format!("Track {r1}"));
                            let l = self.lower_callable_lua(fn_expr, &r_idx_str);
                            preview_row_labels.push(p);
                            preview_row_label_lua_exprs.push(Some(l));
                        }
                    }
                }
            }

            let dynamic_row_label_template = match &effective_row_label_spec {
                Some(LabelSpec::Text(s)) => Some(s.clone()),
                Some(LabelSpec::Callable(_)) => Some(String::new()),
                None => None,
            };
            let dynamic_row_label_lua_expr = match &effective_row_label_spec {
                Some(LabelSpec::Callable(fn_expr)) => {
                    Some(self.lower_callable_lua(fn_expr, "r - 1"))
                }
                _ => None,
            };

            let has_col_label = effective_col_label_spec.is_some() || explicit_col_labels.is_some();
            let mut preview_col_labels = Vec::with_capacity(preview_cols);
            let mut preview_col_label_lua_exprs = Vec::with_capacity(preview_cols);
            for c_idx in 0..preview_cols {
                let c1 = c_idx + 1;
                let c_idx_i64 = c_idx as i64;
                let c_idx_str = c_idx.to_string();
                if let Some(ref list) = explicit_col_labels {
                    preview_col_labels.push(
                        list.get(c_idx)
                            .cloned()
                            .unwrap_or_else(|| format!("Input {c1}")),
                    );
                    preview_col_label_lua_exprs.push(None);
                } else if let Some(ref cspec) = effective_col_label_spec {
                    match cspec {
                        LabelSpec::Text(tmpl) => {
                            preview_col_labels.push(format_col_label_surface(tmpl, c1));
                            preview_col_label_lua_exprs.push(None);
                        }
                        LabelSpec::Callable(fn_expr) => {
                            let p = self
                                .eval_callable_preview(fn_expr, c_idx_i64)
                                .unwrap_or_else(|| format!("Input {c1}"));
                            let l = self.lower_callable_lua(fn_expr, &c_idx_str);
                            preview_col_labels.push(p);
                            preview_col_label_lua_exprs.push(Some(l));
                        }
                    }
                }
            }

            let dynamic_col_label_template = match &effective_col_label_spec {
                Some(LabelSpec::Text(s)) => Some(s.clone()),
                Some(LabelSpec::Callable(_)) => Some(String::new()),
                None => None,
            };
            let dynamic_col_label_lua_expr = match &effective_col_label_spec {
                Some(LabelSpec::Callable(fn_expr)) => {
                    Some(self.lower_callable_lua(fn_expr, "c - 1"))
                }
                _ => None,
            };

            let max_rlbl_chars = preview_row_labels
                .iter()
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(8);
            let default_row_lbl_w = ((max_rlbl_chars as f64) * 7.0 + 12.0).max(56.0);
            let row_label_width = self
                .kwarg_f64(kwargs, "row_label_width")
                .unwrap_or(default_row_lbl_w);
            let row_label_font_size = self.kwarg_u32(kwargs, "row_label_font_size").unwrap_or(11);
            let col_label_font_size = self.kwarg_u32(kwargs, "col_label_font_size").unwrap_or(11);
            let base_gap = self.kwarg_f64(kwargs, "gap").unwrap_or(8.0);
            let col_gap = self.kwarg_f64(kwargs, "col_gap").unwrap_or(base_gap);
            let row_gap = self.kwarg_f64(kwargs, "row_gap").unwrap_or(base_gap);

            let explicit_labels = self.kwarg_str_list(kwargs, "labels");
            let explicit_pretty_names = self.kwarg_str_list(kwargs, "pretty_names");
            let has_dynamic_repeat = (ctl.def.count_property.is_some()
                || rows_property.is_some()
                || cols_property.is_some())
                && explicit_labels.is_none()
                && explicit_pretty_names.is_none()
                && explicit_row_labels.is_none()
                && explicit_col_labels.is_none();

            let total_preview = (preview_rows * preview_cols).max(1);
            let mut cells = Vec::with_capacity(total_preview);
            for r_idx in 0..preview_rows {
                let r1 = r_idx + 1;
                let r_i64 = r_idx as i64;
                let r_str = r_idx.to_string();
                for c_idx in 0..preview_cols {
                    let c1 = c_idx + 1;
                    let c_i64 = c_idx as i64;
                    let c_str = c_idx.to_string();
                    let flat_idx = r_idx * preview_cols + c_idx;
                    let ch = flat_idx + 1;
                    let flat_i64 = flat_idx as i64;
                    let flat_str = flat_idx.to_string();

                    let qsys_key = if total_preview == 1 {
                        ctl.def.name.clone()
                    } else {
                        format!("{} {}", ctl.def.name, ch)
                    };

                    let cell_label = explicit_labels
                        .as_ref()
                        .and_then(|list| list.get(flat_idx).cloned())
                        .or_else(|| match &effective_cell_label_spec {
                            Some(LabelSpec::Text(lbl)) => {
                                Some(format_2d_surface(lbl, r1, c1, ch, has_row_label))
                            }
                            Some(LabelSpec::Callable(fn_expr)) => {
                                if self.callable_arity(fn_expr) >= 2 {
                                    Some(
                                        self.eval_callable_2d_preview(fn_expr, r_i64, c_i64)
                                            .unwrap_or_else(|| format!("{} {}", ctl.def.name, ch)),
                                    )
                                } else {
                                    Some(
                                        self.eval_callable_preview(fn_expr, flat_i64)
                                            .unwrap_or_else(|| format!("{} {}", ctl.def.name, ch)),
                                    )
                                }
                            }
                            None => None,
                        });

                    let cell_pretty = explicit_pretty_names
                        .as_ref()
                        .and_then(|list| list.get(flat_idx).cloned())
                        .or_else(|| match &pretty_spec {
                            Some(LabelSpec::Text(p)) => {
                                Some(format_2d_surface(p, r1, c1, ch, has_row_label))
                            }
                            Some(LabelSpec::Callable(fn_expr)) => {
                                if self.callable_arity(fn_expr) >= 2 {
                                    Some(
                                        self.eval_callable_2d_preview(fn_expr, r_i64, c_i64)
                                            .unwrap_or_else(|| qsys_key.clone()),
                                    )
                                } else {
                                    Some(
                                        self.eval_callable_preview(fn_expr, flat_i64)
                                            .unwrap_or_else(|| qsys_key.clone()),
                                    )
                                }
                            }
                            None => None,
                        })
                        .or_else(|| cell_label.clone())
                        .or_else(|| preview_col_labels.get(c_idx).cloned())
                        .or_else(|| Some(qsys_key.clone()));

                    let cell_legend = legend
                        .as_ref()
                        .map(|l| format_2d_surface(l, r1, c1, ch, has_row_label));

                    let cell_label_lua = match &effective_cell_label_spec {
                        Some(LabelSpec::Callable(fn_expr)) => {
                            let is_2d_fn = self.callable_arity(fn_expr) >= 2;
                            if has_dynamic_repeat {
                                if flat_idx == 0 {
                                    if is_2d_fn {
                                        Some(self.lower_callable_2d_lua(fn_expr, "r - 1", "c - 1"))
                                    } else {
                                        Some(self.lower_callable_lua(fn_expr, "i - 1"))
                                    }
                                } else {
                                    None
                                }
                            } else if is_2d_fn {
                                Some(self.lower_callable_2d_lua(fn_expr, &r_str, &c_str))
                            } else {
                                Some(self.lower_callable_lua(fn_expr, &flat_str))
                            }
                        }
                        _ => None,
                    };

                    let cell_pretty_lua = match &pretty_spec {
                        Some(LabelSpec::Callable(fn_expr)) => {
                            let is_2d_fn = self.callable_arity(fn_expr) >= 2;
                            if has_dynamic_repeat {
                                if flat_idx == 0 {
                                    if is_2d_fn {
                                        Some(self.lower_callable_2d_lua(fn_expr, "r - 1", "c - 1"))
                                    } else {
                                        Some(self.lower_callable_lua(fn_expr, "i - 1"))
                                    }
                                } else {
                                    None
                                }
                            } else if is_2d_fn {
                                Some(self.lower_callable_2d_lua(fn_expr, &r_str, &c_str))
                            } else {
                                Some(self.lower_callable_lua(fn_expr, &flat_str))
                            }
                        }
                        None => cell_label_lua.clone().or_else(|| {
                            if has_dynamic_repeat {
                                if flat_idx == 0 {
                                    dynamic_col_label_lua_expr.clone()
                                } else {
                                    None
                                }
                            } else {
                                preview_col_label_lua_exprs
                                    .get(c_idx)
                                    .and_then(|e| e.clone())
                            }
                        }),
                        _ => None,
                    };

                    let (_, cell_style_lua) = if has_dynamic_repeat {
                        if flat_idx == 0 {
                            self.resolve_style_for_item(
                                &style_spec,
                                ctl,
                                Some(r_i64),
                                Some(c_i64),
                                flat_i64,
                                Some("r - 1"),
                                Some("c - 1"),
                                "i - 1",
                            )
                        } else {
                            (style.clone(), None)
                        }
                    } else {
                        self.resolve_style_for_item(
                            &style_spec,
                            ctl,
                            Some(r_i64),
                            Some(c_i64),
                            flat_i64,
                            Some(&r_str),
                            Some(&c_str),
                            &flat_str,
                        )
                    };

                    cells.push(GridCellPreview {
                        control_name: qsys_key,
                        channel_index: ch,
                        label: cell_label,
                        pretty_name: cell_pretty,
                        legend: cell_legend,
                        label_lua_expr: cell_label_lua,
                        pretty_lua_expr: cell_pretty_lua,
                        style_lua_expr: cell_style_lua,
                    });
                }
            }

            let head_cell = cells.first();
            let head_label = head_cell.and_then(|c| c.label.clone());
            let head_pretty = head_cell.and_then(|c| c.pretty_name.clone());
            let head_legend = head_cell.and_then(|c| c.legend.clone());
            let head_label_lua = head_cell.and_then(|c| c.label_lua_expr.clone());
            let head_pretty_lua = head_cell.and_then(|c| c.pretty_lua_expr.clone());
            let head_style_lua = head_cell.and_then(|c| c.style_lua_expr.clone());
            let head_ctl_name = head_cell
                .map(|c| c.control_name.clone())
                .unwrap_or_else(|| format!("{} 1", ctl.def.name));

            let dyn_cell_label_tmpl = if has_dynamic_repeat {
                match &effective_cell_label_spec {
                    Some(LabelSpec::Text(s)) => Some(normalize_2d_cell_template(s, has_row_label)),
                    Some(LabelSpec::Callable(_)) => Some(String::new()),
                    None => None,
                }
            } else {
                None
            };
            let dyn_cell_pretty_tmpl = if has_dynamic_repeat {
                match &pretty_spec {
                    Some(LabelSpec::Text(s)) => Some(normalize_2d_cell_template(s, has_row_label)),
                    Some(LabelSpec::Callable(_)) => Some(String::new()),
                    None => dyn_cell_label_tmpl.clone().or_else(|| {
                        if let Some(LabelSpec::Text(ref c_tmpl)) = effective_col_label_spec {
                            Some(normalize_col_template_to_2d_pretty(c_tmpl))
                        } else if let Some(LabelSpec::Callable(_)) = effective_col_label_spec {
                            Some(String::new())
                        } else if let Some(LabelSpec::Text(ref r_tmpl)) = effective_row_label_spec {
                            Some(format!("{} {{c}}", r_tmpl.trim_end_matches(':').trim()))
                        } else {
                            Some(ctl.def.name.clone())
                        }
                    }),
                }
            } else {
                None
            };

            return vec![UiNode::Control(UiControlSpec {
                control_name: head_ctl_name,
                style,
                label: head_label,
                label_placement,
                pretty_name: head_pretty,
                legend: head_legend,
                width,
                height,
                color,
                margin,
                read_only,
                invisible,
                type_property,
                count_property: ctl.def.count_property.clone(),
                base_control_name: Some(ctl.def.name.clone()),
                channel_index: Some(1),
                dynamic_label_template: dyn_cell_label_tmpl,
                dynamic_pretty_template: dyn_cell_pretty_tmpl,
                dynamic_legend_template: if has_dynamic_repeat { legend } else { None },
                label_lua_expr: head_label_lua,
                pretty_lua_expr: head_pretty_lua,
                style_lua_expr: head_style_lua,
                grid: Some(UiGridSpec {
                    preview_rows,
                    preview_cols,
                    rows_property,
                    cols_property,
                    col_gap,
                    row_gap,
                    has_row_label,
                    row_label_width,
                    row_label_font_size,
                    preview_row_labels,
                    preview_row_label_lua_exprs,
                    dynamic_row_label_template,
                    dynamic_row_label_lua_expr,
                    has_col_label,
                    col_label_font_size,
                    preview_col_labels,
                    preview_col_label_lua_exprs,
                    dynamic_col_label_template,
                    dynamic_col_label_lua_expr,
                    cells,
                }),
                is_dynamic_repeat_head: has_dynamic_repeat,
                skip_lua_emit: false,
            })];
        }

        // Multi-channel or property-counted control referenced as a group (no index specified):
        // Automatically expand `1..=count` channels for preview, and if `count_property` is set,
        // mark channel 1 as a dynamic repeat head for `for i = 1, props["..."].Value do` in Lua!
        let explicit_labels = self.kwarg_str_list(kwargs, "labels");
        let explicit_pretty_names = self.kwarg_str_list(kwargs, "pretty_names");
        let has_dynamic_repeat = ctl.def.count_property.is_some()
            && explicit_labels.is_none()
            && explicit_pretty_names.is_none();

        let label_text_opt = match &label_override {
            Some(LabelSpec::Text(s)) => Some(s.clone()),
            _ => None,
        };
        let pretty_text_opt = match &pretty_spec {
            Some(LabelSpec::Text(s)) => Some(s.clone()),
            _ => None,
        };
        let dyn_pretty_tmpl = pretty_text_opt
            .clone()
            .or_else(|| label_text_opt.clone())
            .unwrap_or_else(|| ctl.def.name.clone());

        let preview_count = ctl.def.count.max(1);
        let mut expanded = Vec::with_capacity(preview_count);
        for ch in 1..=preview_count {
            let idx0 = (ch - 1) as i64;
            let idx0_str = idx0.to_string();
            let qsys_key = if preview_count == 1 {
                ctl.def.name.clone()
            } else {
                format!("{} {}", ctl.def.name, ch)
            };
            let ch_label = explicit_labels
                .as_ref()
                .and_then(|list| list.get(ch - 1).cloned())
                .or_else(|| match &label_override {
                    Some(LabelSpec::Text(lbl)) => {
                        if preview_count == 1 && !lbl.contains("{i}") && !lbl.contains("{ch}") {
                            Some(lbl.clone())
                        } else {
                            Some(format_channel_surface(lbl, ch))
                        }
                    }
                    Some(LabelSpec::Callable(fn_expr)) => Some(
                        self.eval_callable_preview(fn_expr, idx0)
                            .unwrap_or_else(|| format!("{} {}", ctl.def.name, ch)),
                    ),
                    None => None,
                });
            let ch_pretty = explicit_pretty_names
                .as_ref()
                .and_then(|list| list.get(ch - 1).cloned())
                .or_else(|| match &pretty_spec {
                    Some(LabelSpec::Text(p)) => {
                        if preview_count == 1 && !p.contains("{i}") && !p.contains("{ch}") {
                            Some(p.clone())
                        } else {
                            Some(format_channel_surface(p, ch))
                        }
                    }
                    Some(LabelSpec::Callable(fn_expr)) => Some(
                        self.eval_callable_preview(fn_expr, idx0)
                            .unwrap_or_else(|| qsys_key.clone()),
                    ),
                    None => None,
                })
                .or_else(|| ch_label.clone())
                .or_else(|| Some(qsys_key.clone()));
            let ch_legend = legend.as_ref().map(|l| format_channel_surface(l, ch));

            let item_label_lua = match &label_override {
                Some(LabelSpec::Callable(fn_expr)) => {
                    if has_dynamic_repeat {
                        if ch == 1 {
                            Some(self.lower_callable_lua(fn_expr, "i - 1"))
                        } else {
                            None
                        }
                    } else {
                        Some(self.lower_callable_lua(fn_expr, &idx0_str))
                    }
                }
                _ => None,
            };
            let item_pretty_lua = match &pretty_spec {
                Some(LabelSpec::Callable(fn_expr)) => {
                    if has_dynamic_repeat {
                        if ch == 1 {
                            Some(self.lower_callable_lua(fn_expr, "i - 1"))
                        } else {
                            None
                        }
                    } else {
                        Some(self.lower_callable_lua(fn_expr, &idx0_str))
                    }
                }
                None => item_label_lua.clone(),
                _ => None,
            };

            let (item_style, item_style_lua) = if has_dynamic_repeat {
                if ch == 1 {
                    self.resolve_style_for_item(
                        &style_spec,
                        ctl,
                        None,
                        None,
                        idx0,
                        None,
                        None,
                        "i - 1",
                    )
                } else {
                    (style.clone(), None)
                }
            } else {
                self.resolve_style_for_item(
                    &style_spec,
                    ctl,
                    None,
                    None,
                    idx0,
                    None,
                    None,
                    &idx0_str,
                )
            };

            let dyn_label_tmpl_for_item = if has_dynamic_repeat && ch == 1 {
                match &label_override {
                    Some(LabelSpec::Text(s)) => Some(s.clone()),
                    Some(LabelSpec::Callable(_)) => Some(String::new()),
                    None => None,
                }
            } else {
                None
            };

            expanded.push(UiNode::Control(UiControlSpec {
                control_name: qsys_key,
                style: item_style,
                label: ch_label,
                label_placement,
                pretty_name: ch_pretty,
                legend: ch_legend,
                width,
                height,
                color: color.clone(),
                margin,
                read_only,
                invisible,
                type_property: type_property.clone(),
                count_property: ctl.def.count_property.clone(),
                base_control_name: Some(ctl.def.name.clone()),
                channel_index: Some(ch),
                dynamic_label_template: dyn_label_tmpl_for_item,
                dynamic_pretty_template: if has_dynamic_repeat && ch == 1 {
                    Some(dyn_pretty_tmpl.clone())
                } else {
                    None
                },
                dynamic_legend_template: if has_dynamic_repeat && ch == 1 {
                    legend.clone()
                } else {
                    None
                },
                label_lua_expr: item_label_lua,
                pretty_lua_expr: item_pretty_lua,
                style_lua_expr: item_style_lua,
                grid: None,
                is_dynamic_repeat_head: has_dynamic_repeat && ch == 1,
                skip_lua_emit: has_dynamic_repeat && ch > 1,
            }));
        }

        if let Some(dir_str) = self.kwarg_str(kwargs, "direction") {
            let gap = self.kwarg_f64(kwargs, "gap").unwrap_or(8.0);
            match dir_str.to_ascii_lowercase().as_str() {
                "row" | "horizontal" | "hstack" => {
                    return vec![UiNode::Row {
                        children: expanded,
                        gap,
                        padding: 0.0,
                    }];
                }
                "col" | "column" | "vertical" | "vstack" => {
                    return vec![UiNode::Col {
                        children: expanded,
                        gap,
                        padding: 0.0,
                    }];
                }
                _ => {}
            }
        }

        expanded
    }

    fn parse_ui_dim_kwarg(&self, expr: &ast::Expr) -> Option<(usize, Option<String>)> {
        let lookup_prop = |ident: &str| -> Option<(usize, Option<String>)> {
            let prop = self
                .env
                .properties
                .iter()
                .find(|p| p.ident == ident || p.def.name == ident)?;
            let default_count = match &prop.def.kind {
                PropertyKind::Integer { default, .. } => (*default).max(1) as usize,
                _ => 1,
            };
            Some((default_count, Some(prop.def.name.clone())))
        };
        match &expr.kind {
            ast::ExprKind::Ident(ident) => {
                if let Some(res) = lookup_prop(ident) {
                    return Some(res);
                }
            }
            ast::ExprKind::Attribute { value, attr } => {
                if attr == "Value" {
                    if let ast::ExprKind::Ident(prop_ident) = &value.kind
                        && let Some(res) = lookup_prop(prop_ident)
                    {
                        return Some(res);
                    }
                    if let ast::ExprKind::Attribute {
                        value: inner_obj,
                        attr: prop_ident,
                    } = &value.kind
                        && let ast::ExprKind::Ident(obj) = &inner_obj.kind
                        && (obj == "self" || obj == self.class_name)
                        && let Some(res) = lookup_prop(prop_ident)
                    {
                        return Some(res);
                    }
                }
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                    && let Some(res) = lookup_prop(attr)
                {
                    return Some(res);
                }
            }
            _ => {}
        }
        if self.env.is_dynamic_design_expr(self.class_name, expr) {
            let preview = self
                .env
                .eval_design_expr_preview(self.class_name, expr)?
                .as_i64()?
                .max(1) as usize;
            let tmpl = self
                .env
                .lower_design_expr_to_prop_template(self.class_name, expr);
            let spec = if tmpl.contains("{PROP:") {
                tmpl
            } else {
                format!("{{LUA:{tmpl}}}")
            };
            return Some((preview, Some(spec)));
        }
        self.eval_i64(expr).map(|i| (i.max(1) as usize, None))
    }

    fn callable_arity(&self, fn_expr: &ast::Expr) -> usize {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { params, .. } => params.len(),
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                    && let Some(method) = self.env.class_methods.get(attr)
                {
                    let skip_self =
                        usize::from(method.params.first().is_some_and(|p| p.name == "self"));
                    return method.params.len().saturating_sub(skip_self);
                }
                1
            }
            ast::ExprKind::Ident(name) => {
                if let Some(func) = self
                    .env
                    .local_functions
                    .get(name)
                    .or_else(|| self.env.class_methods.get(name))
                    .or_else(|| self.env.module_functions.get(name))
                {
                    let skip_self =
                        usize::from(func.params.first().is_some_and(|p| p.name == "self"));
                    return func.params.len().saturating_sub(skip_self);
                }
                1
            }
            _ => 1,
        }
    }

    fn lower_callable_0_lua(&self, fn_expr: &ast::Expr) -> String {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { body, .. } => {
                let body_lua = self.lower_ui_expr_to_lua(body, &FxHashMap::default());
                format!("(function() return {body_lua} end)()")
            }
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                {
                    format!("{}_{attr}(nil)", self.class_name)
                } else {
                    let obj_lua = self.lower_ui_expr_to_lua(value, &FxHashMap::default());
                    format!("{obj_lua}.{attr}()")
                }
            }
            ast::ExprKind::Ident(name) => {
                if self.env.local_functions.contains_key(name)
                    || self.env.module_functions.contains_key(name)
                {
                    format!("{name}()")
                } else if self.env.class_methods.contains_key(name) {
                    format!("{}_{name}(nil)", self.class_name)
                } else {
                    format!("{name}()")
                }
            }
            _ => "\"\"".to_string(),
        }
    }

    fn lower_callable_lua(&self, fn_expr: &ast::Expr, idx_lua: &str) -> String {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { params, body } => {
                let param_name = params.first().map(|p| p.name.as_str()).unwrap_or("_i");
                let mut param_map = FxHashMap::default();
                param_map.insert(param_name.to_string(), param_name.to_string());
                let body_lua = self.lower_ui_expr_to_lua(body, &param_map);
                format!("(function({param_name}) return {body_lua} end)({idx_lua})")
            }
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                {
                    format!("{}_{attr}(nil, {idx_lua})", self.class_name)
                } else {
                    let obj_lua = self.lower_ui_expr_to_lua(value, &FxHashMap::default());
                    format!("{obj_lua}.{attr}({idx_lua})")
                }
            }
            ast::ExprKind::Ident(name) => {
                if self.env.local_functions.contains_key(name)
                    || self.env.module_functions.contains_key(name)
                {
                    format!("{name}({idx_lua})")
                } else if self.env.class_methods.contains_key(name) {
                    format!("{}_{name}(nil, {idx_lua})", self.class_name)
                } else {
                    format!("{name}({idx_lua})")
                }
            }
            _ => "\"\"".to_string(),
        }
    }

    fn lower_callable_2d_lua(&self, fn_expr: &ast::Expr, r_lua: &str, c_lua: &str) -> String {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { params, body } => {
                let p0 = params.first().map(|p| p.name.as_str()).unwrap_or("_r");
                let p1 = params.get(1).map(|p| p.name.as_str()).unwrap_or("_c");
                let mut param_map = FxHashMap::default();
                param_map.insert(p0.to_string(), p0.to_string());
                param_map.insert(p1.to_string(), p1.to_string());
                let body_lua = self.lower_ui_expr_to_lua(body, &param_map);
                format!("(function({p0}, {p1}) return {body_lua} end)({r_lua}, {c_lua})")
            }
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                {
                    format!("{}_{attr}(nil, {r_lua}, {c_lua})", self.class_name)
                } else {
                    let obj_lua = self.lower_ui_expr_to_lua(value, &FxHashMap::default());
                    format!("{obj_lua}.{attr}({r_lua}, {c_lua})")
                }
            }
            ast::ExprKind::Ident(name) => {
                if self.env.local_functions.contains_key(name)
                    || self.env.module_functions.contains_key(name)
                {
                    format!("{name}({r_lua}, {c_lua})")
                } else if self.env.class_methods.contains_key(name) {
                    format!("{}_{name}(nil, {r_lua}, {c_lua})", self.class_name)
                } else {
                    format!("{name}({r_lua}, {c_lua})")
                }
            }
            _ => "\"\"".to_string(),
        }
    }

    fn lower_ui_expr_to_lua(
        &self,
        expr: &ast::Expr,
        local_map: &FxHashMap<String, String>,
    ) -> String {
        self.env
            .lower_ui_expr_to_lua_inner(self.class_name, expr, local_map, false)
    }

    fn eval_callable_0_preview(&self, fn_expr: &ast::Expr) -> Option<String> {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { body, .. } => {
                let mut vars = FxHashMap::default();
                for (name, val) in &self.env.constants {
                    vars.insert(name.clone(), PreviewVal::from_const(val));
                }
                self.eval_preview_expr(body, &vars, 0)
                    .map(|v| v.to_display_string())
            }
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                    && let Some(method) = self.env.class_methods.get(attr)
                {
                    return self
                        .eval_preview_function(method, &[], 0)
                        .map(|v| v.to_display_string());
                }
                None
            }
            ast::ExprKind::Ident(name) => {
                let func = self
                    .env
                    .local_functions
                    .get(name)
                    .or_else(|| self.env.class_methods.get(name))
                    .or_else(|| self.env.module_functions.get(name))?;
                self.eval_preview_function(func, &[], 0)
                    .map(|v| v.to_display_string())
            }
            _ => None,
        }
    }

    fn eval_callable_preview(&self, fn_expr: &ast::Expr, idx: i64) -> Option<String> {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { params, body } => {
                let mut vars = FxHashMap::default();
                for (name, val) in &self.env.constants {
                    vars.insert(name.clone(), PreviewVal::from_const(val));
                }
                if let Some(first_param) = params.first() {
                    vars.insert(first_param.name.clone(), PreviewVal::Int(idx));
                }
                self.eval_preview_expr(body, &vars, 0)
                    .map(|v| v.to_display_string())
            }
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                    && let Some(method) = self.env.class_methods.get(attr)
                {
                    return self
                        .eval_preview_function(method, &[PreviewVal::Int(idx)], 0)
                        .map(|v| v.to_display_string());
                }
                None
            }
            ast::ExprKind::Ident(name) => {
                let func = self
                    .env
                    .local_functions
                    .get(name)
                    .or_else(|| self.env.class_methods.get(name))
                    .or_else(|| self.env.module_functions.get(name))?;
                self.eval_preview_function(func, &[PreviewVal::Int(idx)], 0)
                    .map(|v| v.to_display_string())
            }
            _ => None,
        }
    }

    fn eval_callable_2d_preview(&self, fn_expr: &ast::Expr, r: i64, c: i64) -> Option<String> {
        match &fn_expr.kind {
            ast::ExprKind::Lambda { params, body } => {
                let mut vars = FxHashMap::default();
                for (name, val) in &self.env.constants {
                    vars.insert(name.clone(), PreviewVal::from_const(val));
                }
                if let Some(p0) = params.first() {
                    vars.insert(p0.name.clone(), PreviewVal::Int(r));
                }
                if let Some(p1) = params.get(1) {
                    vars.insert(p1.name.clone(), PreviewVal::Int(c));
                }
                self.eval_preview_expr(body, &vars, 0)
                    .map(|v| v.to_display_string())
            }
            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                    && let Some(method) = self.env.class_methods.get(attr)
                {
                    return self
                        .eval_preview_function(method, &[PreviewVal::Int(r), PreviewVal::Int(c)], 0)
                        .map(|v| v.to_display_string());
                }
                None
            }
            ast::ExprKind::Ident(name) => {
                let func = self
                    .env
                    .local_functions
                    .get(name)
                    .or_else(|| self.env.class_methods.get(name))
                    .or_else(|| self.env.module_functions.get(name))?;
                self.eval_preview_function(func, &[PreviewVal::Int(r), PreviewVal::Int(c)], 0)
                    .map(|v| v.to_display_string())
            }
            _ => None,
        }
    }

    fn eval_preview_function(
        &self,
        func: &ast::FunctionDef,
        args: &[PreviewVal],
        depth: usize,
    ) -> Option<PreviewVal> {
        self.env
            .eval_preview_function_in_env(self.class_name, func, args, depth)
    }

    fn eval_preview_expr(
        &self,
        expr: &ast::Expr,
        vars: &FxHashMap<String, PreviewVal>,
        depth: usize,
    ) -> Option<PreviewVal> {
        self.env
            .eval_preview_expr_in_env(self.class_name, expr, vars, depth)
    }

    fn resolve_control_ref(
        &mut self,
        expr: &ast::Expr,
    ) -> Option<(&'a DeclaredControl, Option<usize>)> {
        match &expr.kind {
            ast::ExprKind::Ident(ident) => {
                if let Some(ctl) = self.controls.iter().find(|c| c.ident == *ident) {
                    self.control_resolutions.push((expr.span, ctl.id));
                    Some((ctl, None))
                } else {
                    let candidates = find_closest_control(ident, self.controls);
                    self.diagnostics.report(
                        DiagnosticKind::UnknownClassMember {
                            class_name: self.class_name.to_string(),
                            member: ident.clone(),
                            candidates,
                        },
                        expr.span,
                    );
                    None
                }
            }

            ast::ExprKind::Attribute { value, attr } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && obj == "self"
                {
                    if let Some(ctl) = self.controls.iter().find(|c| c.ident == *attr) {
                        self.control_resolutions.push((expr.span, ctl.id));
                        return Some((ctl, None));
                    } else {
                        let candidates = find_closest_control(attr, self.controls);
                        self.diagnostics.report(
                            DiagnosticKind::UnknownClassMember {
                                class_name: self.class_name.to_string(),
                                member: attr.clone(),
                                candidates,
                            },
                            expr.span,
                        );
                        return None;
                    }
                }
                self.diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: "UI.Control".to_string(),
                        message:
                            "Expected a declared control reference (e.g. 'self.Gain' or 'Gain')"
                                .to_string(),
                    },
                    expr.span,
                );
                None
            }

            ast::ExprKind::Subscript { value, index } => {
                let (ctl, outer_idx) = self.resolve_control_ref(value)?;
                let idx_val = if let Some(r_idx) = outer_idx {
                    let Some(c_val) = self.eval_i64(index) else {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: ctl.ident.clone(),
                                message: "Control subscript index in UI layout must be an integer literal".to_string(),
                            },
                            index.span,
                        );
                        return None;
                    };
                    let cols = ctl.def.cols.unwrap_or(1) as i64;
                    (r_idx as i64) * cols + c_val
                } else if let ast::ExprKind::Tuple(elts) = &index.kind
                    && elts.len() == 2
                {
                    let (Some(r_val), Some(c_val)) =
                        (self.eval_i64(&elts[0]), self.eval_i64(&elts[1]))
                    else {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: ctl.ident.clone(),
                                message: "Control 2D subscript indices in UI layout must be integer literals".to_string(),
                            },
                            index.span,
                        );
                        return None;
                    };
                    if r_val < 0 || c_val < 0 {
                        -1
                    } else {
                        let cols = ctl.def.cols.unwrap_or(1) as i64;
                        r_val * cols + c_val
                    }
                } else {
                    let Some(val) = self.eval_i64(index) else {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: ctl.ident.clone(),
                                message:
                                    "Control subscript index in UI layout must be an integer literal"
                                        .to_string(),
                            },
                            index.span,
                        );
                        return None;
                    };
                    val
                };
                if idx_val < 0
                    || (ctl.def.count_property.is_none() && (idx_val as usize) >= ctl.def.count)
                {
                    self.diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: ctl.ident.clone(),
                            message: format!(
                                "Control '{}' has count {}; index {} is out of bounds (expected 0..{})",
                                ctl.ident,
                                ctl.def.count,
                                idx_val,
                                ctl.def.count.saturating_sub(1)
                            ),
                        },
                        index.span,
                    );
                    return None;
                }
                Some((ctl, Some(idx_val as usize)))
            }

            _ => {
                self.diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: "UI.Control".to_string(),
                        message:
                            "Expected a declared control reference (e.g. 'self.Gain' or 'Gain'), not a string or arbitrary expression"
                                .to_string(),
                    },
                    expr.span,
                );
                None
            }
        }
    }

    fn eval_str(&self, expr: &ast::Expr) -> Option<String> {
        self.env
            .eval_design_expr_preview(self.class_name, expr)
            .and_then(|v| v.as_str().map(String::from))
    }

    fn eval_f64(&self, expr: &ast::Expr) -> Option<f64> {
        self.env
            .eval_design_expr_preview(self.class_name, expr)
            .and_then(|v| v.as_f64())
    }

    fn eval_i64(&self, expr: &ast::Expr) -> Option<i64> {
        self.env
            .eval_design_expr_preview(self.class_name, expr)
            .and_then(|v| v.as_i64())
    }

    fn eval_bool(&self, expr: &ast::Expr) -> Option<bool> {
        self.env
            .eval_design_expr_preview(self.class_name, expr)
            .and_then(|v| v.as_bool())
    }

    fn kwarg_str_list(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<Vec<String>> {
        let (_, expr) = kwargs.iter().find(|(k, _)| k == key)?;
        let PreviewVal::Tuple(items) = self.env.eval_design_expr_preview(self.class_name, expr)?
        else {
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(item.as_str()?.to_string());
        }
        Some(out)
    }

    fn kwarg_str(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<String> {
        kwargs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| self.eval_str(v))
    }

    fn kwarg_f64(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<f64> {
        kwargs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| self.eval_f64(v))
    }

    fn kwarg_u32(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<u32> {
        kwargs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| self.eval_i64(v))
            .and_then(|i| u32::try_from(i).ok())
    }

    fn kwarg_bool(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<bool> {
        kwargs
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| self.eval_bool(v))
    }

    fn kwarg_size(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<(f64, f64)> {
        let (_, expr) = kwargs.iter().find(|(k, _)| k == key)?;
        if let Some(PreviewVal::Tuple(items)) =
            self.env.eval_design_expr_preview(self.class_name, expr)
            && items.len() == 2
        {
            return Some((items[0].as_f64()?, items[1].as_f64()?));
        }
        None
    }

    fn kwarg_color(&self, kwargs: &[(String, ast::Expr)], key: &str) -> Option<Color> {
        let (_, expr) = kwargs.iter().find(|(k, _)| k == key)?;
        let val = self.env.eval_design_expr_preview(self.class_name, expr)?;
        if let Some(s) = val.as_str() {
            return Some(Color::parse_str(s));
        }
        if let PreviewVal::Tuple(items) = val
            && items.len() == 3
        {
            let r = items[0].as_i64()?.clamp(0, 255) as u8;
            let g = items[1].as_i64()?.clamp(0, 255) as u8;
            let b = items[2].as_i64()?.clamp(0, 255) as u8;
            return Some(Color::rgb(r, g, b));
        }
        None
    }
}

fn is_ui_node_or_control_expr(
    expr: &ast::Expr,
    controls: &[DeclaredControl],
    class_name: &str,
) -> bool {
    match &expr.kind {
        ast::ExprKind::List(_) | ast::ExprKind::Tuple(_) => true,
        ast::ExprKind::Call { callee, .. } => is_ui_callee(callee).is_some(),
        ast::ExprKind::Ident(ident) => controls.iter().any(|c| c.ident == *ident),
        ast::ExprKind::Attribute { value, attr } => {
            if let ast::ExprKind::Ident(obj) = &value.kind
                && (obj == "self" || obj == class_name)
            {
                return controls.iter().any(|c| c.ident == *attr);
            }
            false
        }
        ast::ExprKind::Subscript { value, .. } => {
            is_ui_node_or_control_expr(value, controls, class_name)
        }
        _ => false,
    }
}

fn escape_lua_str_content(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn normalize_style_string_literals(expr: &ast::Expr) -> ast::Expr {
    match &expr.kind {
        ast::ExprKind::String(s) => {
            let norm = match s.trim().to_ascii_lowercase().as_str() {
                "fader" => "Fader",
                "knob" => "Knob",
                "button" => "Button",
                "text" => "Text",
                "meter" => "Meter",
                "led" => "Led",
                "listbox" => "ListBox",
                "combobox" | "combo" => "ComboBox",
                "none" => "None",
                _ => s.as_str(),
            };
            ast::Expr {
                kind: ast::ExprKind::String(norm.to_string()),
                span: expr.span,
            }
        }
        ast::ExprKind::BoolOp { op, left, right } => ast::Expr {
            kind: ast::ExprKind::BoolOp {
                op: *op,
                left: Box::new(normalize_style_string_literals(left)),
                right: Box::new(normalize_style_string_literals(right)),
            },
            span: expr.span,
        },
        ast::ExprKind::IfExp { test, body, orelse } => ast::Expr {
            kind: ast::ExprKind::IfExp {
                test: test.clone(),
                body: Box::new(normalize_style_string_literals(body)),
                orelse: Box::new(normalize_style_string_literals(orelse)),
            },
            span: expr.span,
        },
        ast::ExprKind::Lambda { params, body } => ast::Expr {
            kind: ast::ExprKind::Lambda {
                params: params.clone(),
                body: Box::new(normalize_style_string_literals(body)),
            },
            span: expr.span,
        },
        _ => expr.clone(),
    }
}

fn has_row_template_vars(template: &str) -> bool {
    template.contains("{r}")
        || template.contains("{row}")
        || template.contains("{track}")
        || template.contains("{y}")
}

fn has_col_template_vars(template: &str) -> bool {
    template.contains("{c}") || template.contains("{col}") || template.contains("{x}")
}

fn has_2d_template_vars(template: &str) -> bool {
    has_row_template_vars(template) || has_col_template_vars(template)
}

fn has_only_row_template_vars(template: &str) -> bool {
    let has_row = has_row_template_vars(template);
    let has_col_or_flat =
        has_col_template_vars(template) || template.contains("{i}") || template.contains("{ch}");
    has_row && !has_col_or_flat
}

fn has_only_col_template_vars(template: &str) -> bool {
    let has_col = has_col_template_vars(template);
    let has_row_or_flat =
        has_row_template_vars(template) || template.contains("{i}") || template.contains("{ch}");
    has_col && !has_row_or_flat
}

fn normalize_2d_cell_template(template: &str, has_row_label: bool) -> String {
    if has_2d_template_vars(template) || template.contains("{i}") || template.contains("{ch}") {
        template.replace("{y}", "{r}").replace("{x}", "{c}")
    } else if has_row_label {
        format!("{template} {{c}}")
    } else {
        template.to_string()
    }
}

fn normalize_col_template_to_2d_pretty(template: &str) -> String {
    if has_col_template_vars(template) {
        template.replace("{x}", "{c}").replace("{col}", "{c}")
    } else if template.contains("{i}") || template.contains("{ch}") {
        template.replace("{i}", "{c}").replace("{ch}", "{c}")
    } else {
        format!("{template} {{c}}")
    }
}

fn format_2d_template(template: &str, r: usize, c: usize, ch: usize) -> String {
    let r_str = r.to_string();
    let c_str = c.to_string();
    let ch_str = ch.to_string();
    template
        .replace("{r}", &r_str)
        .replace("{row}", &r_str)
        .replace("{track}", &r_str)
        .replace("{y}", &r_str)
        .replace("{c}", &c_str)
        .replace("{col}", &c_str)
        .replace("{x}", &c_str)
        .replace("{i}", &ch_str)
        .replace("{ch}", &ch_str)
}

fn format_2d_surface(template: &str, r: usize, c: usize, ch: usize, has_row_label: bool) -> String {
    if has_2d_template_vars(template) || template.contains("{i}") || template.contains("{ch}") {
        format_2d_template(template, r, c, ch)
    } else if has_row_label {
        format!("{template} {c}")
    } else {
        format!("{template} {ch}")
    }
}

fn format_row_label_surface(template: &str, r: usize) -> String {
    let r_str = r.to_string();
    if has_row_template_vars(template) {
        template
            .replace("{r}", &r_str)
            .replace("{row}", &r_str)
            .replace("{track}", &r_str)
            .replace("{y}", &r_str)
    } else if template.contains("{i}") || template.contains("{ch}") {
        template.replace("{i}", &r_str).replace("{ch}", &r_str)
    } else {
        format!("{template} {r_str}")
    }
}

fn format_col_label_surface(template: &str, c: usize) -> String {
    let c_str = c.to_string();
    if has_col_template_vars(template) {
        template
            .replace("{c}", &c_str)
            .replace("{col}", &c_str)
            .replace("{x}", &c_str)
    } else if template.contains("{i}") || template.contains("{ch}") {
        template.replace("{i}", &c_str).replace("{ch}", &c_str)
    } else {
        format!("{template} {c_str}")
    }
}

fn format_channel_template(template: &str, ch: usize) -> String {
    let ch_str = ch.to_string();
    template.replace("{i}", &ch_str).replace("{ch}", &ch_str)
}

fn format_channel_surface(template: &str, ch: usize) -> String {
    if template.contains("{i}") || template.contains("{ch}") {
        format_channel_template(template, ch)
    } else {
        format!("{template} {ch}")
    }
}
