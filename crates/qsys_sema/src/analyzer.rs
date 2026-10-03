use qsys_diagnostics::{DiagnosticKind, Diagnostics};
use qsys_model::ControlMember;
use qsys_parser::ast;

use crate::symbols::{ScopeKind, ScopeStack, Symbol, SymbolKind};
use crate::types::{ResolvedTarget, Type, TypeMap};

pub struct ExprAnalyzer<'a> {
    pub scope_stack: &'a mut ScopeStack,
    pub type_map: &'a mut TypeMap,
    pub diagnostics: &'a mut Diagnostics,
}

impl<'a> ExprAnalyzer<'a> {
    pub fn new(
        scope_stack: &'a mut ScopeStack,
        type_map: &'a mut TypeMap,
        diagnostics: &'a mut Diagnostics,
    ) -> Self {
        Self {
            scope_stack,
            type_map,
            diagnostics,
        }
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
                if let ast::ExprKind::Ident(ref name) = target.kind {
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
                    self.type_map
                        .insert_resolution(target.span, ResolvedTarget::Local(sym_id));
                    self.type_map.insert_type(target.span, val_ty);
                    self.type_map.insert_symbol_name(sym_id, name.clone());
                } else if let ast::ExprKind::Subscript {
                    ref value,
                    ref index,
                } = target.kind
                {
                    let val_ty = self.infer_expr(value);
                    let _ = self.infer_expr(index);
                    self.type_map.insert_type(target.span, val_ty);
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

                // 1. Register function in outer scope with its real allocated IDs
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

                // 2. Push function scope for parameters and body
                self.scope_stack.push_scope(ScopeKind::Function {
                    name: f.name.clone(),
                });

                for (idx, param) in f.params.iter().enumerate() {
                    let param_sym_id = self.scope_stack.alloc_symbol_id();
                    self.type_map
                        .insert_symbol_name(param_sym_id, param.name.clone());
                    self.scope_stack.define(Symbol::new(
                        param_sym_id,
                        param.name.clone(),
                        SymbolKind::Parameter {
                            id: param_sym_id,
                            index: idx,
                            ty: Type::Any,
                        },
                        param.span,
                    ));
                }

                self.analyze_statements(&f.body);
                self.scope_stack.pop_scope();
            }

            ast::StmtKind::ClassDef(c) => {
                let sym_id = self.scope_stack.alloc_symbol_id();

                self.scope_stack.define(Symbol::new(
                    sym_id,
                    c.name.clone(),
                    SymbolKind::Class {
                        name: c.name.clone(),
                        base: c.bases.first().cloned(),
                    },
                    c.span,
                ));

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

            // Recognize Socket/Timer global classes
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
                        SymbolKind::Class { name, .. } | SymbolKind::UserClass { name } => {
                            Type::UserClass { name: name.clone() }
                        }
                        _ => Type::Any,
                    }
                } else {
                    Type::Any
                }
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
                        _ => Type::Any,
                    },
                    Type::ClassInstance { .. } => {
                        // Check if it's a declared control
                        if let Some(symbol) = self.scope_stack.lookup(attr)
                            && matches!(symbol.kind, SymbolKind::Control { .. })
                        {
                            return Type::ControlHandle;
                        }
                        // Allow standard plugin metadata attributes (self.name, self.version, etc.) without E0207
                        if matches!(
                            attr.as_str(),
                            "name" | "version" | "author" | "description" | "id" | "show_debug"
                        ) {
                            return Type::String;
                        }
                        Type::Any
                    }
                    _ => Type::Any,
                }
            }

            ast::ExprKind::Subscript { value, index } => {
                let val_ty = self.infer_expr(value);
                let _ = self.infer_expr(index);
                match val_ty {
                    Type::List(elem) => *elem,
                    Type::Dict(_, val) => *val,
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
                            Type::String // Python string repetition
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
                    _ => Type::Any,
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
                if let Type::UserClass { name } = callee_ty {
                    for arg in args {
                        let _ = self.infer_expr(arg);
                    }
                    for (_, kw_val) in kwargs {
                        let _ = self.infer_expr(kw_val);
                    }
                    let inst_ty = Type::UserInstance { name };
                    self.type_map.insert_type(expr.span, inst_ty.clone());
                    return inst_ty;
                }

                if let ast::ExprKind::Attribute { value, attr } = &callee.kind
                    && attr == "New"
                {
                    let class_ty = self.infer_expr(value);
                    let return_ty = match class_ty {
                        Type::UdpSocketClass => Type::UdpSocketHandle,
                        Type::TcpSocketClass => Type::TcpSocketHandle,
                        Type::TimerClass => Type::TimerHandle,
                        _ => Type::Any,
                    };
                    if return_ty != Type::Any {
                        self.type_map.insert_type(expr.span, return_ty.clone());
                        return return_ty;
                    }
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

                let _ = self.infer_expr(callee);
                for arg in args {
                    let _ = self.infer_expr(arg);
                }
                for (_, kw_val) in kwargs {
                    let _ = self.infer_expr(kw_val);
                }
                Type::Any
            }

            _ => Type::Any,
        };

        self.type_map.insert_type(expr.span, ty.clone());
        ty
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
