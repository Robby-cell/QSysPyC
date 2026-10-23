use strsim::levenshtein;

use qsys_diagnostics::{DiagnosticKind, Diagnostics};
use qsys_model::{ControlMember, PropertyKind};
use qsys_parser::ast;
use qsys_runtime::{ExecutionContext, GlobalKind, Member, Returns, RuntimeEnv};

use crate::binding::find_closest_member;
use crate::symbols::{ScopeKind, ScopeStack, Symbol, SymbolKind};
use crate::types::{ResolvedTarget, Type, TypeMap};

/// Names owned by the QSysPy declaration DSL (`class X(QSysPlugin)`, `Control.Float(...)`).
/// Everything else in the global scope comes from the `RuntimeEnv`.
const DSL_GLOBALS: &[&str] = &["Control", "Property", "QSysPlugin", "TextController"];

fn find_closest_match(target: &str, candidates: &[String]) -> Option<String> {
    let mut best_match = None;
    let mut min_distance = usize::MAX;

    for candidate in candidates {
        let dist = levenshtein(target, candidate);
        if dist > 0 && dist <= 2 && dist < min_distance {
            min_distance = dist;
            best_match = Some(candidate.clone());
        }
    }

    best_match
}

fn extract_expr_name(expr: &ast::Expr) -> String {
    match &expr.kind {
        ast::ExprKind::Ident(name) => name.clone(),
        ast::ExprKind::Attribute { attr, .. } => attr.clone(),
        _ => "<control>".to_string(),
    }
}

pub struct ExprAnalyzer<'a> {
    pub scope_stack: &'a mut ScopeStack,
    pub type_map: &'a mut TypeMap,
    pub diagnostics: &'a mut Diagnostics,
    pub env: &'a RuntimeEnv,
    /// Execution phase of the code being analyzed, used to enforce global availability.
    pub context: ExecutionContext,
}

impl<'a> ExprAnalyzer<'a> {
    pub fn new(
        scope_stack: &'a mut ScopeStack,
        type_map: &'a mut TypeMap,
        diagnostics: &'a mut Diagnostics,
        env: &'a RuntimeEnv,
    ) -> Self {
        Self {
            scope_stack,
            type_map,
            diagnostics,
            env,
            context: ExecutionContext::Unknown,
        }
    }

    pub fn with_context(mut self, context: ExecutionContext) -> Self {
        self.context = context;
        self
    }

    pub fn analyze_statements(&mut self, stmts: &[ast::Stmt]) {
        for stmt in stmts {
            self.analyze_stmt(stmt);
        }
    }

    pub fn analyze_stmt(&mut self, stmt: &ast::Stmt) {
        match &stmt.kind {
            ast::StmtKind::Assign { target, value } => {
                let val_ty = self.infer_expr(value);
                match &target.kind {
                    ast::ExprKind::Ident(name) => {
                        let sym_id = if let Some(existing) = self.scope_stack.lookup(name) {
                            match existing.kind {
                                SymbolKind::Variable { id, .. }
                                | SymbolKind::Parameter { id, .. } => id,
                                _ => self.scope_stack.alloc_symbol_id(),
                            }
                        } else {
                            self.scope_stack.alloc_symbol_id()
                        };
                        self.scope_stack.define(Symbol::new(
                            sym_id,
                            name.clone(),
                            SymbolKind::Variable {
                                id: sym_id,
                                ty: val_ty.clone(),
                            },
                            target.span,
                        ));
                        self.type_map
                            .insert_resolution(target.span, ResolvedTarget::Local(sym_id));
                        self.type_map.insert_type(target.span, val_ty);
                        self.type_map.insert_symbol_name(sym_id, name.clone());
                    }
                    ast::ExprKind::Subscript {
                        value: sub_val,
                        index: sub_idx,
                    } => {
                        let sub_ty = self.infer_expr(sub_val);
                        let _ = self.infer_expr(sub_idx);
                        self.type_map.insert_type(target.span, sub_ty);
                    }
                    ast::ExprKind::Attribute {
                        value: obj_val,
                        attr,
                    } => {
                        if let ast::ExprKind::Ident(obj_name) = &obj_val.kind
                            && let Some(sym) = self.scope_stack.lookup(obj_name)
                            && let SymbolKind::Variable {
                                ty: Type::UserInstance { ref name },
                                ..
                            }
                            | SymbolKind::Parameter {
                                ty: Type::UserInstance { ref name },
                                ..
                            } = sym.kind
                        {
                            self.type_map.insert_class_member_type(
                                name.clone(),
                                attr.clone(),
                                val_ty,
                            );
                        }
                        let _ = self.infer_expr(target);
                    }
                    _ => {
                        let _ = self.infer_expr(target);
                    }
                }
            }

            ast::StmtKind::AugAssign { target, value, .. } => {
                let _ = self.infer_expr(target);
                let _ = self.infer_expr(value);
            }

            ast::StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.infer_expr(condition);
                self.type_map.insert_type(condition.span, cond_ty);

                self.scope_stack.push_scope(ScopeKind::Block);
                self.analyze_statements(then_branch);
                self.scope_stack.pop_scope();

                if !else_branch.is_empty() {
                    self.scope_stack.push_scope(ScopeKind::Block);
                    self.analyze_statements(else_branch);
                    self.scope_stack.pop_scope();
                }
            }

            ast::StmtKind::While { condition, body } => {
                let cond_ty = self.infer_expr(condition);
                self.type_map.insert_type(condition.span, cond_ty);

                self.scope_stack.push_scope(ScopeKind::Block);
                self.analyze_statements(body);
                self.scope_stack.pop_scope();
            }

            ast::StmtKind::For {
                target,
                iterable,
                body,
            } => {
                let iter_ty = self.infer_expr(iterable);
                let elem_ty = match iter_ty {
                    Type::List(elem) => *elem,
                    Type::ControlHandle => Type::ControlHandle,
                    _ => Type::Any,
                };

                let sym_id = self.scope_stack.alloc_symbol_id();
                self.type_map.insert_symbol_name(sym_id, target.clone());
                self.scope_stack.push_scope(ScopeKind::Block);
                self.scope_stack.define(Symbol::new(
                    sym_id,
                    target.clone(),
                    SymbolKind::Variable {
                        id: sym_id,
                        ty: elem_ty,
                    },
                    stmt.span,
                ));
                self.type_map
                    .insert_resolution(stmt.span, ResolvedTarget::Local(sym_id));

                self.analyze_statements(body);
                self.scope_stack.pop_scope();
            }

            ast::StmtKind::Return(Some(expr)) => {
                let _ = self.infer_expr(expr);
            }

            ast::StmtKind::Expr(expr) => {
                let _ = self.infer_expr(expr);
            }

            ast::StmtKind::FunctionDef(f) => {
                let func_id = self.scope_stack.alloc_function_id();
                let sym_id = self.scope_stack.alloc_symbol_id();

                let enclosing_class = match &self.scope_stack.current_scope().kind {
                    ScopeKind::Class { name } => Some(name.clone()),
                    _ => None,
                };

                self.scope_stack.define(Symbol::new(
                    sym_id,
                    f.name.clone(),
                    SymbolKind::Method {
                        id: func_id,
                        param_count: f.params.len(),
                        has_self: f.params.first().is_some_and(|p| p.name == "self"),
                    },
                    f.span,
                ));
                self.type_map
                    .insert_resolution(f.span, ResolvedTarget::Function(func_id));

                self.scope_stack.push_scope(ScopeKind::Function {
                    name: f.name.clone(),
                });

                for (idx, param) in f.params.iter().enumerate() {
                    let param_sym_id = self.scope_stack.alloc_symbol_id();
                    let param_ty = if idx == 0 && param.name == "self" {
                        if let Some(ref cls_name) = enclosing_class {
                            Type::UserInstance {
                                name: cls_name.clone(),
                            }
                        } else {
                            Type::Any
                        }
                    } else {
                        crate::type_from_annotation(param.type_annotation.as_deref(), self.env)
                    };
                    self.type_map
                        .insert_symbol_name(param_sym_id, param.name.clone());
                    self.type_map
                        .insert_resolution(param.span, ResolvedTarget::Local(param_sym_id));
                    self.type_map.insert_type(param.span, param_ty.clone());
                    self.scope_stack.define(Symbol::new(
                        param_sym_id,
                        param.name.clone(),
                        SymbolKind::Parameter {
                            id: param_sym_id,
                            index: idx,
                            ty: param_ty,
                        },
                        param.span,
                    ));
                }

                // A nested function body only runs when called, so its execution phase is unknown.
                let outer_context = std::mem::replace(&mut self.context, ExecutionContext::Unknown);
                self.analyze_statements(&f.body);
                self.context = outer_context;
                self.scope_stack.pop_scope();

                let mut ret_ty = crate::type_from_annotation(f.return_type.as_deref(), self.env);
                if ret_ty == Type::Any {
                    for body_stmt in &f.body {
                        if let ast::StmtKind::Return(Some(ret_expr)) = &body_stmt.kind
                            && let Some(inferred) = self.type_map.get_type(ret_expr.span)
                            && *inferred != Type::Any
                        {
                            ret_ty = inferred.clone();
                            break;
                        }
                    }
                }

                if let Some(cls_name) = enclosing_class {
                    self.type_map
                        .insert_class_member_type(cls_name, f.name.clone(), ret_ty);
                } else {
                    self.type_map
                        .insert_function_return_type(f.name.clone(), ret_ty);
                }
            }

            ast::StmtKind::ClassDef(c) => {
                let sym_id = self.scope_stack.alloc_symbol_id();

                if let Some(base_name) = c.bases.first()
                    && !matches!(
                        base_name.as_str(),
                        "QSysPlugin" | "TextController" | "object"
                    )
                {
                    self.type_map
                        .insert_class_base(c.name.clone(), base_name.clone());
                }

                self.scope_stack.define(Symbol::new(
                    sym_id,
                    c.name.clone(),
                    SymbolKind::Class {
                        name: c.name.clone(),
                        base: c.bases.first().cloned(),
                    },
                    c.span,
                ));

                // Pre-register methods on the class so intra-class `self.method()` calls resolve cleanly
                for body_stmt in &c.body {
                    if let ast::StmtKind::FunctionDef(f) = &body_stmt.kind {
                        let annotated_ret =
                            crate::type_from_annotation(f.return_type.as_deref(), self.env);
                        self.type_map.insert_class_member_type(
                            c.name.clone(),
                            f.name.clone(),
                            annotated_ret,
                        );
                    }
                }

                self.scope_stack.push_scope(ScopeKind::Class {
                    name: c.name.clone(),
                });
                self.analyze_statements(&c.body);
                self.scope_stack.pop_scope();
            }

            _ => {}
        }
    }

    pub fn infer_expr(&mut self, expr: &ast::Expr) -> Type {
        let ty = match &expr.kind {
            ast::ExprKind::Int(_) => Type::Int,
            ast::ExprKind::Float(_) => Type::Float,
            ast::ExprKind::String(_) => Type::String,
            ast::ExprKind::Bool(_) => Type::Bool,
            ast::ExprKind::None => Type::None,

            ast::ExprKind::Ident(name) => {
                if let Some(symbol) = self.scope_stack.lookup(name) {
                    match &symbol.kind {
                        SymbolKind::Variable { id, ty } | SymbolKind::Parameter { id, ty, .. } => {
                            self.type_map
                                .insert_resolution(expr.span, ResolvedTarget::Local(*id));
                            ty.clone()
                        }
                        SymbolKind::Control { id, .. } => {
                            self.type_map
                                .insert_resolution(expr.span, ResolvedTarget::Control(*id));
                            Type::ControlHandle
                        }
                        SymbolKind::Property { id, .. } => {
                            self.type_map
                                .insert_resolution(expr.span, ResolvedTarget::Property(*id));
                            Type::PropertyHandle
                        }
                        SymbolKind::Method { id, .. } => {
                            self.type_map
                                .insert_resolution(expr.span, ResolvedTarget::Function(*id));
                            Type::Any
                        }
                        SymbolKind::Class { name, .. } | SymbolKind::UserClass { name } => {
                            Type::UserClass { name: name.clone() }
                        }
                    }
                } else if let Some(global) = self.env.global(name) {
                    if !global.availability.permits(self.context) {
                        self.diagnostics.report(
                            DiagnosticKind::UnavailableInContext {
                                name: name.clone(),
                                available: global.availability.describe().to_string(),
                                context: self.context.describe().to_string(),
                            },
                            expr.span,
                        );
                    }
                    self.type_map
                        .insert_resolution(expr.span, ResolvedTarget::Global(name.clone()));
                    match &global.kind {
                        GlobalKind::Value(ty) => Type::from_ref(ty),
                        GlobalKind::Function(_) => Type::Any,
                    }
                } else if DSL_GLOBALS.contains(&name.as_str()) {
                    self.type_map
                        .insert_resolution(expr.span, ResolvedTarget::Global(name.clone()));
                    Type::Any
                } else if let Some(reason) = self.env.removed_reason(name) {
                    self.diagnostics.report(
                        DiagnosticKind::RemovedBuiltin {
                            name: name.clone(),
                            reason: reason.to_string(),
                        },
                        expr.span,
                    );
                    Type::Any
                } else {
                    let mut visible = self.scope_stack.all_visible_names();
                    visible.extend(self.env.global_names().map(str::to_string));
                    visible.extend(DSL_GLOBALS.iter().map(|g| (*g).to_string()));
                    let candidates = find_closest_match(name, &visible)
                        .map(|c| vec![c])
                        .unwrap_or_default();

                    self.diagnostics.report(
                        DiagnosticKind::UndefinedVariable {
                            name: name.clone(),
                            candidates,
                        },
                        expr.span,
                    );
                    Type::Any
                }
            }

            ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
                let elem_ty = if let Some(first) = items.first() {
                    self.infer_expr(first)
                } else {
                    Type::Any
                };
                for item in items.iter().skip(1) {
                    let _ = self.infer_expr(item);
                }
                Type::List(Box::new(elem_ty))
            }

            ast::ExprKind::Dict(pairs) => {
                let (key_ty, val_ty) = if let Some((k, v)) = pairs.first() {
                    (self.infer_expr(k), self.infer_expr(v))
                } else {
                    (Type::Any, Type::Any)
                };
                for (k, v) in pairs.iter().skip(1) {
                    let _ = self.infer_expr(k);
                    let _ = self.infer_expr(v);
                }
                Type::Dict(Box::new(key_ty), Box::new(val_ty))
            }

            ast::ExprKind::Attribute { value, attr } => {
                let base_ty = self.infer_expr(value);

                // Support `self.<Property>.Value` by propagating the property resolution
                if attr == "Value"
                    && let Some(ResolvedTarget::Property(prop_id)) =
                        self.type_map.get_resolution(value.span).cloned()
                {
                    self.type_map
                        .insert_resolution(expr.span, ResolvedTarget::Property(prop_id));
                    self.type_map.insert_type(expr.span, base_ty.clone());
                    return base_ty;
                }

                match base_ty {
                    Type::ControlHandle => match ControlMember::from_ident(attr) {
                        Some(ControlMember::Value) | Some(ControlMember::RampTime) => Type::Float,
                        Some(ControlMember::String)
                        | Some(ControlMember::Color)
                        | Some(ControlMember::Legend) => Type::String,
                        Some(ControlMember::Boolean)
                        | Some(ControlMember::IsDisabled)
                        | Some(ControlMember::IsInvisible) => Type::Bool,
                        Some(ControlMember::Values) => Type::List(Box::new(Type::Float)),
                        Some(ControlMember::Strings) => Type::List(Box::new(Type::String)),
                        Some(ControlMember::Data)
                        | Some(ControlMember::Generic)
                        | Some(ControlMember::Any)
                        | Some(ControlMember::EventHandler) => Type::Any,
                        None => {
                            let control_name = extract_expr_name(value);
                            let candidates = find_closest_member(attr)
                                .map(|c| vec![c.to_string()])
                                .unwrap_or_default();

                            self.diagnostics.report(
                                DiagnosticKind::UnknownControlMember {
                                    control_name,
                                    member: attr.clone(),
                                    candidates,
                                },
                                expr.span,
                            );
                            Type::Any
                        }
                    },
                    Type::ClassInstance { ref name } => {
                        if let Some(symbol) = self.scope_stack.lookup(attr) {
                            match &symbol.kind {
                                SymbolKind::Control { id, .. } => {
                                    self.type_map
                                        .insert_resolution(expr.span, ResolvedTarget::Control(*id));
                                    Type::ControlHandle
                                }
                                SymbolKind::Property { id, def } => {
                                    self.type_map.insert_resolution(
                                        expr.span,
                                        ResolvedTarget::Property(*id),
                                    );
                                    match def.kind {
                                        PropertyKind::Integer { .. } => Type::Int,
                                        PropertyKind::Double { .. } => Type::Float,
                                        PropertyKind::String { .. } | PropertyKind::Enum { .. } => {
                                            Type::String
                                        }
                                        PropertyKind::Boolean { .. } => Type::Bool,
                                    }
                                }
                                SymbolKind::Method { id, .. } => {
                                    self.type_map.insert_resolution(
                                        expr.span,
                                        ResolvedTarget::Function(*id),
                                    );
                                    Type::Any
                                }
                                SymbolKind::Variable { id, ty } => {
                                    self.type_map
                                        .insert_resolution(expr.span, ResolvedTarget::Local(*id));
                                    ty.clone()
                                }
                                _ => Type::Any,
                            }
                        } else if matches!(
                            attr.as_str(),
                            "name"
                                | "version"
                                | "author"
                                | "description"
                                | "id"
                                | "manufacturer"
                                | "show_debug"
                        ) {
                            if attr == "show_debug" {
                                Type::Bool
                            } else {
                                Type::String
                            }
                        } else {
                            let class_members = self.scope_stack.class_member_names();
                            let candidates = find_closest_match(attr, &class_members)
                                .map(|c| vec![c])
                                .unwrap_or_default();

                            self.diagnostics.report(
                                DiagnosticKind::UnknownClassMember {
                                    class_name: name.clone(),
                                    member: attr.clone(),
                                    candidates,
                                },
                                expr.span,
                            );
                            Type::Any
                        }
                    }
                    Type::UserInstance { ref name } => {
                        if let Some(member_ty) =
                            self.type_map.get_class_member_type(name, attr).cloned()
                        {
                            member_ty
                        } else {
                            let known_members = self.type_map.class_member_names(name);
                            if !known_members.is_empty() {
                                let candidates = find_closest_match(attr, &known_members)
                                    .map(|c| vec![c])
                                    .unwrap_or_default();
                                self.diagnostics.report(
                                    DiagnosticKind::UnknownClassMember {
                                        class_name: name.clone(),
                                        member: attr.clone(),
                                        candidates,
                                    },
                                    expr.span,
                                );
                            }
                            Type::Any
                        }
                    }
                    Type::Native { ref name } => match self.env.member(name, attr) {
                        Some(Member::Field(field)) => Type::from_ref(&field.ty),
                        Some(Member::Method(_)) | None => Type::Any,
                    },
                    _ => Type::Any,
                }
            }

            ast::ExprKind::Subscript { value, index } => {
                let val_ty = self.infer_expr(value);
                let _ = self.infer_expr(index);
                match val_ty {
                    Type::ControlHandle => Type::ControlHandle,
                    Type::List(elem) => *elem,
                    Type::Dict(_, val) => *val,
                    Type::String => Type::String,
                    _ => Type::Any,
                }
            }

            ast::ExprKind::Binary { left, op, right } => {
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);

                match op {
                    ast::BinaryOp::Eq
                    | ast::BinaryOp::NotEq
                    | ast::BinaryOp::Lt
                    | ast::BinaryOp::LtEq
                    | ast::BinaryOp::Gt
                    | ast::BinaryOp::GtEq => Type::Bool,

                    ast::BinaryOp::Add => {
                        if left_ty == Type::String || right_ty == Type::String {
                            Type::String
                        } else if left_ty == Type::Float || right_ty == Type::Float {
                            Type::Float
                        } else if left_ty == Type::Int && right_ty == Type::Int {
                            Type::Int
                        } else {
                            Type::Any
                        }
                    }

                    ast::BinaryOp::Mul => {
                        if (left_ty == Type::String && right_ty == Type::Int)
                            || (left_ty == Type::Int && right_ty == Type::String)
                        {
                            Type::String
                        } else if left_ty == Type::Float || right_ty == Type::Float {
                            Type::Float
                        } else if left_ty == Type::Int && right_ty == Type::Int {
                            Type::Int
                        } else {
                            Type::Any
                        }
                    }

                    ast::BinaryOp::Sub | ast::BinaryOp::Mod | ast::BinaryOp::Pow => {
                        if left_ty == Type::Float || right_ty == Type::Float {
                            Type::Float
                        } else if left_ty == Type::Int && right_ty == Type::Int {
                            Type::Int
                        } else {
                            Type::Any
                        }
                    }

                    ast::BinaryOp::Div => Type::Float,
                    ast::BinaryOp::FloorDiv
                    | ast::BinaryOp::BitAnd
                    | ast::BinaryOp::BitOr
                    | ast::BinaryOp::BitXor
                    | ast::BinaryOp::LShift
                    | ast::BinaryOp::RShift => Type::Int,
                }
            }

            ast::ExprKind::BoolOp { left, right, .. } => {
                let left_ty = self.infer_expr(left);
                let right_ty = self.infer_expr(right);
                if left_ty == right_ty {
                    left_ty
                } else {
                    Type::Any
                }
            }

            ast::ExprKind::Unary { op, operand } => {
                let operand_ty = self.infer_expr(operand);
                match op {
                    ast::UnaryOp::Not => Type::Bool,
                    ast::UnaryOp::Neg | ast::UnaryOp::Pos => {
                        if operand_ty.is_numeric() {
                            operand_ty
                        } else {
                            Type::Float
                        }
                    }
                    ast::UnaryOp::BitNot => Type::Int,
                }
            }

            ast::ExprKind::Call {
                callee,
                args,
                kwargs,
            } => {
                let callee_ty = self.infer_expr(callee);
                let mut arg_types = Vec::with_capacity(args.len());
                for arg in args {
                    arg_types.push(self.infer_expr(arg));
                }
                for (_, kw_val) in kwargs {
                    let _ = self.infer_expr(kw_val);
                }

                if is_qsys_lua_call(callee) {
                    if !kwargs.is_empty() {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidEscapeHatchCall {
                                message: "qsys.lua() does not accept keyword arguments".to_string(),
                            },
                            expr.span,
                        );
                    } else if args.len() != 1 {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidEscapeHatchCall {
                                message: format!(
                                    "qsys.lua() expects exactly 1 argument, but got {}",
                                    args.len()
                                ),
                            },
                            expr.span,
                        );
                    } else if !matches!(args[0].kind, ast::ExprKind::String(_)) {
                        self.diagnostics.report(
                            DiagnosticKind::InvalidEscapeHatchCall {
                                message: "qsys.lua() expects a raw string literal".to_string(),
                            },
                            args[0].span,
                        );
                    }
                }

                if let Type::UserClass { name } = callee_ty {
                    Type::UserInstance { name }
                } else if let ast::ExprKind::Attribute { value, attr } = &callee.kind {
                    let receiver_ty = self.type_map.get_type(value.span).cloned();
                    if let Some(Type::Native { ref name }) = receiver_ty {
                        match self.env.member(name, attr) {
                            Some(Member::Method(method)) => {
                                self.return_type_of(&method.returns, &arg_types)
                            }
                            Some(Member::Field(_)) | None => Type::Any,
                        }
                    } else if let Some(
                        Type::UserInstance { ref name } | Type::ClassInstance { ref name },
                    ) = receiver_ty
                        && let Some(ret_ty) =
                            self.type_map.get_class_member_type(name, attr).cloned()
                    {
                        ret_ty
                    } else if receiver_ty == Some(Type::String) {
                        match attr.as_str() {
                            "upper" | "lower" | "strip" | "lstrip" | "rstrip" | "replace"
                            | "gsub" | "sub" | "format" | "rep" => Type::String,
                            "startswith" | "endswith" => Type::Bool,
                            "find" | "byte" => Type::Int,
                            _ => Type::Any,
                        }
                    } else {
                        Type::Any
                    }
                } else if let ast::ExprKind::Ident(fn_name) = &callee.kind {
                    let is_runtime_global = matches!(
                        self.type_map.get_resolution(callee.span),
                        Some(ResolvedTarget::Global(_))
                    );
                    if is_runtime_global && fn_name == "super" {
                        if let Some(sym) = self.scope_stack.lookup("self")
                            && let SymbolKind::Variable {
                                ty: Type::UserInstance { ref name },
                                ..
                            }
                            | SymbolKind::Parameter {
                                ty: Type::UserInstance { ref name },
                                ..
                            } = sym.kind
                            && let Some(base_name) = self.type_map.get_class_base(name)
                        {
                            Type::UserInstance {
                                name: base_name.to_string(),
                            }
                        } else {
                            Type::Any
                        }
                    } else if is_runtime_global
                        && let Some(function) =
                            self.env.global(fn_name).and_then(|g| g.as_function())
                    {
                        self.return_type_of(&function.returns, &arg_types)
                    } else {
                        self.type_map
                            .get_function_return_type(fn_name)
                            .cloned()
                            .unwrap_or(Type::Any)
                    }
                } else {
                    Type::Any
                }
            }

            ast::ExprKind::Error => Type::Any,
        };

        self.type_map.insert_type(expr.span, ty.clone());
        ty
    }

    fn return_type_of(&self, returns: &Returns, arg_types: &[Type]) -> Type {
        match returns {
            Returns::Type(ty) => Type::from_ref(ty),
            Returns::NumericPromotion => {
                if !arg_types.is_empty() && arg_types.iter().all(|t| *t == Type::Int) {
                    Type::Int
                } else {
                    Type::Float
                }
            }
        }
    }
}

fn is_qsys_lua_call(callee: &ast::Expr) -> bool {
    match &callee.kind {
        ast::ExprKind::Attribute { value, attr } => {
            if let ast::ExprKind::Ident(ref obj) = value.kind {
                obj == "qsys" && attr == "lua"
            } else {
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_ir::ControlId;
    use qsys_parser::parse_module;

    use crate::analyze;
    use crate::types::{ResolvedTarget, TruthinessStrategy, Type};

    #[test]
    fn test_self_control_attribute_resolves_to_control_id() {
        let src = r#"
class MyPlugin(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    def runtime(self):
        v = self.Gain.Value
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).unwrap();
        let model = analyze(&parsed).expect("Should analyze cleanly");

        let has_ctl_resolution = model
            .type_map
            .resolutions()
            .values()
            .any(|r| *r == ResolvedTarget::Control(ControlId(0)));
        assert!(
            has_ctl_resolution,
            "self.Gain must resolve to ResolvedTarget::Control(ControlId(0))"
        );
    }

    #[test]
    fn test_fuzzy_member_diagnosis_on_control_handle() {
        let src = r#"
class MyPlugin(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    @Gain.EventHandler
    def on_gain(self, ctl):
        x = ctl.Valu
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).unwrap();
        let err = analyze(&parsed).expect_err("Should fail due to typo ctl.Valu");
        assert_eq!(err.items[0].code(), "E0201");
        assert!(err.items[0].message().contains("did you mean 'Value'?"));
    }

    #[test]
    fn test_catches_undefined_variable_typo_for_self() {
        let src = r#"
class MyPlugin(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    def runtime(self):
        sel.Gain = 42.0
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).unwrap();
        let err = analyze(&parsed).expect_err("Should fail due to typo sel");
        assert_eq!(err.items[0].code(), "E0206");
        assert!(err.items[0].message().contains("did you mean 'self'?"));
    }

    #[test]
    fn test_catches_undeclared_attribute_on_self() {
        let src = r#"
class MyPlugin(QSysPlugin):
    MasterGain = Control.Float(name="Gain", min=-100.0, max=20.0, default=0.0)

    def runtime(self):
        self.MusterGain = 42.0
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).unwrap();
        let err = analyze(&parsed).expect_err("Should fail due to typo self.MusterGain");
        assert_eq!(err.items[0].code(), "E0207");
        assert!(
            err.items[0]
                .message()
                .contains("did you mean 'MasterGain'?")
        );
    }

    #[test]
    fn test_numeric_truthiness_inferred() {
        let src = r#"
class MyPlugin(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    @Gain.EventHandler
    def on_gain(self, ctl):
        if ctl.Value:
            pass
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).unwrap();
        let model = analyze(&parsed).expect("Should analyze");

        let float_spans: Vec<_> = model
            .type_map
            .types()
            .iter()
            .filter(|(_, ty)| **ty == Type::Float)
            .map(|(span, _)| *span)
            .collect();
        assert!(!float_spans.is_empty());
        for span in float_spans {
            assert_eq!(
                model.type_map.get_truthiness(span),
                TruthinessStrategy::NumericNotZero
            );
        }
    }
}
