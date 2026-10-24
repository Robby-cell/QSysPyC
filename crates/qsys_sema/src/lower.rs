use rustc_hash::FxHashMap;

use qsys_diagnostics::Diagnostics;
use qsys_ir::*;
use qsys_parser::ast;
use qsys_runtime::{CallStyle, Lowering, RuntimeEnv};

use crate::declarations::DeclaredControl;
use crate::types::{ResolvedTarget, Type};
use crate::ui_layout::{
    extract_graphics_append, is_layout_target, is_ui_layout_expr, parse_ui_layout_expr,
    substitute_ui_vars,
};
use crate::{SemanticModel, TargetKind, TruthinessStrategy, TypeMap};

pub fn lower_to_ir(model: &SemanticModel) -> IRProgram {
    match &model.target {
        TargetKind::Plugin(p) => {
            let mut lowerer =
                StmtLowerer::new(model, true, &p.class_name, &p.controls, &p.helper_classes);

            let classes = p
                .helper_classes
                .iter()
                .map(|c| lowerer.lower_class(c))
                .collect();

            let methods = p
                .methods
                .iter()
                .map(|m| lowerer.lower_function(m.id, &m.name, &m.params, &m.body, m.span))
                .collect();

            let init_body = lowerer.lower_statements(&p.init_statements);

            let event_bindings = p
                .event_bindings
                .iter()
                .map(|b| IREventBinding {
                    control_id: b.control_id,
                    control_name: b.control_name.clone(),
                    channel_index: b.channel_index,
                    member: b.member,
                    handler_id: b.handler_id,
                    handler_name: b.handler_name.clone(),
                    span: b.span,
                })
                .collect();

            let controls = p
                .controls
                .iter()
                .map(|c| IRControlBinding {
                    id: c.id,
                    ident: c.ident.clone(),
                    def: c.def.clone(),
                })
                .collect();

            let properties = p
                .properties
                .iter()
                .map(|pr| IRPropertyBinding {
                    id: pr.id,
                    ident: pr.ident.clone(),
                    def: pr.def.clone(),
                })
                .collect();

            let symbol_names = model.type_map.symbol_names().clone();

            IRProgram::Plugin(Box::new(IRPlugin {
                info: p.info.clone(),
                properties,
                controls,
                pages: p.pages.clone(),
                class_name: p.class_name.clone(),
                classes,
                methods,
                event_bindings,
                init_body,
                symbol_names,
                top_level_stmts: lowerer.lower_statements(&p.module_statements),
                span: p.span,
            }))
        }

        TargetKind::TextController(c) => {
            let mut lowerer =
                StmtLowerer::new(model, false, &c.class_name, &c.controls, &c.helper_classes);

            let classes = c
                .helper_classes
                .iter()
                .map(|cls| lowerer.lower_class(cls))
                .collect();

            let methods = c
                .methods
                .iter()
                .map(|m| lowerer.lower_function(m.id, &m.name, &m.params, &m.body, m.span))
                .collect();

            let top_level_stmts = lowerer.lower_statements(&c.module_statements);
            let body = lowerer.lower_statements(&c.init_statements);

            let event_bindings = c
                .event_bindings
                .iter()
                .map(|b| IREventBinding {
                    control_id: b.control_id,
                    control_name: b.control_name.clone(),
                    channel_index: b.channel_index,
                    member: b.member,
                    handler_id: b.handler_id,
                    handler_name: b.handler_name.clone(),
                    span: b.span,
                })
                .collect();

            let bound_pins = c.controls.iter().map(|ctl| ctl.def.name.clone()).collect();
            let controls = c
                .controls
                .iter()
                .map(|ctl| IRControlBinding {
                    id: ctl.id,
                    ident: ctl.ident.clone(),
                    def: ctl.def.clone(),
                })
                .collect();

            IRProgram::Script(Box::new(IRScript {
                name: c.class_name.clone(),
                is_controller: true,
                bound_pins,
                controls,
                classes,
                methods,
                event_bindings,
                symbol_names: model.type_map.symbol_names().clone(),
                top_level_stmts,
                body,
                span: c.span,
            }))
        }

        TargetKind::ProceduralScript(s) => {
            let mut lowerer = StmtLowerer::new(model, false, "Script", &[], &s.classes);

            let classes = s
                .classes
                .iter()
                .map(|cls| lowerer.lower_class(cls))
                .collect();

            let methods = s
                .methods
                .iter()
                .map(|m| lowerer.lower_function(m.id, &m.name, &m.params, &m.body, m.span))
                .collect();
            let body = lowerer.lower_statements(&s.statements);

            IRProgram::Script(Box::new(IRScript {
                name: "main".to_string(),
                is_controller: false,
                bound_pins: s.bound_pins.clone(),
                controls: Vec::new(),
                classes,
                methods,
                event_bindings: Vec::new(),
                symbol_names: model.type_map.symbol_names().clone(),
                top_level_stmts: Vec::new(),
                body,
                span: qsys_diagnostics::Span::new(0, 0),
            }))
        }
    }
}

struct StmtLowerer<'a> {
    type_map: &'a TypeMap,
    env: &'a RuntimeEnv,
    is_plugin: bool,
    plugin_class_name: &'a str,
    controls: &'a [DeclaredControl],
    custom_classes: &'a [ast::ClassDef],
    current_class_base: Option<String>,
    local_ui_vars: FxHashMap<String, ast::Expr>,
}

impl<'a> StmtLowerer<'a> {
    fn new(
        model: &'a SemanticModel,
        is_plugin: bool,
        plugin_class_name: &'a str,
        controls: &'a [DeclaredControl],
        custom_classes: &'a [ast::ClassDef],
    ) -> Self {
        Self {
            type_map: &model.type_map,
            env: &model.env,
            is_plugin,
            plugin_class_name,
            controls,
            custom_classes,
            current_class_base: None,
            local_ui_vars: FxHashMap::default(),
        }
    }

    fn lower_ui_expr_to_stmt(
        &self,
        expr: &ast::Expr,
        span: qsys_diagnostics::Span,
    ) -> Option<IRStmt> {
        let mut dummy_diags = Diagnostics::new();
        let mut dummy_res = Vec::new();
        let mut pages = parse_ui_layout_expr(
            self.plugin_class_name,
            expr,
            self.controls,
            &mut dummy_diags,
            &mut dummy_res,
        );
        pages
            .pop()
            .map(|page| IRStmt::EmitPageLayout { page, span })
    }

    fn lower_function(
        &mut self,
        id: FunctionId,
        name: &str,
        params: &[(SymbolId, String)],
        body: &[ast::Stmt],
        span: qsys_diagnostics::Span,
    ) -> IRFunction {
        IRFunction {
            id,
            name: name.to_string(),
            params: params
                .iter()
                .map(|(sid, sname)| IRParam {
                    id: *sid,
                    name: sname.clone(),
                })
                .collect(),
            body: self.lower_statements(body),
            span,
        }
    }

    fn lower_statements(&mut self, stmts: &[ast::Stmt]) -> Vec<IRStmt> {
        let mut ir_stmts = Vec::new();
        for stmt in stmts {
            if let ast::StmtKind::ImportFrom {
                module, aliases, ..
            } = &stmt.kind
            {
                if module != "qsys" && !module.starts_with("qsys.") {
                    let mod_stem = module
                        .trim_start_matches('.')
                        .rsplit('.')
                        .next()
                        .unwrap_or(module.as_str())
                        .to_string();
                    if self.type_map.is_known_module(&mod_stem) {
                        for (orig_name, alias_opt) in aliases {
                            if orig_name == "*" {
                                for member_name in self.type_map.module_member_names(&mod_stem) {
                                    ir_stmts.push(IRStmt::ImportBinding {
                                        local_name: member_name.clone(),
                                        target: IRExpr::Attribute {
                                            object: Box::new(IRExpr::Global(mod_stem.clone())),
                                            attr: member_name,
                                        },
                                        span: stmt.span,
                                    });
                                }
                            } else {
                                let local_name =
                                    alias_opt.clone().unwrap_or_else(|| orig_name.clone());
                                ir_stmts.push(IRStmt::ImportBinding {
                                    local_name,
                                    target: IRExpr::Attribute {
                                        object: Box::new(IRExpr::Global(mod_stem.clone())),
                                        attr: orig_name.clone(),
                                    },
                                    span: stmt.span,
                                });
                            }
                        }
                    }
                }
                continue;
            }
            if let Some(lowered) = self.lower_stmt(stmt) {
                ir_stmts.push(lowered);
            }
        }
        ir_stmts
    }

    fn lower_stmt(&mut self, stmt: &ast::Stmt) -> Option<IRStmt> {
        let span = stmt.span;

        match &stmt.kind {
            ast::StmtKind::Assign { target, value } => {
                let sub_val = substitute_ui_vars(value, &self.local_ui_vars);
                if is_ui_layout_expr(&sub_val) {
                    if is_layout_target(target) {
                        return self.lower_ui_expr_to_stmt(&sub_val, span);
                    }
                    if let ast::ExprKind::Ident(var_name) = &target.kind {
                        self.local_ui_vars.insert(var_name.clone(), sub_val);
                        return None;
                    }
                }

                let target_ir = match &target.kind {
                    ast::ExprKind::Subscript {
                        value: sub_val,
                        index: sub_idx,
                    } => {
                        let val_ty = self.type_map.get_type(sub_val.span);
                        let is_zero_based =
                            matches!(val_ty, Some(Type::List(_) | Type::ControlHandle));
                        IRExpr::Index {
                            base: Box::new(self.lower_expr(sub_val)),
                            index: Box::new(self.lower_expr(sub_idx)),
                            is_zero_based,
                        }
                    }
                    _ => self.lower_expr(target),
                };
                let value_ir = self.lower_expr(value);
                Some(IRStmt::Assign {
                    target: target_ir,
                    value: value_ir,
                    span,
                })
            }

            ast::StmtKind::AugAssign { target, op, value } => {
                let target_ir = self.lower_expr(target);
                let value_ir = self.lower_expr(value);
                let mut binary_op = lower_binary_op(*op);

                if *op == ast::BinaryOp::Add {
                    let target_ty = self.type_map.get_type(target.span);
                    let val_ty = self.type_map.get_type(value.span);
                    if matches!(target_ty, Some(Type::String))
                        || matches!(val_ty, Some(Type::String))
                    {
                        binary_op = IRBinaryOp::StringConcat;
                    }
                }

                let desugared_value = IRExpr::Binary {
                    left: Box::new(target_ir.clone()),
                    op: binary_op,
                    right: Box::new(value_ir),
                };

                Some(IRStmt::Assign {
                    target: target_ir,
                    value: desugared_value,
                    span,
                })
            }

            ast::StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let condition_ir = self.lower_expr(condition);
                let truthiness = lower_truthiness(self.type_map.get_truthiness(condition.span));
                let then_ir = self.lower_statements(then_branch);
                let else_ir = self.lower_statements(else_branch);

                Some(IRStmt::If {
                    condition: condition_ir,
                    truthiness,
                    then_branch: then_ir,
                    else_branch: else_ir,
                    span,
                })
            }

            ast::StmtKind::While { condition, body } => {
                let condition_ir = self.lower_expr(condition);
                let truthiness = lower_truthiness(self.type_map.get_truthiness(condition.span));
                let body_ir = self.lower_statements(body);

                Some(IRStmt::While {
                    condition: condition_ir,
                    truthiness,
                    body: body_ir,
                    span,
                })
            }

            ast::StmtKind::For {
                target,
                iterable,
                body,
            } => {
                let iter_ir = self.lower_expr(iterable);
                let target_id = self
                    .type_map
                    .get_resolution(stmt.span)
                    .and_then(|r| match r {
                        ResolvedTarget::Local(id) => Some(*id),
                        _ => None,
                    })
                    .unwrap_or(SymbolId(0));

                let body_ir = self.lower_statements(body);

                Some(IRStmt::For {
                    target_id,
                    target_name: target.clone(),
                    iterable: iter_ir,
                    body: body_ir,
                    span,
                })
            }

            ast::StmtKind::Return(val) => {
                if let Some(v) = val {
                    let sub_v = substitute_ui_vars(v, &self.local_ui_vars);
                    if is_ui_layout_expr(&sub_v) {
                        return self.lower_ui_expr_to_stmt(&sub_v, span);
                    }
                }
                let val_ir = val.as_ref().map(|v| self.lower_expr(v));
                Some(IRStmt::Return {
                    value: val_ir,
                    span,
                })
            }

            ast::StmtKind::Break => Some(IRStmt::Break { span }),

            ast::StmtKind::Expr(expr) => {
                let sub_expr = substitute_ui_vars(expr, &self.local_ui_vars);
                if is_ui_layout_expr(&sub_expr) {
                    return self.lower_ui_expr_to_stmt(&sub_expr, span);
                }
                if let Some(appended) = extract_graphics_append(&sub_expr) {
                    return self.lower_ui_expr_to_stmt(&appended, span);
                }
                if let Some(raw_code) = extract_raw_lua(expr) {
                    Some(IRStmt::RawLua {
                        code: raw_code,
                        span,
                    })
                } else {
                    Some(IRStmt::Expr {
                        expr: self.lower_expr(expr),
                        span,
                    })
                }
            }

            ast::StmtKind::Pass | ast::StmtKind::ImportFrom { .. } => None,

            ast::StmtKind::Import { module, alias } => {
                if module == "qsys" || module.starts_with("qsys.") {
                    return None;
                }
                let mod_stem = module
                    .trim_start_matches('.')
                    .rsplit('.')
                    .next()
                    .unwrap_or(module.as_str())
                    .to_string();
                if let Some(alias_name) = alias
                    && alias_name != &mod_stem
                    && self.type_map.is_known_module(&mod_stem)
                {
                    return Some(IRStmt::ImportBinding {
                        local_name: alias_name.clone(),
                        target: IRExpr::Global(mod_stem),
                        span,
                    });
                }
                None
            }

            ast::StmtKind::ModuleDef { name, body, .. } => {
                let lowered_body = self.lower_statements(body);
                let mut exports = Vec::new();
                for s in body {
                    match &s.kind {
                        ast::StmtKind::ClassDef(cls) => {
                            if !exports.contains(&cls.name) {
                                exports.push(cls.name.clone());
                            }
                        }
                        ast::StmtKind::FunctionDef(f) => {
                            if !exports.contains(&f.name) {
                                exports.push(f.name.clone());
                            }
                        }
                        ast::StmtKind::Assign { target, .. } => {
                            if let ast::ExprKind::Ident(var_name) = &target.kind
                                && !exports.contains(var_name)
                            {
                                exports.push(var_name.clone());
                            }
                        }
                        _ => {}
                    }
                }
                Some(IRStmt::ModuleDef {
                    name: name.clone(),
                    body: lowered_body,
                    exports,
                    span,
                })
            }

            ast::StmtKind::FunctionDef(f) => {
                let func_id = self
                    .type_map
                    .get_resolution(f.span)
                    .and_then(|r| match r {
                        ResolvedTarget::Function(id) => Some(*id),
                        _ => None,
                    })
                    .unwrap_or(FunctionId(0));

                let params = f
                    .params
                    .iter()
                    .map(|p| {
                        let sym_id = self
                            .type_map
                            .get_resolution(p.span)
                            .and_then(|r| match r {
                                ResolvedTarget::Local(id) => Some(*id),
                                _ => None,
                            })
                            .unwrap_or(SymbolId(0));
                        IRParam {
                            name: p.name.clone(),
                            id: sym_id,
                        }
                    })
                    .collect();

                let body = self.lower_statements(&f.body);

                Some(IRStmt::FunctionDef(IRFunction {
                    id: func_id,
                    name: f.name.clone(),
                    params,
                    body,
                    span: f.span,
                }))
            }

            ast::StmtKind::ClassDef(cls) => Some(IRStmt::ClassDef(self.lower_class(cls))),
        }
    }

    fn lower_expr(&self, expr: &ast::Expr) -> IRExpr {
        if matches!(
            expr.kind,
            ast::ExprKind::Ident(_) | ast::ExprKind::Attribute { .. }
        ) && let Some(target) = self.type_map.get_resolution(expr.span)
        {
            match target {
                ResolvedTarget::Local(id) => return IRExpr::Local(*id),
                ResolvedTarget::Control(id) => return IRExpr::Control(*id),
                ResolvedTarget::Property(id) => return IRExpr::Property(*id),
                ResolvedTarget::Function(id) => return IRExpr::Function(*id),
                ResolvedTarget::Global(name) => return IRExpr::Global(name.clone()),
            }
        }

        match &expr.kind {
            ast::ExprKind::Int(i) => IRExpr::Int(*i),
            ast::ExprKind::Float(f) => IRExpr::Float(*f),
            ast::ExprKind::String(s) => IRExpr::String(s.clone()),
            ast::ExprKind::Bool(b) => IRExpr::Bool(*b),
            ast::ExprKind::None => IRExpr::Nil,
            ast::ExprKind::Ident(name) => IRExpr::Global(name.clone()),

            ast::ExprKind::Tuple(items) => {
                let lowered = items.iter().map(|item| self.lower_expr(item)).collect();
                IRExpr::Tuple(lowered)
            }

            ast::ExprKind::List(items) => {
                let lowered = items.iter().map(|item| self.lower_expr(item)).collect();
                IRExpr::List(lowered)
            }

            ast::ExprKind::Dict(pairs) => {
                let lowered = pairs
                    .iter()
                    .map(|(k, v)| (self.lower_expr(k), self.lower_expr(v)))
                    .collect();
                IRExpr::Dict(lowered)
            }

            ast::ExprKind::Binary { left, op, right } => {
                let left_ir = self.lower_expr(left);
                let right_ir = self.lower_expr(right);

                let is_str_expr = |e: &IRExpr, span: qsys_diagnostics::Span| -> bool {
                    matches!(e, IRExpr::String(_))
                        || matches!(
                            e,
                            IRExpr::Binary {
                                op: IRBinaryOp::Concat
                                    | IRBinaryOp::StringConcat
                                    | IRBinaryOp::StringRep,
                                ..
                            }
                        )
                        || self.type_map.get_type(span) == Some(&Type::String)
                };

                let is_int_expr = |e: &IRExpr, span: qsys_diagnostics::Span| -> bool {
                    matches!(e, IRExpr::Int(_)) || self.type_map.get_type(span) == Some(&Type::Int)
                };

                if *op == ast::BinaryOp::Add
                    && (is_str_expr(&left_ir, left.span) || is_str_expr(&right_ir, right.span))
                {
                    return IRExpr::Binary {
                        left: Box::new(left_ir),
                        op: IRBinaryOp::StringConcat,
                        right: Box::new(right_ir),
                    };
                }

                if *op == ast::BinaryOp::Mul {
                    if is_str_expr(&left_ir, left.span) && is_int_expr(&right_ir, right.span) {
                        return IRExpr::Binary {
                            left: Box::new(left_ir),
                            op: IRBinaryOp::StringRep,
                            right: Box::new(right_ir),
                        };
                    }
                    if is_int_expr(&left_ir, left.span) && is_str_expr(&right_ir, right.span) {
                        return IRExpr::Binary {
                            left: Box::new(right_ir),
                            op: IRBinaryOp::StringRep,
                            right: Box::new(left_ir),
                        };
                    }
                }

                IRExpr::Binary {
                    left: Box::new(left_ir),
                    op: lower_binary_op(*op),
                    right: Box::new(right_ir),
                }
            }

            ast::ExprKind::BoolOp { op, left, right } => {
                let left_truthiness = lower_truthiness(self.type_map.get_truthiness(left.span));
                match op {
                    ast::LogicalOp::And => IRExpr::LogicalAnd {
                        left: Box::new(self.lower_expr(left)),
                        right: Box::new(self.lower_expr(right)),
                        left_truthiness,
                    },
                    ast::LogicalOp::Or => IRExpr::LogicalOr {
                        left: Box::new(self.lower_expr(left)),
                        right: Box::new(self.lower_expr(right)),
                        left_truthiness,
                    },
                }
            }

            ast::ExprKind::Unary { op, operand } => {
                let truthiness = lower_truthiness(self.type_map.get_truthiness(operand.span));
                IRExpr::Unary {
                    op: lower_unary_op(*op),
                    operand: Box::new(self.lower_expr(operand)),
                    truthiness,
                }
            }

            ast::ExprKind::Call {
                callee,
                args,
                kwargs,
            } => {
                if let Some(raw_code) = extract_raw_lua(expr) {
                    IRExpr::RawLua(raw_code)
                } else {
                    let mut args_ir: Vec<IRExpr> =
                        args.iter().map(|arg| self.lower_expr(arg)).collect();
                    let kwargs_ir: Vec<(String, IRExpr)> = kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), self.lower_expr(v)))
                        .collect();

                    let callee_ty = self.type_map.get_type(callee.span);

                    // Calling a custom class: `Device(...)` or `helpers.Device(...)` -> lowers to `<class>.new(...)`
                    if let Some(Type::UserClass { .. }) = callee_ty {
                        let class_obj = self.lower_expr(callee);
                        return IRExpr::Call {
                            callee: Box::new(IRExpr::Attribute {
                                object: Box::new(class_obj),
                                attr: "new".to_string(),
                            }),
                            args: args_ir,
                            kwargs: kwargs_ir,
                        };
                    }

                    // Builtin lowering declared by the runtime env (e.g. `len(x)` -> `#x`).
                    // Only applies when the name resolved to the runtime global, so user
                    // definitions that shadow a builtin are called as written.
                    if let ast::ExprKind::Ident(fn_name) = &callee.kind
                        && matches!(
                            self.type_map.get_resolution(callee.span),
                            Some(ResolvedTarget::Global(_))
                        )
                        && let Some(function) =
                            self.env.global(fn_name).and_then(|g| g.as_function())
                    {
                        match &function.lowering {
                            Lowering::LengthOperator
                                if args_ir.len() == 1 && kwargs_ir.is_empty() =>
                            {
                                return IRExpr::Unary {
                                    op: IRUnaryOp::Len,
                                    operand: Box::new(args_ir.remove(0)),
                                    truthiness: IRTruthiness::NumericNotZero,
                                };
                            }
                            Lowering::Rename(target) => {
                                return IRExpr::Call {
                                    callee: Box::new(IRExpr::Global(target.clone())),
                                    args: args_ir,
                                    kwargs: kwargs_ir,
                                };
                            }
                            Lowering::Qualified { module, name } => {
                                return IRExpr::Call {
                                    callee: Box::new(IRExpr::Attribute {
                                        object: Box::new(IRExpr::Global(module.clone())),
                                        attr: name.clone(),
                                    }),
                                    args: args_ir,
                                    kwargs: kwargs_ir,
                                };
                            }
                            Lowering::LengthOperator | Lowering::Direct => {}
                        }
                    }

                    if let ast::ExprKind::Attribute { value, attr } = &callee.kind {
                        // list.append(x) -> table.insert(list, x)
                        if attr == "append" && args.len() == 1 {
                            return IRExpr::Call {
                                callee: Box::new(IRExpr::Attribute {
                                    object: Box::new(IRExpr::Global("table".to_string())),
                                    attr: "insert".to_string(),
                                }),
                                args: vec![self.lower_expr(value), self.lower_expr(&args[0])],
                                kwargs: vec![],
                            };
                        }
                        // list.insert(0, x) -> table.insert(list, 1, x)
                        if attr == "insert" && args.len() == 2 {
                            let index_expr = IRExpr::Binary {
                                left: Box::new(self.lower_expr(&args[0])),
                                op: IRBinaryOp::Add,
                                right: Box::new(IRExpr::Int(1)),
                            };
                            return IRExpr::Call {
                                callee: Box::new(IRExpr::Attribute {
                                    object: Box::new(IRExpr::Global("table".to_string())),
                                    attr: "insert".to_string(),
                                }),
                                args: vec![
                                    self.lower_expr(value),
                                    index_expr,
                                    self.lower_expr(&args[1]),
                                ],
                                kwargs: vec![],
                            };
                        }
                        // list.pop() -> table.remove(list)
                        if attr == "pop" && args.is_empty() {
                            return IRExpr::Call {
                                callee: Box::new(IRExpr::Attribute {
                                    object: Box::new(IRExpr::Global("table".to_string())),
                                    attr: "remove".to_string(),
                                }),
                                args: vec![self.lower_expr(value)],
                                kwargs: vec![],
                            };
                        }
                        // dict.get(key) -> dict[key]
                        if attr == "get" && args.len() == 1 {
                            return IRExpr::Index {
                                base: Box::new(self.lower_expr(value)),
                                index: Box::new(self.lower_expr(&args[0])),
                                is_zero_based: false,
                            };
                        }

                        // `super().method(...)` -> `BaseName.method(self, ...)`
                        if let ast::ExprKind::Call {
                            callee: super_callee,
                            args: super_args,
                            ..
                        } = &value.kind
                            && super_args.is_empty()
                            && matches!(&super_callee.kind, ast::ExprKind::Ident(id) if id == "super")
                            && let Some(ref base_name) = self.current_class_base
                        {
                            let target_method = if attr == "__init__" {
                                "__init"
                            } else {
                                attr.as_str()
                            };
                            let mut call_args = Vec::with_capacity(args_ir.len() + 1);
                            call_args.push(IRExpr::Global("self".to_string()));
                            call_args.extend(args_ir);
                            return IRExpr::Call {
                                callee: Box::new(IRExpr::Attribute {
                                    object: Box::new(IRExpr::Global(base_name.clone())),
                                    attr: target_method.to_string(),
                                }),
                                args: call_args,
                                kwargs: kwargs_ir,
                            };
                        }

                        let obj_ty = self.type_map.get_type(value.span);

                        // Native runtime objects use the call style declared by the runtime env
                        // (`timer:Start(1)` for handles, `Timer.New()` for factories); custom class
                        // instances always dispatch with `obj:method(...)`.
                        let uses_colon = match obj_ty {
                            Some(Type::Native { name }) => {
                                self.env.call_style(name, attr) == CallStyle::Colon
                            }
                            Some(Type::UserInstance { .. }) => true,
                            _ => false,
                        };
                        if uses_colon {
                            return IRExpr::MethodCall {
                                object: Box::new(self.lower_expr(value)),
                                method: attr.clone(),
                                args: args_ir,
                                kwargs: kwargs_ir,
                            };
                        }

                        // String instance methods -> `s:upper()`, `s:lower()`, `s:gsub(...)`, `s:sub(...)`
                        if obj_ty == Some(&Type::String)
                            && !matches!(&value.kind, ast::ExprKind::Ident(id) if id == "string")
                        {
                            let lua_method = match attr.as_str() {
                                "replace" => Some("gsub"),
                                "upper" | "lower" | "gsub" | "sub" | "find" | "match"
                                | "gmatch" | "byte" | "rep" | "reverse" | "format" => {
                                    Some(attr.as_str())
                                }
                                _ => None,
                            };
                            if let Some(m_name) = lua_method {
                                return IRExpr::MethodCall {
                                    object: Box::new(self.lower_expr(value)),
                                    method: m_name.to_string(),
                                    args: args_ir,
                                    kwargs: kwargs_ir,
                                };
                            }
                        }

                        // Calling a method on `self` inside a QSysPlugin -> prepend `nil` for `self`
                        if self.is_plugin && matches!(obj_ty, Some(Type::ClassInstance { .. })) {
                            args_ir.insert(0, IRExpr::Nil);
                        }
                    }

                    let callee_ir = Box::new(self.lower_expr(callee));
                    IRExpr::Call {
                        callee: callee_ir,
                        args: args_ir,
                        kwargs: kwargs_ir,
                    }
                }
            }

            ast::ExprKind::Attribute { value, attr } => IRExpr::Attribute {
                object: Box::new(self.lower_expr(value)),
                attr: attr.clone(),
            },

            // Subscript Indexing (Dictionaries do NOT get +1 offset)
            ast::ExprKind::Subscript { value, index } => {
                let val_ty = self.type_map.get_type(value.span);
                let idx_ty = self.type_map.get_type(index.span);

                let is_zero_based = match (val_ty, idx_ty) {
                    (Some(Type::Dict(_, _)), _) => false,
                    (_, Some(Type::String)) => false,
                    (Some(Type::List(_) | Type::ControlHandle), _) => true,
                    (Some(Type::UserInstance { .. }), _) => false,
                    _ => false,
                };

                IRExpr::Index {
                    base: Box::new(self.lower_expr(value)),
                    index: Box::new(self.lower_expr(index)),
                    is_zero_based,
                }
            }

            ast::ExprKind::Error => IRExpr::Nil,
        }
    }

    fn lower_class(&mut self, cls: &ast::ClassDef) -> IRClass {
        let base = cls
            .bases
            .first()
            .filter(|b| !matches!(b.as_str(), "QSysPlugin" | "TextController" | "object"))
            .cloned();

        let prev_base = std::mem::replace(&mut self.current_class_base, base.clone());
        let mut class_vars = Vec::new();
        let mut constructor = None;
        let mut methods = Vec::new();

        for stmt in &cls.body {
            match &stmt.kind {
                ast::StmtKind::FunctionDef(f) => {
                    let lowered_fn = self.lower_class_method(f);

                    if f.name == "__init__" {
                        constructor = Some(lowered_fn);
                    } else {
                        methods.push(lowered_fn);
                    }
                }
                ast::StmtKind::Assign { target, value } => {
                    if let ast::ExprKind::Ident(attr_name) = &target.kind {
                        class_vars.push((attr_name.clone(), self.lower_expr(value)));
                    }
                }
                _ => {}
            }
        }

        // If this subclass has no `__init__`, inherit the nearest ancestor's `__init__`
        if constructor.is_none()
            && let Some(ref base_name) = base
            && let Some((ancestor_cls, ancestor_init)) =
                find_inherited_init(base_name, self.custom_classes)
        {
            let ancestor_base = ancestor_cls
                .bases
                .first()
                .filter(|b| !matches!(b.as_str(), "QSysPlugin" | "TextController" | "object"))
                .cloned();
            let saved = std::mem::replace(&mut self.current_class_base, ancestor_base);
            constructor = Some(self.lower_class_method(ancestor_init));
            self.current_class_base = saved;
        }

        self.current_class_base = prev_base;

        let emit_init_method = class_needs_init_method(&cls.name, self.custom_classes);

        IRClass {
            name: cls.name.clone(),
            base,
            class_vars,
            constructor,
            emit_init_method,
            methods,
            span: cls.span,
        }
    }

    fn lower_class_method(&mut self, f: &ast::FunctionDef) -> IRFunction {
        let func_id = self
            .type_map
            .get_resolution(f.span)
            .and_then(|r| match r {
                ResolvedTarget::Function(id) => Some(*id),
                _ => None,
            })
            .unwrap_or(FunctionId(0));

        // Filter out `self` parameter because Lua methods and constructors
        // manage `self` implicitly via metatables or colon calls
        let params: Vec<_> = f
            .params
            .iter()
            .filter(|p| p.name != "self")
            .map(|p| {
                let sym_id = self
                    .type_map
                    .get_resolution(p.span)
                    .and_then(|r| match r {
                        ResolvedTarget::Local(id) => Some(*id),
                        _ => None,
                    })
                    .unwrap_or(SymbolId(0));
                (sym_id, p.name.clone())
            })
            .collect();

        self.lower_function(func_id, &f.name, &params, &f.body, f.span)
    }
}

fn find_inherited_init<'a>(
    base_name: &str,
    classes: &'a [ast::ClassDef],
) -> Option<(&'a ast::ClassDef, &'a ast::FunctionDef)> {
    let mut current = Some(base_name);
    let mut depth = 0;
    while let Some(target_name) = current {
        if depth > 16 {
            break;
        }
        depth += 1;
        let short_target = target_name.rsplit('.').next().unwrap_or(target_name);
        let cls = classes.iter().find(|c| c.name == short_target)?;
        for stmt in &cls.body {
            if let ast::StmtKind::FunctionDef(f) = &stmt.kind
                && f.name == "__init__"
            {
                return Some((cls, f));
            }
        }
        current = cls
            .bases
            .first()
            .filter(|b| !matches!(b.as_str(), "QSysPlugin" | "TextController" | "object"))
            .map(|s| s.as_str());
    }
    None
}

fn class_needs_init_method(class_name: &str, classes: &[ast::ClassDef]) -> bool {
    classes.iter().any(|c| {
        c.bases.first().is_some_and(|b| {
            let short_b = b.rsplit('.').next().unwrap_or(b.as_str());
            short_b == class_name
        }) && c.body.iter().any(|stmt| {
            if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                f.body.iter().any(stmt_calls_super_init)
            } else {
                false
            }
        })
    })
}

fn stmt_calls_super_init(stmt: &ast::Stmt) -> bool {
    match &stmt.kind {
        ast::StmtKind::Expr(expr)
        | ast::StmtKind::Return(Some(expr))
        | ast::StmtKind::Assign { value: expr, .. }
        | ast::StmtKind::AugAssign { value: expr, .. } => expr_calls_super_init(expr),
        ast::StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_calls_super_init(condition)
                || then_branch.iter().any(stmt_calls_super_init)
                || else_branch.iter().any(stmt_calls_super_init)
        }
        ast::StmtKind::While { condition, body } => {
            expr_calls_super_init(condition) || body.iter().any(stmt_calls_super_init)
        }
        ast::StmtKind::For { iterable, body, .. } => {
            expr_calls_super_init(iterable) || body.iter().any(stmt_calls_super_init)
        }
        _ => false,
    }
}

fn expr_calls_super_init(expr: &ast::Expr) -> bool {
    if let ast::ExprKind::Call { callee, args, .. } = &expr.kind {
        if let ast::ExprKind::Attribute { value, attr } = &callee.kind
            && attr == "__init__"
            && let ast::ExprKind::Call {
                callee: super_callee,
                args: super_args,
                ..
            } = &value.kind
            && super_args.is_empty()
            && matches!(&super_callee.kind, ast::ExprKind::Ident(id) if id == "super")
        {
            return true;
        }
        return expr_calls_super_init(callee) || args.iter().any(expr_calls_super_init);
    }
    false
}

fn lower_binary_op(op: ast::BinaryOp) -> IRBinaryOp {
    match op {
        ast::BinaryOp::Add => IRBinaryOp::Add,
        ast::BinaryOp::Sub => IRBinaryOp::Sub,
        ast::BinaryOp::Mul => IRBinaryOp::Mul,
        ast::BinaryOp::Div => IRBinaryOp::Div,
        ast::BinaryOp::FloorDiv => IRBinaryOp::FloorDiv,
        ast::BinaryOp::Mod => IRBinaryOp::Mod,
        ast::BinaryOp::Pow => IRBinaryOp::Pow,
        ast::BinaryOp::BitAnd => IRBinaryOp::BitAnd,
        ast::BinaryOp::BitOr => IRBinaryOp::BitOr,
        ast::BinaryOp::BitXor => IRBinaryOp::BitXor,
        ast::BinaryOp::LShift => IRBinaryOp::LShift,
        ast::BinaryOp::RShift => IRBinaryOp::RShift,
        ast::BinaryOp::Eq => IRBinaryOp::Eq,
        ast::BinaryOp::NotEq => IRBinaryOp::NotEq,
        ast::BinaryOp::Lt => IRBinaryOp::Lt,
        ast::BinaryOp::LtEq => IRBinaryOp::LtEq,
        ast::BinaryOp::Gt => IRBinaryOp::Gt,
        ast::BinaryOp::GtEq => IRBinaryOp::GtEq,
    }
}

fn lower_unary_op(op: ast::UnaryOp) -> IRUnaryOp {
    match op {
        ast::UnaryOp::Not => IRUnaryOp::Not,
        ast::UnaryOp::Neg => IRUnaryOp::Neg,
        ast::UnaryOp::Pos => IRUnaryOp::Pos,
        ast::UnaryOp::BitNot => IRUnaryOp::BitNot,
    }
}

fn lower_truthiness(strategy: TruthinessStrategy) -> IRTruthiness {
    match strategy {
        TruthinessStrategy::Direct => IRTruthiness::Direct,
        TruthinessStrategy::NumericNotZero => IRTruthiness::NumericNotZero,
        TruthinessStrategy::StringNonEmpty => IRTruthiness::StringNonEmpty,
        TruthinessStrategy::CollectionNonEmpty => IRTruthiness::CollectionNonEmpty,
        TruthinessStrategy::AlwaysFalse => IRTruthiness::AlwaysFalse,
        TruthinessStrategy::Dynamic => IRTruthiness::Dynamic,
    }
}

fn extract_raw_lua(expr: &ast::Expr) -> Option<String> {
    if let ast::ExprKind::Call {
        callee,
        args,
        kwargs,
    } = &expr.kind
        && kwargs.is_empty()
        && args.len() == 1
        && let ast::ExprKind::Attribute { value, attr } = &callee.kind
        && let ast::ExprKind::Ident(ref obj) = value.kind
        && obj == "qsys"
        && attr == "lua"
        && let Some(first_arg) = args.first()
        && let ast::ExprKind::String(ref s) = first_arg.kind
    {
        return Some(s.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_ir::{IRBinaryOp, IRExpr, IRProgram, IRStmt};
    use qsys_parser::parse_module;

    use crate::{analyze, lower_to_ir};

    #[test]
    fn test_string_concatenation_typed_to_ir() {
        let src = r#"
class MyPlugin(QSysPlugin):
    Status = Control.Text(name="Status", default="Ready")

    def runtime(self):
        msg = "State: " + self.Status.String
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).unwrap();
        let model = analyze(&parsed).unwrap();
        let ir = lower_to_ir(&model);

        match ir {
            IRProgram::Plugin(p) => {
                let stmt = &p.methods[0].body[0];
                match stmt {
                    IRStmt::Assign {
                        value:
                            IRExpr::Binary {
                                op: IRBinaryOp::StringConcat,
                                ..
                            },
                        ..
                    } => {}
                    other => panic!("Expected StringConcat binary op, got {other:?}"),
                }
            }
            _ => panic!("Expected Plugin IR"),
        }
    }
}
