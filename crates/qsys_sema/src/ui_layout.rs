use rustc_hash::FxHashMap;
use strsim::levenshtein;

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::ControlId;
use qsys_model::{
    Color, ControlStyle, FlexDirection, LabelPlacement, PageLayout, PropertyKind, UiControlSpec,
    UiNode, UiPageSpec,
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
}

#[derive(Debug, Clone)]
enum LabelSpec {
    Text(String),
    Callable(ast::Expr),
}

#[derive(Debug, Clone)]
enum PreviewVal {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Tuple(Vec<PreviewVal>),
    Prop(String),
}

impl PreviewVal {
    fn to_display_string(&self) -> String {
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

    fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    fn is_truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Int(i) => *i != 0,
            Self::Float(f) => *f != 0.0,
            Self::Str(s) => !s.is_empty(),
            Self::Tuple(items) => !items.is_empty(),
            Self::Prop(_) => true,
        }
    }

    fn from_const(c: &ConstValue) -> Self {
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
        let mut name = get_kwarg_str(kwargs, "title")
            .or_else(|| get_kwarg_str(kwargs, "name"))
            .unwrap_or_else(|| "Main".to_string());

        let mut child_start_idx = 0;
        if let Some(first) = args.first()
            && let Some(s) = eval_constant(first).and_then(|c| c.as_str().map(String::from))
        {
            name = s;
            child_start_idx = 1;
        }

        let padding = get_kwarg_f64(kwargs, "padding").unwrap_or(16.0);
        let gap = get_kwarg_f64(kwargs, "gap").unwrap_or(12.0);
        let width = get_kwarg_f64(kwargs, "width");

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
                        let gap = get_kwarg_f64(kwargs, "gap").unwrap_or(8.0);
                        let padding = get_kwarg_f64(kwargs, "padding").unwrap_or(0.0);
                        let children = self.collect_children(args, kwargs);
                        vec![UiNode::Row {
                            children,
                            gap,
                            padding,
                        }]
                    }

                    "Col" | "Column" | "VStack" | "Div" => {
                        let gap = get_kwarg_f64(kwargs, "gap").unwrap_or(8.0);
                        let padding = get_kwarg_f64(kwargs, "padding").unwrap_or(0.0);
                        let children = self.collect_children(args, kwargs);
                        vec![UiNode::Col {
                            children,
                            gap,
                            padding,
                        }]
                    }

                    "GroupBox" | "Section" | "Card" | "Panel" => {
                        let mut title = get_kwarg_str(kwargs, "title")
                            .or_else(|| get_kwarg_str(kwargs, "text"))
                            .unwrap_or_default();
                        let mut child_start = 0;
                        if let Some(first) = args.first()
                            && let Some(s) =
                                eval_constant(first).and_then(|c| c.as_str().map(String::from))
                        {
                            title = s;
                            child_start = 1;
                        }

                        let dir_str =
                            get_kwarg_str(kwargs, "direction").unwrap_or_else(|| "col".to_string());
                        let direction = match dir_str.to_ascii_lowercase().as_str() {
                            "row" | "horizontal" | "hstack" => FlexDirection::Row,
                            _ => FlexDirection::Column,
                        };
                        let gap = get_kwarg_f64(kwargs, "gap").unwrap_or(10.0);
                        let padding = get_kwarg_f64(kwargs, "padding").unwrap_or(12.0);
                        let width = get_kwarg_f64(kwargs, "width");
                        let height = get_kwarg_f64(kwargs, "height");
                        let color = get_kwarg_color(kwargs, "color")
                            .or_else(|| get_kwarg_color(kwargs, "fill"));
                        let stroke = get_kwarg_u32(kwargs, "stroke")
                            .or_else(|| get_kwarg_u32(kwargs, "stroke_width"))
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
                            .and_then(eval_constant)
                            .and_then(|c| c.as_str().map(String::from))
                            .or_else(|| get_kwarg_str(kwargs, "text"))
                            .or_else(|| get_kwarg_str(kwargs, "title"))
                            .unwrap_or_default();
                        let width = get_kwarg_f64(kwargs, "width");
                        let height = get_kwarg_f64(kwargs, "height").unwrap_or(28.0);
                        let font_size = get_kwarg_u32(kwargs, "font_size").unwrap_or(14);
                        let color = get_kwarg_color(kwargs, "color");
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
                            .and_then(eval_constant)
                            .and_then(|c| c.as_str().map(String::from))
                            .or_else(|| get_kwarg_str(kwargs, "text"))
                            .unwrap_or_default();
                        let width = get_kwarg_f64(kwargs, "width");
                        let height = get_kwarg_f64(kwargs, "height").unwrap_or(22.0);
                        let font_size = get_kwarg_u32(kwargs, "font_size").unwrap_or(12);
                        let h_align =
                            get_kwarg_str(kwargs, "align").unwrap_or_else(|| "Left".to_string());
                        let color = get_kwarg_color(kwargs, "color");
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
                            .and_then(eval_constant)
                            .and_then(|c| c.as_f64())
                            .or_else(|| get_kwarg_f64(kwargs, "width"))
                            .or_else(|| get_kwarg_f64(kwargs, "size"))
                            .unwrap_or(12.0);
                        let height = args
                            .get(1)
                            .and_then(eval_constant)
                            .and_then(|c| c.as_f64())
                            .or_else(|| get_kwarg_f64(kwargs, "height"))
                            .or_else(|| get_kwarg_f64(kwargs, "size"))
                            .unwrap_or(12.0);
                        vec![UiNode::Spacer { width, height }]
                    }

                    "Field" => self.parse_field_call(args, kwargs, expr.span),

                    "Control" => self.parse_control_call(args, kwargs, expr.span),

                    other => {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: format!("UI.{other}"),
                                message: format!(
                                    "Unknown UI component 'UI.{other}'. Expected Page, GroupBox, Row, Col, Header, Label, Field, Control, or Spacer"
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
                    self.build_control_nodes(ctl, sub_idx, &[], None, expr.span)
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

    fn parse_field_call(
        &mut self,
        args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
        span: Span,
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
            self.build_control_nodes(ctl, sub_idx, kwargs, label_spec, span)
        } else {
            Vec::new()
        }
    }

    fn parse_control_call(
        &mut self,
        args: &[ast::Expr],
        kwargs: &[(String, ast::Expr)],
        span: Span,
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
            self.build_control_nodes(ctl, sub_idx, kwargs, explicit_label, span)
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
    ) -> Vec<UiNode> {
        let explicit_style = get_kwarg_str(kwargs, "style").map(|s| ControlStyle::from_name(&s));
        let type_property = if explicit_style.is_none() {
            ctl.def
                .generic_spec
                .as_ref()
                .map(|g| g.property_name.clone())
        } else {
            None
        };
        let style = explicit_style.unwrap_or_else(|| ControlStyle::default_for_kind(&ctl.def.kind));

        let label_pos_str = get_kwarg_str(kwargs, "label_pos").unwrap_or_else(|| "top".to_string());
        let label_width = get_kwarg_f64(kwargs, "label_width").unwrap_or(88.0);
        let label_placement = match label_pos_str.to_ascii_lowercase().as_str() {
            "left" | "inline" | "side" => LabelPlacement::Left { width: label_width },
            _ => LabelPlacement::Top,
        };

        let pretty_spec = kwargs
            .iter()
            .find(|(k, _)| k == "pretty_name")
            .and_then(|(_, v)| self.parse_label_spec(v, "pretty_name"));
        let legend = get_kwarg_str(kwargs, "legend");
        let size_tuple = get_kwarg_size(kwargs, "size");
        let width = get_kwarg_f64(kwargs, "width").or_else(|| size_tuple.map(|(w, _)| w));
        let height = get_kwarg_f64(kwargs, "height").or_else(|| size_tuple.map(|(_, h)| h));
        let color = get_kwarg_color(kwargs, "color");
        let margin = get_kwarg_f64(kwargs, "margin");
        let read_only = get_kwarg_bool(kwargs, "read_only").unwrap_or(false);
        let invisible = get_kwarg_bool(kwargs, "invisible").unwrap_or(false);

        let kwarg_idx = kwargs
            .iter()
            .find(|(k, _)| k == "index")
            .and_then(|(_, v)| eval_constant(v))
            .and_then(|c| c.as_i64());

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
                is_dynamic_repeat_head: false,
                skip_lua_emit: false,
            })];
        }

        // Multi-channel or property-counted control with a specific 0-based channel index (`ChGain[0]`)
        if let Some(idx) = effective_idx {
            let ch = idx + 1;
            let idx_str = idx.to_string();
            let qsys_key = format!("{} {}", ctl.def.name, ch);
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
                style,
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
                is_dynamic_repeat_head: false,
                skip_lua_emit: false,
            })];
        }

        // Multi-channel or property-counted control referenced as a group (no index specified):
        // Automatically expand `1..=count` channels for preview, and if `count_property` is set,
        // mark channel 1 as a dynamic repeat head for `for i = 1, props["..."].Value do` in Lua!
        let explicit_labels = get_kwarg_str_list(kwargs, "labels");
        let explicit_pretty_names = get_kwarg_str_list(kwargs, "pretty_names");
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
                style: style.clone(),
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
                is_dynamic_repeat_head: has_dynamic_repeat && ch == 1,
                skip_lua_emit: has_dynamic_repeat && ch > 1,
            }));
        }

        if let Some(dir_str) = get_kwarg_str(kwargs, "direction") {
            let gap = get_kwarg_f64(kwargs, "gap").unwrap_or(8.0);
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

    fn lower_ui_expr_to_lua(
        &self,
        expr: &ast::Expr,
        local_map: &FxHashMap<String, String>,
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
                if let Some(prop) = self.env.properties.iter().find(|p| p.ident == *name) {
                    return format!("props[\"{}\"]", escape_lua_str_content(&prop.def.name));
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
                        && (obj_name == "self" || obj_name == self.class_name)
                        && let Some(prop) =
                            self.env.properties.iter().find(|p| p.ident == *prop_ident)
                    {
                        return format!(
                            "props[\"{}\"].Value",
                            escape_lua_str_content(&prop.def.name)
                        );
                    }
                    if let ast::ExprKind::Ident(prop_ident) = &value.kind
                        && let Some(prop) =
                            self.env.properties.iter().find(|p| p.ident == *prop_ident)
                    {
                        return format!(
                            "props[\"{}\"].Value",
                            escape_lua_str_content(&prop.def.name)
                        );
                    }
                }
                if let ast::ExprKind::Ident(obj_name) = &value.kind
                    && (obj_name == "self" || obj_name == self.class_name)
                    && let Some(prop) = self.env.properties.iter().find(|p| p.ident == *attr)
                {
                    return format!("props[\"{}\"]", escape_lua_str_content(&prop.def.name));
                }
                let base = self.lower_ui_expr_to_lua(value, local_map);
                format!("{base}.{attr}")
            }
            ast::ExprKind::Subscript { value, index } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "props" || obj == "Properties")
                {
                    let idx_lua = self.lower_ui_expr_to_lua(index, local_map);
                    return format!("props[{idx_lua}]");
                }
                let base = self.lower_ui_expr_to_lua(value, local_map);
                let idx_lua = self.lower_ui_expr_to_lua(index, local_map);
                format!("{base}[({idx_lua}) + 1]")
            }
            ast::ExprKind::Unary { op, operand } => {
                let inner = self.lower_ui_expr_to_lua(operand, local_map);
                match op {
                    ast::UnaryOp::Neg => format!("(-{inner})"),
                    ast::UnaryOp::Pos => inner,
                    ast::UnaryOp::Not => format!("(not {inner})"),
                    ast::UnaryOp::BitNot => format!("(~{inner})"),
                }
            }
            ast::ExprKind::Binary { left, op, right } => {
                let l = self.lower_ui_expr_to_lua(left, local_map);
                let r = self.lower_ui_expr_to_lua(right, local_map);
                let is_str = |e: &ast::Expr| -> bool {
                    matches!(e.kind, ast::ExprKind::String(_))
                        || matches!(
                            &e.kind,
                            ast::ExprKind::Call { callee, .. }
                                if matches!(&callee.kind, ast::ExprKind::Ident(n) if n == "str" || n == "tostring")
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
            ast::ExprKind::Call { callee, args, .. } => {
                let rendered_args: Vec<String> = args
                    .iter()
                    .map(|a| self.lower_ui_expr_to_lua(a, local_map))
                    .collect();
                if let ast::ExprKind::Ident(fn_name) = &callee.kind {
                    if fn_name == "str" && rendered_args.len() == 1 {
                        return format!("tostring({})", rendered_args[0]);
                    }
                    if fn_name == "int" && rendered_args.len() == 1 {
                        return format!("math.floor(tonumber({}) or 0)", rendered_args[0]);
                    }
                    if !self.env.local_functions.contains_key(fn_name)
                        && !self.env.module_functions.contains_key(fn_name)
                        && self.env.class_methods.contains_key(fn_name)
                    {
                        let mut all_args = vec!["nil".to_string()];
                        all_args.extend(rendered_args);
                        return format!("{}_{fn_name}({})", self.class_name, all_args.join(", "));
                    }
                    return format!("{fn_name}({})", rendered_args.join(", "));
                }
                if let ast::ExprKind::Attribute { value, attr } = &callee.kind
                    && let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                {
                    let mut all_args = vec!["nil".to_string()];
                    all_args.extend(rendered_args);
                    return format!("{}_{attr}({})", self.class_name, all_args.join(", "));
                }
                let callee_lua = self.lower_ui_expr_to_lua(callee, local_map);
                format!("{callee_lua}({})", rendered_args.join(", "))
            }
            _ => "\"\"".to_string(),
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

    fn eval_preview_function(
        &self,
        func: &ast::FunctionDef,
        args: &[PreviewVal],
        depth: usize,
    ) -> Option<PreviewVal> {
        if depth > 8 {
            return None;
        }
        let mut vars = FxHashMap::default();
        for (name, val) in &self.env.constants {
            vars.insert(name.clone(), PreviewVal::from_const(val));
        }
        let skip_self = usize::from(func.params.first().is_some_and(|p| p.name == "self"));
        for (param, arg_val) in func.params.iter().skip(skip_self).zip(args.iter()) {
            vars.insert(param.name.clone(), arg_val.clone());
        }
        self.eval_preview_stmts(&func.body, &mut vars, depth)
    }

    fn eval_preview_stmts(
        &self,
        stmts: &[ast::Stmt],
        vars: &mut FxHashMap<String, PreviewVal>,
        depth: usize,
    ) -> Option<PreviewVal> {
        for stmt in stmts {
            match &stmt.kind {
                ast::StmtKind::Pass | ast::StmtKind::Global(_) | ast::StmtKind::Nonlocal(_) => {}
                ast::StmtKind::Expr(e) if matches!(e.kind, ast::ExprKind::String(_)) => {}
                ast::StmtKind::Assign { target, value, .. } => {
                    let val = self.eval_preview_expr(value, vars, depth)?;
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
                    let cond = self.eval_preview_expr(condition, vars, depth)?;
                    let branch = if cond.is_truthy() {
                        then_branch
                    } else {
                        else_branch
                    };
                    if let Some(ret) = self.eval_preview_stmts(branch, vars, depth) {
                        return Some(ret);
                    }
                }
                ast::StmtKind::Return(Some(ret_expr)) => {
                    return self.eval_preview_expr(ret_expr, vars, depth);
                }
                ast::StmtKind::Return(None) => return None,
                _ => {}
            }
        }
        None
    }

    fn lookup_property_default(&self, ident_or_name: &str) -> Option<PreviewVal> {
        let prop = self
            .env
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

    fn eval_preview_expr(
        &self,
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
                    out.push(self.eval_preview_expr(item, vars, depth)?);
                }
                Some(PreviewVal::Tuple(out))
            }
            ast::ExprKind::Ident(name) => {
                if let Some(v) = vars.get(name) {
                    return Some(v.clone());
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
                        && (obj_name == "self" || obj_name == self.class_name)
                    {
                        return self.lookup_property_default(prop_ident);
                    }
                    if let ast::ExprKind::Ident(prop_ident) = &value.kind
                        && let Some(val) = self.lookup_property_default(prop_ident)
                    {
                        return Some(val);
                    }
                    if let Some(PreviewVal::Prop(prop_name)) =
                        self.eval_preview_expr(value, vars, depth)
                    {
                        return self.lookup_property_default(&prop_name);
                    }
                }
                if let ast::ExprKind::Ident(obj_name) = &value.kind
                    && (obj_name == "self" || obj_name == self.class_name)
                {
                    if let Some(val) = self.lookup_property_default(attr) {
                        return Some(val);
                    }
                    if let Some(cval) = self.env.constants.get(attr) {
                        return Some(PreviewVal::from_const(cval));
                    }
                }
                None
            }
            ast::ExprKind::Subscript { value, index } => {
                if let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "props" || obj == "Properties")
                    && let Some(PreviewVal::Str(prop_name)) =
                        self.eval_preview_expr(index, vars, depth)
                {
                    return Some(PreviewVal::Prop(prop_name));
                }
                let container = self.eval_preview_expr(value, vars, depth)?;
                let idx = self.eval_preview_expr(index, vars, depth)?.as_i64()?;
                if let PreviewVal::Tuple(items) = container
                    && idx >= 0
                {
                    return items.get(idx as usize).cloned();
                }
                None
            }
            ast::ExprKind::Unary { op, operand } => {
                let val = self.eval_preview_expr(operand, vars, depth)?;
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
                let l = self.eval_preview_expr(left, vars, depth)?;
                let r = self.eval_preview_expr(right, vars, depth)?;
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
                    (PreviewVal::Int(a), ast::BinaryOp::Div, PreviewVal::Int(b)) if b != 0 => {
                        Some(PreviewVal::Float(a as f64 / b as f64))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::Eq, PreviewVal::Int(b)) => {
                        Some(PreviewVal::Bool(a == b))
                    }
                    (PreviewVal::Int(a), ast::BinaryOp::NotEq, PreviewVal::Int(b)) => {
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
                    (a, ast::BinaryOp::Add, b) => {
                        Some(PreviewVal::Float(a.as_f64()? + b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Sub, b) => {
                        Some(PreviewVal::Float(a.as_f64()? - b.as_f64()?))
                    }
                    (a, ast::BinaryOp::Mul, b) => {
                        Some(PreviewVal::Float(a.as_f64()? * b.as_f64()?))
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
            ast::ExprKind::Call { callee, args, .. } => {
                let mut eval_args = Vec::with_capacity(args.len());
                for arg in args {
                    eval_args.push(self.eval_preview_expr(arg, vars, depth)?);
                }
                if let ast::ExprKind::Ident(fn_name) = &callee.kind {
                    if fn_name == "str" && eval_args.len() == 1 {
                        return Some(PreviewVal::Str(eval_args[0].to_display_string()));
                    }
                    if fn_name == "int" && eval_args.len() == 1 {
                        return eval_args[0].as_i64().map(PreviewVal::Int);
                    }
                    let func = self
                        .env
                        .local_functions
                        .get(fn_name)
                        .or_else(|| self.env.class_methods.get(fn_name))
                        .or_else(|| self.env.module_functions.get(fn_name))?;
                    return self.eval_preview_function(func, &eval_args, depth + 1);
                }
                if let ast::ExprKind::Attribute { value, attr } = &callee.kind
                    && let ast::ExprKind::Ident(obj) = &value.kind
                    && (obj == "self" || obj == self.class_name)
                {
                    let method = self.env.class_methods.get(attr)?;
                    return self.eval_preview_function(method, &eval_args, depth + 1);
                }
                None
            }
            _ => None,
        }
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
                let (ctl, _) = self.resolve_control_ref(value)?;
                let Some(idx_val) = eval_constant(index).and_then(|c| c.as_i64()) else {
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

fn get_kwarg_str_list(kwargs: &[(String, ast::Expr)], key: &str) -> Option<Vec<String>> {
    let (_, expr) = kwargs.iter().find(|(k, _)| k == key)?;
    let ConstValue::List(items) = eval_constant(expr)? else {
        return None;
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(item.as_str()?.to_string());
    }
    Some(out)
}

fn get_kwarg_str(kwargs: &[(String, ast::Expr)], key: &str) -> Option<String> {
    kwargs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| eval_constant(v))
        .and_then(|c| c.as_str().map(String::from))
}

fn get_kwarg_f64(kwargs: &[(String, ast::Expr)], key: &str) -> Option<f64> {
    kwargs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| eval_constant(v))
        .and_then(|c| c.as_f64())
}

fn get_kwarg_u32(kwargs: &[(String, ast::Expr)], key: &str) -> Option<u32> {
    kwargs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| eval_constant(v))
        .and_then(|c| c.as_i64())
        .and_then(|i| u32::try_from(i).ok())
}

fn get_kwarg_bool(kwargs: &[(String, ast::Expr)], key: &str) -> Option<bool> {
    kwargs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| eval_constant(v))
        .and_then(|c| c.as_bool())
}

fn get_kwarg_size(kwargs: &[(String, ast::Expr)], key: &str) -> Option<(f64, f64)> {
    let (_, expr) = kwargs.iter().find(|(k, _)| k == key)?;
    if let ast::ExprKind::Tuple(elts) | ast::ExprKind::List(elts) = &expr.kind
        && elts.len() == 2
    {
        let w = eval_constant(&elts[0]).and_then(|c| c.as_f64())?;
        let h = eval_constant(&elts[1]).and_then(|c| c.as_f64())?;
        return Some((w, h));
    }
    None
}

fn get_kwarg_color(kwargs: &[(String, ast::Expr)], key: &str) -> Option<Color> {
    let (_, expr) = kwargs.iter().find(|(k, _)| k == key)?;
    if let Some(s) = eval_constant(expr).and_then(|c| c.as_str().map(String::from)) {
        return Some(Color::parse_str(&s));
    }
    if let ast::ExprKind::Tuple(elts) | ast::ExprKind::List(elts) = &expr.kind
        && elts.len() == 3
    {
        let r = eval_constant(&elts[0])
            .and_then(|c| c.as_i64())?
            .clamp(0, 255) as u8;
        let g = eval_constant(&elts[1])
            .and_then(|c| c.as_i64())?
            .clamp(0, 255) as u8;
        let b = eval_constant(&elts[2])
            .and_then(|c| c.as_i64())?
            .clamp(0, 255) as u8;
        return Some(Color::rgb(r, g, b));
    }
    None
}
