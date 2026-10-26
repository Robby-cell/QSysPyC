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

pub(crate) fn find_first_return_expr(stmts: &[ast::Stmt]) -> Option<&ast::Expr> {
    for stmt in stmts {
        match &stmt.kind {
            ast::StmtKind::Return(Some(ret_expr)) => return Some(ret_expr),
            ast::StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let Some(e) = find_first_return_expr(then_branch) {
                    return Some(e);
                }
                if let Some(e) = find_first_return_expr(else_branch) {
                    return Some(e);
                }
            }
            ast::StmtKind::While { body, .. } | ast::StmtKind::For { body, .. } => {
                if let Some(e) = find_first_return_expr(body) {
                    return Some(e);
                }
            }
            _ => {}
        }
    }
    None
}

pub struct ExprAnalyzer<'a> {
    pub scope_stack: &'a mut ScopeStack,
    pub type_map: &'a mut TypeMap,
    pub diagnostics: &'a mut Diagnostics,
    pub env: &'a RuntimeEnv,
    /// Execution phase of the code being analyzed, used to enforce global availability.
    pub context: ExecutionContext,
    pub all_module_stmts: Option<&'a [ast::Stmt]>,
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
            all_module_stmts: None,
        }
    }

    pub fn with_context(mut self, context: ExecutionContext) -> Self {
        self.context = context;
        self
    }

    pub fn with_all_module_stmts(mut self, stmts: &'a [ast::Stmt]) -> Self {
        self.all_module_stmts = Some(stmts);
        self
    }

    pub fn analyze_statements(&mut self, stmts: &[ast::Stmt]) {
        if matches!(
            self.scope_stack.current_scope().kind,
            ScopeKind::Class { .. }
        ) {
            for stmt in stmts {
                self.analyze_stmt(stmt);
            }
            return;
        }

        // Pass 1: Process bundled module definitions and imports so module namespaces are bound
        for stmt in stmts {
            if matches!(
                stmt.kind,
                ast::StmtKind::ModuleDef { .. }
                    | ast::StmtKind::Import { .. }
                    | ast::StmtKind::ImportFrom { .. }
            ) {
                self.analyze_stmt(stmt);
            }
        }

        // Pass 2: Hoist ClassDef, FunctionDef, and module-level Assign declarations
        for stmt in stmts {
            if let ast::StmtKind::ClassDef(c) = &stmt.kind {
                self.pre_register_class(c);
            }
        }
        for stmt in stmts {
            if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                self.pre_register_function(f);
            }
        }
        if matches!(self.scope_stack.current_scope().kind, ScopeKind::Module) {
            self.pre_register_globals(stmts);
            if let Some(all_stmts) = self.all_module_stmts {
                self.pre_register_globals(all_stmts);
            }
            for stmt in stmts {
                if let ast::StmtKind::Assign {
                    target,
                    value,
                    type_annotation,
                } = &stmt.kind
                {
                    let ann_ty = crate::type_from_annotation_with_scope(
                        type_annotation.as_deref(),
                        self.env,
                        self.scope_stack,
                    );
                    let quick_ty = if !matches!(ann_ty, Type::Any | Type::None) {
                        ann_ty
                    } else {
                        self.peek_expr_type(value)
                    };
                    self.pre_register_assign_target(target, quick_ty);
                }
            }
            self.pre_upgrade_outer_vars(stmts);
            if let Some(all_stmts) = self.all_module_stmts {
                self.pre_upgrade_outer_vars(all_stmts);
            }
        }

        // Pass 3: Analyze ClassDef bodies first so all class/instance attributes and method return types are populated
        for stmt in stmts {
            if matches!(stmt.kind, ast::StmtKind::ClassDef(_)) {
                self.analyze_stmt(stmt);
            }
        }

        // Pass 4: Analyze FunctionDef bodies and remaining statements
        for stmt in stmts {
            if !matches!(
                stmt.kind,
                ast::StmtKind::ModuleDef { .. }
                    | ast::StmtKind::Import { .. }
                    | ast::StmtKind::ImportFrom { .. }
                    | ast::StmtKind::ClassDef(_)
            ) {
                self.analyze_stmt(stmt);
            }
        }
    }

    fn pre_register_assign_target(&mut self, target: &ast::Expr, val_ty: Type) {
        match &target.kind {
            ast::ExprKind::Ident(name) => {
                if self.scope_stack.lookup(name).is_none() {
                    let sym_id = self.scope_stack.alloc_symbol_id();
                    self.scope_stack.define(Symbol::new(
                        sym_id,
                        name.clone(),
                        SymbolKind::Variable {
                            id: sym_id,
                            ty: val_ty.clone(),
                        },
                        target.span,
                    ));
                    self.type_map.insert_symbol_name(sym_id, name.clone());
                    self.type_map
                        .insert_resolution(target.span, ResolvedTarget::Local(sym_id));
                    self.type_map.insert_type(target.span, val_ty);
                }
            }
            ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
                for (idx, item) in items.iter().enumerate() {
                    let elem_ty = match &val_ty {
                        Type::Tuple(tys) => tys.get(idx).cloned().unwrap_or(Type::Any),
                        Type::List(elem) => (**elem).clone(),
                        _ => Type::Any,
                    };
                    self.pre_register_assign_target(item, elem_ty);
                }
            }
            _ => {}
        }
    }

    pub fn pre_register_globals(&mut self, stmts: &[ast::Stmt]) {
        for stmt in stmts {
            match &stmt.kind {
                ast::StmtKind::Global(names) => {
                    for (name, span) in names {
                        let (sym_id, _) = self.scope_stack.define_or_lookup_global(name, *span);
                        self.type_map.insert_symbol_name(sym_id, name.clone());
                    }
                }
                ast::StmtKind::FunctionDef(f) => {
                    self.pre_register_globals(&f.body);
                }
                ast::StmtKind::ClassDef(c) => {
                    self.pre_register_globals(&c.body);
                }
                ast::StmtKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    self.pre_register_globals(then_branch);
                    self.pre_register_globals(else_branch);
                }
                ast::StmtKind::While { body, .. } | ast::StmtKind::For { body, .. } => {
                    self.pre_register_globals(body);
                }
                _ => {}
            }
        }
    }

    fn upgrade_target_var_types(&mut self, target: &ast::Expr, val_ty: &Type) {
        match &target.kind {
            ast::ExprKind::Ident(name) => {
                if let Some(sym) = self.scope_stack.lookup(name)
                    && let SymbolKind::Variable { id, ty } = &sym.kind
                    && matches!(ty, Type::None | Type::Any)
                    && !matches!(val_ty, Type::Any | Type::None)
                {
                    let sym_id = *id;
                    self.scope_stack
                        .update_variable_type(sym_id, val_ty.clone());
                }
            }
            ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
                for (idx, item) in items.iter().enumerate() {
                    let elem_ty = match val_ty {
                        Type::Tuple(tys) => tys.get(idx).cloned().unwrap_or(Type::Any),
                        Type::List(elem) => (**elem).clone(),
                        _ => Type::Any,
                    };
                    self.upgrade_target_var_types(item, &elem_ty);
                }
            }
            _ => {}
        }
    }

    pub fn pre_upgrade_outer_vars(&mut self, stmts: &[ast::Stmt]) {
        self.pre_register_globals(stmts);
        for stmt in stmts {
            match &stmt.kind {
                ast::StmtKind::Assign {
                    target,
                    value,
                    type_annotation,
                } => {
                    let ann_ty = crate::type_from_annotation_with_scope(
                        type_annotation.as_deref(),
                        self.env,
                        self.scope_stack,
                    );
                    let new_ty = if !matches!(ann_ty, Type::Any | Type::None) {
                        ann_ty
                    } else {
                        self.peek_expr_type(value)
                    };
                    self.upgrade_target_var_types(target, &new_ty);
                }
                ast::StmtKind::FunctionDef(f) => {
                    self.pre_upgrade_outer_vars(&f.body);
                }
                ast::StmtKind::ClassDef(c) => {
                    for body_stmt in &c.body {
                        if let ast::StmtKind::FunctionDef(f) = &body_stmt.kind {
                            self.pre_upgrade_outer_vars(&f.body);
                        }
                    }
                }
                ast::StmtKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    self.pre_upgrade_outer_vars(then_branch);
                    self.pre_upgrade_outer_vars(else_branch);
                }
                ast::StmtKind::While { body, .. } | ast::StmtKind::For { body, .. } => {
                    self.pre_upgrade_outer_vars(body);
                }
                _ => {}
            }
        }
    }

    fn peek_expr_type(&self, expr: &ast::Expr) -> Type {
        match &expr.kind {
            ast::ExprKind::Int(_) => Type::Int,
            ast::ExprKind::Float(_) => Type::Float,
            ast::ExprKind::String(_) => Type::String,
            ast::ExprKind::Bool(_) => Type::Bool,
            ast::ExprKind::None => Type::None,
            ast::ExprKind::Tuple(items) => {
                Type::Tuple(items.iter().map(|i| self.peek_expr_type(i)).collect())
            }
            ast::ExprKind::List(items) => {
                let elem = items
                    .first()
                    .map(|i| self.peek_expr_type(i))
                    .unwrap_or(Type::Any);
                Type::List(Box::new(elem))
            }
            ast::ExprKind::Ident(name) => {
                if let Some(sym) = self.scope_stack.lookup(name) {
                    match &sym.kind {
                        SymbolKind::Variable { ty, .. } | SymbolKind::Parameter { ty, .. } => {
                            ty.clone()
                        }
                        SymbolKind::Class { name, .. } | SymbolKind::UserClass { name } => {
                            Type::UserClass { name: name.clone() }
                        }
                        SymbolKind::Module { name } => Type::UserModule { name: name.clone() },
                        _ => Type::Any,
                    }
                } else {
                    Type::Any
                }
            }
            ast::ExprKind::Call { callee, .. } => {
                let callee_ty = self.peek_expr_type(callee);
                if let Type::UserClass { name } = callee_ty {
                    Type::UserInstance { name }
                } else if let ast::ExprKind::Ident(fn_name) = &callee.kind {
                    self.type_map
                        .get_function_return_type(fn_name)
                        .cloned()
                        .unwrap_or(Type::Any)
                } else {
                    Type::Any
                }
            }
            ast::ExprKind::Attribute { value, attr } => {
                let base_ty = self.peek_expr_type(value);
                match base_ty {
                    Type::UserInstance { ref name }
                    | Type::UserClass { ref name }
                    | Type::ClassInstance { ref name } => self
                        .type_map
                        .get_class_member_type(name, attr)
                        .cloned()
                        .unwrap_or(Type::Any),
                    Type::UserModule { ref name } => self
                        .type_map
                        .get_module_member_type(name, attr)
                        .cloned()
                        .unwrap_or(Type::Any),
                    _ => Type::Any,
                }
            }
            _ => Type::Any,
        }
    }

    fn pre_register_class(&mut self, c: &ast::ClassDef) {
        let sym_id = self.scope_stack.alloc_symbol_id();

        if let Some(base_name) = c.bases.first()
            && !matches!(
                base_name.as_str(),
                "QSysPlugin" | "TextController" | "object"
            )
        {
            let base_cls = base_name
                .rsplit('.')
                .next()
                .unwrap_or(base_name.as_str())
                .to_string();
            self.type_map.insert_class_base(c.name.clone(), base_cls);
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

        for body_stmt in &c.body {
            match &body_stmt.kind {
                ast::StmtKind::FunctionDef(f) => {
                    let skip_self = usize::from(f.params.first().is_some_and(|p| p.name == "self"));
                    let sig_params = f
                        .params
                        .iter()
                        .skip(skip_self)
                        .map(|p| (p.name.clone(), p.default.as_deref().cloned()))
                        .collect();
                    self.type_map.insert_class_method_params(
                        c.name.clone(),
                        f.name.clone(),
                        sig_params,
                    );

                    let mut ret_ty = crate::type_from_annotation_with_scope(
                        f.return_type.as_deref(),
                        self.env,
                        self.scope_stack,
                    );
                    if ret_ty == Type::Any
                        && let Some(ret_expr) = find_first_return_expr(&f.body)
                    {
                        let peeked = self.peek_expr_type(ret_expr);
                        if peeked != Type::Any {
                            ret_ty = peeked;
                        }
                    }
                    self.type_map
                        .insert_class_member_type(c.name.clone(), f.name.clone(), ret_ty);

                    for method_stmt in &f.body {
                        if let ast::StmtKind::Assign {
                            target,
                            value,
                            type_annotation,
                        } = &method_stmt.kind
                            && let ast::ExprKind::Attribute {
                                value: obj_val,
                                attr,
                            } = &target.kind
                            && matches!(&obj_val.kind, ast::ExprKind::Ident(id) if id == "self")
                        {
                            let ann_ty = crate::type_from_annotation_with_scope(
                                type_annotation.as_deref(),
                                self.env,
                                self.scope_stack,
                            );
                            let attr_ty = if !matches!(ann_ty, Type::Any | Type::None) {
                                ann_ty
                            } else if let ast::ExprKind::Ident(rhs_id) = &value.kind
                                && let Some(param) = f.params.iter().find(|p| p.name == *rhs_id)
                            {
                                let p_ty = crate::type_from_annotation_with_scope(
                                    param.type_annotation.as_deref(),
                                    self.env,
                                    self.scope_stack,
                                );
                                if !matches!(p_ty, Type::Any | Type::None) {
                                    p_ty
                                } else if let Some(ref def_expr) = param.default {
                                    self.peek_expr_type(def_expr)
                                } else {
                                    self.peek_expr_type(value)
                                }
                            } else {
                                self.peek_expr_type(value)
                            };
                            if !matches!(attr_ty, Type::Any | Type::None) {
                                self.type_map.insert_class_member_type(
                                    c.name.clone(),
                                    attr.clone(),
                                    attr_ty,
                                );
                            }
                        }
                    }
                }
                ast::StmtKind::Assign {
                    target,
                    value,
                    type_annotation,
                } => {
                    if let ast::ExprKind::Ident(attr_name) = &target.kind {
                        let ann_ty = crate::type_from_annotation_with_scope(
                            type_annotation.as_deref(),
                            self.env,
                            self.scope_stack,
                        );
                        let attr_ty = if !matches!(ann_ty, Type::Any | Type::None) {
                            ann_ty
                        } else {
                            self.peek_expr_type(value)
                        };
                        self.type_map.insert_class_member_type(
                            c.name.clone(),
                            attr_name.clone(),
                            attr_ty,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn pre_register_function(&mut self, f: &ast::FunctionDef) {
        if self.type_map.get_resolution(f.span).is_some() {
            return;
        }
        let func_id = self.scope_stack.alloc_function_id();
        let sym_id = self.scope_stack.alloc_symbol_id();
        let has_self = f.params.first().is_some_and(|p| p.name == "self");

        self.scope_stack.define(Symbol::new(
            sym_id,
            f.name.clone(),
            SymbolKind::Method {
                id: func_id,
                param_count: f.params.len(),
                has_self,
            },
            f.span,
        ));
        self.type_map
            .insert_resolution(f.span, ResolvedTarget::Function(func_id));

        let sig_params = f
            .params
            .iter()
            .skip(usize::from(has_self))
            .map(|p| (p.name.clone(), p.default.as_deref().cloned()))
            .collect();
        self.type_map
            .insert_function_params(f.name.clone(), sig_params);

        let mut ret_ty = crate::type_from_annotation_with_scope(
            f.return_type.as_deref(),
            self.env,
            self.scope_stack,
        );
        if ret_ty == Type::Any
            && let Some(ret_expr) = find_first_return_expr(&f.body)
        {
            let peeked = self.peek_expr_type(ret_expr);
            if peeked != Type::Any {
                ret_ty = peeked;
            }
        }
        if ret_ty != Type::Any {
            self.type_map
                .insert_function_return_type(f.name.clone(), ret_ty);
        }
    }

    fn analyze_assign_target(&mut self, target: &ast::Expr, val_ty: Type) {
        match &target.kind {
            ast::ExprKind::Ident(name) => {
                let enclosing_class = match &self.scope_stack.current_scope().kind {
                    ScopeKind::Class { name } => Some(name.clone()),
                    _ => None,
                };
                let (sym_id, prev_ty, is_outer_var) = if self.scope_stack.is_explicit_global(name) {
                    if let Some(existing) = self.scope_stack.lookup_in_module(name) {
                        match &existing.kind {
                            SymbolKind::Variable { id, ty }
                            | SymbolKind::Parameter { id, ty, .. } => (*id, Some(ty.clone()), true),
                            _ => {
                                let (id, ty) =
                                    self.scope_stack.define_or_lookup_global(name, target.span);
                                (id, Some(ty), true)
                            }
                        }
                    } else {
                        let (id, ty) = self.scope_stack.define_or_lookup_global(name, target.span);
                        (id, Some(ty), true)
                    }
                } else if let Some(existing) = self.scope_stack.current_scope().symbols.get(name) {
                    match &existing.kind {
                        SymbolKind::Variable { id, ty } | SymbolKind::Parameter { id, ty, .. } => {
                            (*id, Some(ty.clone()), false)
                        }
                        _ => (self.scope_stack.alloc_symbol_id(), None, false),
                    }
                } else if enclosing_class.is_none()
                    && let Some(existing) = self.scope_stack.lookup(name)
                {
                    match &existing.kind {
                        SymbolKind::Variable { id, ty } | SymbolKind::Parameter { id, ty, .. } => {
                            (*id, Some(ty.clone()), true)
                        }
                        _ => (self.scope_stack.alloc_symbol_id(), None, false),
                    }
                } else {
                    (self.scope_stack.alloc_symbol_id(), None, false)
                };
                let effective_ty = if matches!(val_ty, Type::None | Type::Any)
                    && let Some(ref prev) = prev_ty
                    && !matches!(prev, Type::None | Type::Any)
                {
                    prev.clone()
                } else {
                    val_ty
                };
                if is_outer_var && !matches!(effective_ty, Type::None | Type::Any) {
                    self.scope_stack
                        .update_variable_type(sym_id, effective_ty.clone());
                }
                if let Some(cls_name) = enclosing_class {
                    self.type_map.insert_class_member_type(
                        cls_name,
                        name.clone(),
                        effective_ty.clone(),
                    );
                }
                self.scope_stack.define(Symbol::new(
                    sym_id,
                    name.clone(),
                    SymbolKind::Variable {
                        id: sym_id,
                        ty: effective_ty.clone(),
                    },
                    target.span,
                ));
                self.type_map
                    .insert_resolution(target.span, ResolvedTarget::Local(sym_id));
                self.type_map.insert_type(target.span, effective_ty);
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
                    self.type_map
                        .insert_class_member_type(name.clone(), attr.clone(), val_ty);
                }
                let _ = self.infer_expr(target);
            }
            ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) => {
                for (idx, item) in items.iter().enumerate() {
                    let elem_ty = match &val_ty {
                        Type::Tuple(tys) => tys.get(idx).cloned().unwrap_or(Type::Any),
                        Type::List(elem) => (**elem).clone(),
                        _ => Type::Any,
                    };
                    self.analyze_assign_target(item, elem_ty);
                }
                self.type_map.insert_type(target.span, val_ty);
            }
            _ => {
                let _ = self.infer_expr(target);
            }
        }
    }

    pub fn analyze_stmt(&mut self, stmt: &ast::Stmt) {
        match &stmt.kind {
            ast::StmtKind::Assign {
                target,
                value,
                type_annotation,
            } => {
                let val_ty = self.infer_expr(value);
                let ann_ty = crate::type_from_annotation_with_scope(
                    type_annotation.as_deref(),
                    self.env,
                    self.scope_stack,
                );
                let effective_ty = if !matches!(ann_ty, Type::Any | Type::None) {
                    ann_ty
                } else {
                    val_ty
                };
                self.analyze_assign_target(target, effective_ty);
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
                let enclosing_class = match &self.scope_stack.current_scope().kind {
                    ScopeKind::Class { name } => Some(name.clone()),
                    _ => None,
                };

                let func_id = if let Some(ResolvedTarget::Function(existing_id)) =
                    self.type_map.get_resolution(f.span)
                {
                    *existing_id
                } else {
                    let new_id = self.scope_stack.alloc_function_id();
                    let sym_id = self.scope_stack.alloc_symbol_id();
                    self.scope_stack.define(Symbol::new(
                        sym_id,
                        f.name.clone(),
                        SymbolKind::Method {
                            id: new_id,
                            param_count: f.params.len(),
                            has_self: f.params.first().is_some_and(|p| p.name == "self"),
                        },
                        f.span,
                    ));
                    self.type_map
                        .insert_resolution(f.span, ResolvedTarget::Function(new_id));
                    new_id
                };
                let _ = func_id;

                let has_self = f.params.first().is_some_and(|p| p.name == "self");
                let sig_params = f
                    .params
                    .iter()
                    .skip(usize::from(has_self))
                    .map(|p| (p.name.clone(), p.default.as_deref().cloned()))
                    .collect();
                if let Some(ref cls_name) = enclosing_class {
                    self.type_map.insert_class_method_params(
                        cls_name.clone(),
                        f.name.clone(),
                        sig_params,
                    );
                } else {
                    self.type_map
                        .insert_function_params(f.name.clone(), sig_params);
                }

                self.scope_stack.push_scope(ScopeKind::Function {
                    name: f.name.clone(),
                });

                for (idx, param) in f.params.iter().enumerate() {
                    let default_ty = param.default.as_ref().map(|d| self.infer_expr(d));
                    let param_sym_id = self.scope_stack.alloc_symbol_id();
                    let mut param_ty = if idx == 0 && param.name == "self" {
                        if let Some(ref cls_name) = enclosing_class {
                            Type::UserInstance {
                                name: cls_name.clone(),
                            }
                        } else {
                            Type::Any
                        }
                    } else {
                        crate::type_from_annotation_with_scope(
                            param.type_annotation.as_deref(),
                            self.env,
                            self.scope_stack,
                        )
                    };
                    if param_ty == Type::Any
                        && let Some(ref dt) = default_ty
                        && !matches!(dt, Type::Any | Type::None)
                    {
                        param_ty = dt.clone();
                    }
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

                // A module-level function in a plugin (ExecutionContext::Both) only runs when called,
                // whereas a function in a runtime-only script stays in ExecutionContext::Runtime.
                let fn_context = if self.context == ExecutionContext::Both {
                    ExecutionContext::Unknown
                } else {
                    self.context
                };
                let outer_context = std::mem::replace(&mut self.context, fn_context);
                self.analyze_statements(&f.body);
                self.context = outer_context;
                self.scope_stack.pop_scope();

                let mut ret_ty = crate::type_from_annotation_with_scope(
                    f.return_type.as_deref(),
                    self.env,
                    self.scope_stack,
                );
                if ret_ty == Type::Any
                    && let Some(ret_expr) = find_first_return_expr(&f.body)
                    && let Some(inferred) = self.type_map.get_type(ret_expr.span)
                    && *inferred != Type::Any
                {
                    ret_ty = inferred.clone();
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
                if !self
                    .scope_stack
                    .current_scope()
                    .symbols
                    .contains_key(&c.name)
                {
                    self.pre_register_class(c);
                }

                self.scope_stack.push_scope(ScopeKind::Class {
                    name: c.name.clone(),
                });
                self.analyze_statements(&c.body);
                self.scope_stack.pop_scope();
            }

            ast::StmtKind::ModuleDef { name, body, .. } => {
                self.type_map.register_module(name.clone());
                self.scope_stack.push_scope(ScopeKind::Module);
                self.analyze_statements(body);

                let exported: Vec<(String, Type)> =
                    self.scope_stack
                        .current_scope()
                        .symbols
                        .iter()
                        .map(|(sym_name, sym)| {
                            let ty = match &sym.kind {
                                SymbolKind::Class { name: cls_name, .. }
                                | SymbolKind::UserClass { name: cls_name } => Type::UserClass {
                                    name: cls_name.clone(),
                                },
                                SymbolKind::Method { .. } => self
                                    .type_map
                                    .get_function_return_type(sym_name)
                                    .cloned()
                                    .unwrap_or(Type::Any),
                                SymbolKind::Variable { ty, .. }
                                | SymbolKind::Parameter { ty, .. } => ty.clone(),
                                SymbolKind::Module { name: sub_mod } => Type::UserModule {
                                    name: sub_mod.clone(),
                                },
                                _ => Type::Any,
                            };
                            (sym_name.clone(), ty)
                        })
                        .collect();

                self.scope_stack.pop_scope();

                for (member_name, member_ty) in exported {
                    self.type_map
                        .insert_module_member_type(name.clone(), member_name, member_ty);
                }
            }

            ast::StmtKind::Import { module, alias } => {
                if module == "qsys" || module.starts_with("qsys.") {
                    return;
                }
                let mod_stem = module
                    .trim_start_matches('.')
                    .rsplit('.')
                    .next()
                    .unwrap_or(module.as_str())
                    .to_string();
                let bound_name = alias.clone().unwrap_or_else(|| mod_stem.clone());
                let sym_id = self.scope_stack.alloc_symbol_id();
                self.scope_stack.define(Symbol::new(
                    sym_id,
                    bound_name.clone(),
                    SymbolKind::Module {
                        name: mod_stem.clone(),
                    },
                    stmt.span,
                ));
                self.type_map.insert_symbol_name(sym_id, bound_name);
                self.type_map
                    .insert_type(stmt.span, Type::UserModule { name: mod_stem });
            }

            ast::StmtKind::ImportFrom {
                module, aliases, ..
            } => {
                if module == "qsys" || module.starts_with("qsys.") {
                    return;
                }
                let mod_stem = module
                    .trim_start_matches('.')
                    .rsplit('.')
                    .next()
                    .unwrap_or(module.as_str())
                    .to_string();

                for (orig_name, alias_opt) in aliases {
                    if orig_name == "*" {
                        for member_name in self.type_map.module_member_names(&mod_stem) {
                            if let Some(member_ty) = self
                                .type_map
                                .get_module_member_type(&mod_stem, &member_name)
                                .cloned()
                            {
                                let sym_id = self.scope_stack.alloc_symbol_id();
                                let kind = if let Type::UserClass { ref name } = member_ty {
                                    SymbolKind::Class {
                                        name: name.clone(),
                                        base: None,
                                    }
                                } else {
                                    SymbolKind::Variable {
                                        id: sym_id,
                                        ty: member_ty,
                                    }
                                };
                                self.scope_stack.define(Symbol::new(
                                    sym_id,
                                    member_name.clone(),
                                    kind,
                                    stmt.span,
                                ));
                                self.type_map.insert_symbol_name(sym_id, member_name);
                            }
                        }
                        continue;
                    }

                    let bound_name = alias_opt.clone().unwrap_or_else(|| orig_name.clone());
                    if let Some(member_ty) = self
                        .type_map
                        .get_module_member_type(&mod_stem, orig_name)
                        .cloned()
                    {
                        let sym_id = self.scope_stack.alloc_symbol_id();
                        let kind = if let Type::UserClass { ref name } = member_ty {
                            SymbolKind::Class {
                                name: name.clone(),
                                base: None,
                            }
                        } else {
                            self.type_map
                                .insert_function_return_type(bound_name.clone(), member_ty.clone());
                            SymbolKind::Variable {
                                id: sym_id,
                                ty: member_ty,
                            }
                        };
                        self.scope_stack.define(Symbol::new(
                            sym_id,
                            bound_name.clone(),
                            kind,
                            stmt.span,
                        ));
                        self.type_map.insert_symbol_name(sym_id, bound_name);
                    } else if self.type_map.is_known_module(&mod_stem) {
                        let known_members = self.type_map.module_member_names(&mod_stem);
                        let candidates = find_closest_match(orig_name, &known_members)
                            .map(|c| vec![c])
                            .unwrap_or_default();
                        self.diagnostics.report(
                            DiagnosticKind::UnknownClassMember {
                                class_name: mod_stem.clone(),
                                member: orig_name.clone(),
                                candidates,
                            },
                            stmt.span,
                        );
                    } else {
                        let sym_id = self.scope_stack.alloc_symbol_id();
                        self.scope_stack.define(Symbol::new(
                            sym_id,
                            bound_name.clone(),
                            SymbolKind::Variable {
                                id: sym_id,
                                ty: Type::Any,
                            },
                            stmt.span,
                        ));
                        self.type_map.insert_symbol_name(sym_id, bound_name);
                    }
                }
            }

            ast::StmtKind::Global(names) => {
                for (name, span) in names {
                    let (sym_id, ty) = self.scope_stack.define_or_lookup_global(name, *span);
                    self.type_map.insert_symbol_name(sym_id, name.clone());
                    self.type_map
                        .insert_resolution(*span, ResolvedTarget::Local(sym_id));
                    self.type_map.insert_type(*span, ty);
                }
            }

            ast::StmtKind::Nonlocal(names) => {
                for (name, span) in names {
                    if let Some(sym) = self.scope_stack.lookup_nonlocal(name) {
                        let sym_id = sym.id;
                        let ty = match &sym.kind {
                            SymbolKind::Variable { ty, .. } | SymbolKind::Parameter { ty, .. } => {
                                ty.clone()
                            }
                            _ => Type::Any,
                        };
                        self.scope_stack.define(Symbol::new(
                            sym_id,
                            name.clone(),
                            SymbolKind::Variable {
                                id: sym_id,
                                ty: ty.clone(),
                            },
                            *span,
                        ));
                        self.type_map.insert_symbol_name(sym_id, name.clone());
                        self.type_map
                            .insert_resolution(*span, ResolvedTarget::Local(sym_id));
                        self.type_map.insert_type(*span, ty);
                    }
                }
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
                        SymbolKind::Module { name } => {
                            self.type_map
                                .insert_resolution(expr.span, ResolvedTarget::Local(symbol.id));
                            Type::UserModule { name: name.clone() }
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

            ast::ExprKind::Tuple(items) => {
                let elem_tys = items.iter().map(|item| self.infer_expr(item)).collect();
                Type::Tuple(elem_tys)
            }

            ast::ExprKind::List(items) => {
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
                        Some(ControlMember::Strings) | Some(ControlMember::Choices) => {
                            Type::List(Box::new(Type::String))
                        }
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
                    Type::UserInstance { ref name } | Type::UserClass { ref name } => {
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
                    Type::UserModule { ref name } => {
                        if let Some(member_ty) =
                            self.type_map.get_module_member_type(name, attr).cloned()
                        {
                            member_ty
                        } else {
                            if self.type_map.is_known_module(name) {
                                let known_members = self.type_map.module_member_names(name);
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
                let idx_ty = self.infer_expr(index);
                match val_ty {
                    Type::ControlHandle => {
                        if idx_ty == Type::String || matches!(index.kind, ast::ExprKind::String(_))
                        {
                            Type::Any
                        } else {
                            Type::ControlHandle
                        }
                    }
                    Type::List(elem) => *elem,
                    Type::Tuple(elems) => {
                        if let ast::ExprKind::Int(i) = index.kind
                            && i >= 0
                        {
                            elems.get(i as usize).cloned().unwrap_or(Type::Any)
                        } else {
                            elems.first().cloned().unwrap_or(Type::Any)
                        }
                    }
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
                    } else if let Some(Type::UserModule { ref name }) = receiver_ty
                        && let Some(ret_ty) =
                            self.type_map.get_module_member_type(name, attr).cloned()
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

            ast::ExprKind::Lambda { params, body } => {
                self.scope_stack.push_scope(ScopeKind::Function {
                    name: "<lambda>".to_string(),
                });
                for (idx, param) in params.iter().enumerate() {
                    if let Some(ref def_expr) = param.default {
                        let _ = self.infer_expr(def_expr);
                    }
                    let param_sym = self.scope_stack.alloc_symbol_id();
                    self.type_map
                        .insert_symbol_name(param_sym, param.name.clone());
                    self.scope_stack.define(Symbol::new(
                        param_sym,
                        param.name.clone(),
                        SymbolKind::Parameter {
                            id: param_sym,
                            ty: Type::Any,
                            index: idx,
                        },
                        param.span,
                    ));
                    self.type_map
                        .insert_resolution(param.span, ResolvedTarget::Local(param_sym));
                    self.type_map.insert_type(param.span, Type::Any);
                }
                let _ = self.infer_expr(body);
                self.scope_stack.pop_scope();
                Type::Any
            }

            ast::ExprKind::IfExp { test, body, orelse } => {
                let test_ty = self.infer_expr(test);
                self.type_map.insert_type(test.span, test_ty);
                let body_ty = self.infer_expr(body);
                let orelse_ty = self.infer_expr(orelse);
                if body_ty == orelse_ty || orelse_ty == Type::Any || orelse_ty == Type::None {
                    body_ty
                } else if body_ty == Type::Any || body_ty == Type::None {
                    orelse_ty
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
