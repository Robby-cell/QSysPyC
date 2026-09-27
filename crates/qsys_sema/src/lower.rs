use qsys_ir::*;
use qsys_parser::ast;

use crate::types::{ResolvedTarget, Type};
use crate::{SemanticModel, TargetKind, TruthinessStrategy, TypeMap};

pub fn lower_to_ir(model: &SemanticModel) -> IRProgram {
    match &model.target {
        TargetKind::Plugin(p) => {
            let mut lowerer = StmtLowerer::new(&model.type_map);

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

            IRProgram::Plugin(IRPlugin {
                info: p.info.clone(),
                properties,
                controls,
                class_name: p.class_name.clone(),
                classes,
                methods,
                event_bindings,
                init_body,
                span: p.span,
            })
        }

        TargetKind::TextController(c) => {
            let mut lowerer = StmtLowerer::new(&model.type_map);

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

            let body = lowerer.lower_statements(&c.init_statements);

            let event_bindings = c
                .event_bindings
                .iter()
                .map(|b| IREventBinding {
                    control_id: b.control_id,
                    control_name: b.control_name.clone(),
                    member: b.member,
                    handler_id: b.handler_id,
                    handler_name: b.handler_name.clone(),
                    span: b.span,
                })
                .collect();

            let bound_pins = c.controls.iter().map(|ctl| ctl.def.name.clone()).collect();

            IRProgram::Script(IRScript {
                name: c.class_name.clone(),
                is_controller: true,
                bound_pins,
                classes,
                methods,
                event_bindings,
                body,
                span: c.span,
            })
        }

        TargetKind::ProceduralScript(s) => {
            let mut lowerer = StmtLowerer::new(&model.type_map);

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

            IRProgram::Script(IRScript {
                name: "main".to_string(),
                is_controller: false,
                bound_pins: s.bound_pins.clone(),
                classes,
                methods,
                event_bindings: Vec::new(),
                body,
                span: qsys_diagnostics::Span::new(0, 0),
            })
        }
    }
}

struct StmtLowerer<'a> {
    type_map: &'a TypeMap,
}

impl<'a> StmtLowerer<'a> {
    fn new(type_map: &'a TypeMap) -> Self {
        Self { type_map }
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
                let target_ir = self.lower_expr(target);
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
                let val_ir = val.as_ref().map(|v| self.lower_expr(v));
                Some(IRStmt::Return {
                    value: val_ir,
                    span,
                })
            }

            ast::StmtKind::Expr(expr) => {
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

            ast::StmtKind::Pass
            | ast::StmtKind::Import { .. }
            | ast::StmtKind::ImportFrom { .. } => None,

            _ => None,
        }
    }

    fn lower_expr(&self, expr: &ast::Expr) -> IRExpr {
        if let Some(target) = self.type_map.get_resolution(expr.span) {
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
                let mut ir_op = lower_binary_op(*op);
                let left_ty = self.type_map.get_type(left.span);
                let right_ty = self.type_map.get_type(right.span);

                if *op == ast::BinaryOp::Add {
                    if matches!(left_ty, Some(Type::String))
                        || matches!(right_ty, Some(Type::String))
                    {
                        ir_op = IRBinaryOp::StringConcat;
                    }
                } else if *op == ast::BinaryOp::Mul
                    && ((matches!(left_ty, Some(Type::String))
                        && matches!(right_ty, Some(Type::Int)))
                        || (matches!(left_ty, Some(Type::Int))
                            && matches!(right_ty, Some(Type::String))))
                {
                    ir_op = IRBinaryOp::StringRep;
                }

                // If `reps * "a"`, swap operands so Lua receives (str, count) -> string.rep("a", reps)
                if ir_op == IRBinaryOp::StringRep && matches!(left_ty, Some(Type::Int)) {
                    return IRExpr::Binary {
                        left: Box::new(self.lower_expr(right)),
                        op: ir_op,
                        right: Box::new(self.lower_expr(left)),
                    };
                }

                IRExpr::Binary {
                    left: Box::new(self.lower_expr(left)),
                    op: ir_op,
                    right: Box::new(self.lower_expr(right)),
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
                    let args_ir: Vec<IRExpr> =
                        args.iter().map(|arg| self.lower_expr(arg)).collect();
                    let kwargs_ir = kwargs
                        .iter()
                        .map(|(k, v)| (k.clone(), self.lower_expr(v)))
                        .collect();

                    let callee_ty = self.type_map.get_type(callee.span);

                    // Calling a custom class: `Device(...)` -> lowers to `Device.new(...)`
                    if let Some(Type::UserClass { name }) = callee_ty {
                        return IRExpr::Call {
                            callee: Box::new(IRExpr::Attribute {
                                object: Box::new(IRExpr::Global(name.clone())),
                                attr: "new".to_string(),
                            }),
                            args: args_ir,
                            kwargs: kwargs_ir,
                        };
                    }

                    // If calling an instance method on a Socket or Timer, lower to MethodCall (`object:method(...)`)
                    if let ast::ExprKind::Attribute { value, attr } = &callee.kind {
                        let val_ty = self.type_map.get_type(value.span);
                        if matches!(val_ty, Some(Type::UserInstance { .. })) {
                            return IRExpr::MethodCall {
                                object: Box::new(self.lower_expr(value)),
                                method: attr.clone(),
                                args: args_ir,
                                kwargs: kwargs_ir,
                            };
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
                    (Some(Type::List(_)), _) => true,
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
        let mut constructor = None;
        let mut methods = Vec::new();

        for stmt in &cls.body {
            if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                let func_id = FunctionId(0);
                // Filter out `self` parameter because Lua methods and constructors
                // manage `self` implicitly via metatables or colon calls
                let params: Vec<_> = f
                    .params
                    .iter()
                    .filter(|p| p.name != "self")
                    .map(|p| (SymbolId(0), p.name.clone()))
                    .collect();

                let lowered_fn = self.lower_function(func_id, &f.name, &params, &f.body, f.span);

                if f.name == "__init__" {
                    constructor = Some(lowered_fn);
                } else {
                    methods.push(lowered_fn);
                }
            }
        }

        IRClass {
            name: cls.name.clone(),
            constructor,
            methods,
            span: cls.span,
        }
    }
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
