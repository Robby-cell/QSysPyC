use ruff_python_ast as ruff;
use ruff_text_size::{Ranged, TextRange};

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};

use crate::ast::*;

#[inline]
pub(crate) fn text_range_to_span(range: TextRange) -> Span {
    Span::new(range.start().to_usize(), range.end().to_usize())
}

fn extract_base_name(expr: &ruff::Expr) -> Option<String> {
    match expr {
        ruff::Expr::Name(n) => Some(n.id.to_string()),
        ruff::Expr::Attribute(a) => {
            let prefix = extract_base_name(&a.value)?;
            Some(format!("{prefix}.{}", a.attr))
        }
        _ => None,
    }
}

pub(crate) struct Lowerer<'a> {
    pub diagnostics: &'a mut Diagnostics,
}

impl<'a> Lowerer<'a> {
    pub fn lower_suite(&mut self, suite: &[ruff::Stmt]) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        for stmt in suite {
            self.lower_stmt(stmt, &mut stmts);
        }
        stmts
    }

    fn lower_operator(&mut self, op: ruff::Operator, span: Span) -> Option<BinaryOp> {
        match op {
            ruff::Operator::Add => Some(BinaryOp::Add),
            ruff::Operator::Sub => Some(BinaryOp::Sub),
            ruff::Operator::Mult => Some(BinaryOp::Mul),
            ruff::Operator::Div => Some(BinaryOp::Div),
            ruff::Operator::FloorDiv => Some(BinaryOp::FloorDiv),
            ruff::Operator::Mod => Some(BinaryOp::Mod),
            ruff::Operator::Pow => Some(BinaryOp::Pow),
            ruff::Operator::BitAnd => Some(BinaryOp::BitAnd),
            ruff::Operator::BitOr => Some(BinaryOp::BitOr),
            ruff::Operator::BitXor => Some(BinaryOp::BitXor),
            ruff::Operator::LShift => Some(BinaryOp::LShift),
            ruff::Operator::RShift => Some(BinaryOp::RShift),
            other => {
                self.diagnostics.report(
                    DiagnosticKind::UnsupportedOperator {
                        op: format!("{:?}", other),
                    },
                    span,
                );
                None
            }
        }
    }

    fn lower_stmt(&mut self, stmt: &ruff::Stmt, out: &mut Vec<Stmt>) {
        let span = text_range_to_span(stmt.range());

        match stmt {
            ruff::Stmt::Import(imp) => {
                for alias in &imp.names {
                    let alias_span = text_range_to_span(alias.range());
                    out.push(Stmt::new(
                        StmtKind::Import {
                            module: alias.name.to_string(),
                            alias: alias.asname.as_ref().map(|a| a.to_string()),
                        },
                        alias_span,
                    ));
                }
            }

            ruff::Stmt::ImportFrom(imp) => {
                let module = imp
                    .module
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or_default();
                let names = imp.names.iter().map(|a| a.name.to_string()).collect();
                let aliases = imp
                    .names
                    .iter()
                    .map(|a| (a.name.to_string(), a.asname.as_ref().map(|n| n.to_string())))
                    .collect();
                out.push(Stmt::new(
                    StmtKind::ImportFrom {
                        module,
                        names,
                        aliases,
                    },
                    span,
                ));
            }

            ruff::Stmt::ClassDef(c) => {
                let bases = c.bases().iter().filter_map(extract_base_name).collect();

                let body = self.lower_suite(&c.body);
                out.push(Stmt::new(
                    StmtKind::ClassDef(ClassDef {
                        name: c.name.to_string(),
                        bases,
                        body,
                        span,
                    }),
                    span,
                ));
            }

            ruff::Stmt::FunctionDef(f) => {
                if f.is_async {
                    self.diagnostics
                        .report(DiagnosticKind::unsupported_async("async def"), span);
                    return;
                }

                let params = f
                    .parameters
                    .args
                    .iter()
                    .map(|arg| Param {
                        name: arg.parameter.name.to_string(),
                        type_annotation: arg
                            .parameter
                            .annotation
                            .as_ref()
                            .map(|a| lower_type_annotation(a)),
                        default: arg.default.as_ref().map(|d| Box::new(self.lower_expr(d))),
                        span: text_range_to_span(arg.range()),
                    })
                    .collect();

                let decorators = f
                    .decorator_list
                    .iter()
                    .map(|d| Decorator {
                        expr: self.lower_expr(&d.expression),
                        span: text_range_to_span(d.range()),
                    })
                    .collect();

                let return_type = f.returns.as_ref().map(|r| lower_type_annotation(r));

                let body = self.lower_suite(&f.body);

                out.push(Stmt::new(
                    StmtKind::FunctionDef(FunctionDef {
                        name: f.name.to_string(),
                        params,
                        decorators,
                        return_type,
                        body,
                        span,
                    }),
                    span,
                ));
            }

            ruff::Stmt::Assign(a) => {
                if a.targets.len() > 1 {
                    self.diagnostics
                        .report(DiagnosticKind::UnsupportedChainedAssignment, span);
                    return;
                }
                if let Some(first_target) = a.targets.first() {
                    let target = self.lower_expr(first_target);
                    let value = self.lower_expr(&a.value);
                    out.push(Stmt::new(
                        StmtKind::Assign {
                            target,
                            value,
                            type_annotation: None,
                        },
                        span,
                    ));
                }
            }

            ruff::Stmt::AnnAssign(a) => {
                let target = self.lower_expr(&a.target);
                let value = a
                    .value
                    .as_ref()
                    .map(|val| self.lower_expr(val))
                    .unwrap_or_else(|| Expr::new(ExprKind::None, span));
                let type_annotation = Some(lower_type_annotation(&a.annotation));
                out.push(Stmt::new(
                    StmtKind::Assign {
                        target,
                        value,
                        type_annotation,
                    },
                    span,
                ));
            }

            ruff::Stmt::AugAssign(a) => {
                let target = self.lower_expr(&a.target);
                let value = self.lower_expr(&a.value);
                if let Some(op) = self.lower_operator(a.op, span) {
                    out.push(Stmt::new(StmtKind::AugAssign { target, op, value }, span));
                }
            }

            ruff::Stmt::If(i) => {
                // Correctly fold elif_else_clauses right-to-left into recursive If trees
                let mut current_else: Vec<Stmt> = Vec::new();

                for clause in i.elif_else_clauses.iter().rev() {
                    let clause_span = text_range_to_span(clause.range);
                    let clause_body = self.lower_suite(&clause.body);

                    if let Some(ref test) = clause.test {
                        let cond = self.lower_expr(test);
                        let elif_stmt = Stmt::new(
                            StmtKind::If {
                                condition: cond,
                                then_branch: clause_body,
                                else_branch: current_else,
                            },
                            clause_span,
                        );
                        current_else = vec![elif_stmt];
                    } else {
                        current_else = clause_body;
                    }
                }

                let condition = self.lower_expr(&i.test);
                let then_branch = self.lower_suite(&i.body);

                out.push(Stmt::new(
                    StmtKind::If {
                        condition,
                        then_branch,
                        else_branch: current_else,
                    },
                    span,
                ));
            }

            ruff::Stmt::While(w) => {
                let condition = self.lower_expr(&w.test);
                let body = self.lower_suite(&w.body);
                out.push(Stmt::new(StmtKind::While { condition, body }, span));
            }

            ruff::Stmt::For(f) => {
                if f.is_async {
                    self.diagnostics
                        .report(DiagnosticKind::unsupported_async("async for"), span);
                    return;
                }

                let Some(targets) = extract_comp_targets(&f.target) else {
                    self.diagnostics.report(
                        DiagnosticKind::UnsupportedForTarget,
                        text_range_to_span(f.target.range()),
                    );
                    return;
                };
                let target = targets
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect::<Vec<_>>()
                    .join(", ");

                let iterable = self.lower_expr(&f.iter);
                let body = self.lower_suite(&f.body);
                out.push(Stmt::new(
                    StmtKind::For {
                        target,
                        iterable,
                        body,
                    },
                    span,
                ));
            }

            ruff::Stmt::Return(r) => {
                let value = r.value.as_ref().map(|v| self.lower_expr(v));
                out.push(Stmt::new(StmtKind::Return(value), span));
            }

            ruff::Stmt::Expr(e) => {
                let expr = self.lower_expr(&e.value);
                out.push(Stmt::new(StmtKind::Expr(expr), span));
            }

            ruff::Stmt::Pass(_) => {
                out.push(Stmt::new(StmtKind::Pass, span));
            }

            ruff::Stmt::Global(g) => {
                let names = g
                    .names
                    .iter()
                    .map(|n| (n.id.to_string(), text_range_to_span(n.range())))
                    .collect();
                out.push(Stmt::new(StmtKind::Global(names), span));
            }

            ruff::Stmt::Nonlocal(n) => {
                let names = n
                    .names
                    .iter()
                    .map(|n| (n.id.to_string(), text_range_to_span(n.range())))
                    .collect();
                out.push(Stmt::new(StmtKind::Nonlocal(names), span));
            }

            ruff::Stmt::Break(_) => out.push(Stmt::new(StmtKind::Break, span)),

            ruff::Stmt::Match(_) => {
                self.diagnostics
                    .report(DiagnosticKind::UnsupportedMatchStatement, span);
            }

            other => {
                self.diagnostics.report(
                    DiagnosticKind::unsupported_statement(format!(
                        "{:?}",
                        std::mem::discriminant(other)
                    )),
                    span,
                );
            }
        }
    }

    fn lower_expr(&mut self, expr: &ruff::Expr) -> Expr {
        let span = text_range_to_span(expr.range());

        let kind = match expr {
            ruff::Expr::Name(n) => ExprKind::Ident(n.id.to_string()),

            ruff::Expr::NumberLiteral(num) => match num.value {
                ruff::Number::Int(ref i) => ExprKind::Int(i.as_i64().unwrap_or(0)),
                ruff::Number::Float(f) => ExprKind::Float(f),
                ruff::Number::Complex { .. } => {
                    self.diagnostics
                        .report(DiagnosticKind::UnsupportedComplexLiteral, span);
                    ExprKind::Error
                }
            },

            ruff::Expr::StringLiteral(s) => ExprKind::String(s.value.to_str().to_string()),

            ruff::Expr::BytesLiteral(b) => {
                let mut s = String::new();
                for part in &b.value {
                    for &byte in part.as_slice() {
                        s.push(char::from(byte));
                    }
                }
                ExprKind::String(s)
            }

            ruff::Expr::FString(f) => {
                let mut parts: Vec<Expr> = Vec::new();

                for part in &f.value {
                    match part {
                        ruff::FStringPartRef::Literal(lit) => {
                            parts.push(Expr::new(
                                ExprKind::String(lit.value.to_string()),
                                text_range_to_span(lit.range()),
                            ));
                        }
                        ruff::FStringPartRef::FString(f_string) => {
                            for element in &f_string.elements {
                                match element {
                                    ruff::InterpolatedStringElement::Literal(lit) => {
                                        parts.push(Expr::new(
                                            ExprKind::String(lit.value.to_string()),
                                            text_range_to_span(lit.range()),
                                        ));
                                    }
                                    ruff::InterpolatedStringElement::Interpolation(interp) => {
                                        let inner = self.lower_expr(&interp.expression);
                                        let span = text_range_to_span(interp.range());
                                        let callee_span = Span::new(span.start, span.start);

                                        let str_call = Expr::new(
                                            ExprKind::Call {
                                                callee: Box::new(Expr::new(
                                                    ExprKind::Ident("str".to_string()),
                                                    callee_span,
                                                )),
                                                args: vec![inner],
                                                kwargs: Vec::new(),
                                            },
                                            span,
                                        );
                                        parts.push(str_call);
                                    }
                                }
                            }
                        }
                    }
                }

                if parts.is_empty() {
                    ExprKind::String(String::new())
                } else {
                    let mut combined = parts.remove(0);
                    for next_part in parts {
                        let span = combined.span.merge(next_part.span);
                        combined = Expr::new(
                            ExprKind::Binary {
                                left: Box::new(combined),
                                op: BinaryOp::Add,
                                right: Box::new(next_part),
                            },
                            span,
                        );
                    }
                    combined.kind
                }
            }

            ruff::Expr::BooleanLiteral(b) => ExprKind::Bool(b.value),
            ruff::Expr::NoneLiteral(_) => ExprKind::None,

            ruff::Expr::Tuple(t) => {
                let elements = t.elts.iter().map(|e| self.lower_expr(e)).collect();
                ExprKind::Tuple(elements)
            }

            ruff::Expr::List(l) => {
                let elts = l.elts.iter().map(|e| self.lower_expr(e)).collect();
                ExprKind::List(elts)
            }

            // Lower Python dictionary literals: { "k": v, 1: 2 }
            ruff::Expr::Dict(d) => {
                let mut pairs = Vec::new();
                for item in &d.items {
                    if let Some(ref key) = item.key {
                        let k = self.lower_expr(key);
                        let v = self.lower_expr(&item.value);
                        pairs.push((k, v));
                    } else {
                        self.diagnostics.report(
                            DiagnosticKind::unsupported_expression(
                                "Dict unpacking ('**dict') is not supported in QSysPy",
                            ),
                            span,
                        );
                    }
                }
                ExprKind::Dict(pairs)
            }

            ruff::Expr::Call(c) => {
                let callee = Box::new(self.lower_expr(&c.func));
                let mut args = Vec::new();
                for a in &c.arguments.args {
                    if let ruff::Expr::Starred(s) = a {
                        let unpacked = self.lower_expr(&s.value);
                        match unpacked.kind {
                            ExprKind::List(items) | ExprKind::Tuple(items) => {
                                args.extend(items);
                            }
                            _ => args.push(unpacked),
                        }
                    } else {
                        args.push(self.lower_expr(a));
                    }
                }
                let kwargs = c
                    .arguments
                    .keywords
                    .iter()
                    .filter_map(|kw| {
                        kw.arg
                            .as_ref()
                            .map(|name| (name.to_string(), self.lower_expr(&kw.value)))
                    })
                    .collect();

                ExprKind::Call {
                    callee,
                    args,
                    kwargs,
                }
            }

            ruff::Expr::ListComp(comp) => {
                self.lower_comprehension(&comp.elt, &comp.generators, span)
            }

            ruff::Expr::Generator(gen_expr) => {
                self.lower_comprehension(&gen_expr.elt, &gen_expr.generators, span)
            }

            ruff::Expr::Attribute(a) => {
                let value = Box::new(self.lower_expr(&a.value));
                let attr = a.attr.to_string();
                ExprKind::Attribute { value, attr }
            }

            ruff::Expr::Subscript(s) => {
                let value = Box::new(self.lower_expr(&s.value));
                let index = Box::new(self.lower_expr(&s.slice));
                ExprKind::Subscript { value, index }
            }

            ruff::Expr::Lambda(lam) => {
                let params = lam
                    .parameters
                    .as_ref()
                    .map(|ps| {
                        ps.args
                            .iter()
                            .map(|arg| Param {
                                name: arg.parameter.name.to_string(),
                                type_annotation: None,
                                default: arg.default.as_ref().map(|d| Box::new(self.lower_expr(d))),
                                span: text_range_to_span(arg.range()),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let body = Box::new(self.lower_expr(&lam.body));
                ExprKind::Lambda { params, body }
            }

            ruff::Expr::If(if_expr) => {
                let test = Box::new(self.lower_expr(&if_expr.test));
                let body = Box::new(self.lower_expr(&if_expr.body));
                let orelse = Box::new(self.lower_expr(&if_expr.orelse));
                ExprKind::IfExp { test, body, orelse }
            }

            ruff::Expr::BinOp(b) => {
                let left = Box::new(self.lower_expr(&b.left));
                let right = Box::new(self.lower_expr(&b.right));
                if let Some(op) = self.lower_operator(b.op, span) {
                    ExprKind::Binary { left, op, right }
                } else {
                    ExprKind::Error
                }
            }

            ruff::Expr::BoolOp(b) => {
                let op = match b.op {
                    ruff::BoolOp::And => LogicalOp::And,
                    ruff::BoolOp::Or => LogicalOp::Or,
                };
                let mut values = b.values.iter();
                if let Some(first) = values.next() {
                    let mut current = self.lower_expr(first);
                    for next in values {
                        let next_expr = self.lower_expr(next);
                        current = Expr::new(
                            ExprKind::BoolOp {
                                op,
                                left: Box::new(current),
                                right: Box::new(next_expr),
                            },
                            span,
                        );
                    }
                    return current;
                } else {
                    ExprKind::Error
                }
            }

            ruff::Expr::Compare(c) => {
                if c.ops.len() > 1 || c.operands.len() > 2 {
                    self.diagnostics
                        .report(DiagnosticKind::UnsupportedChainedComparison, span);
                    return Expr::new(ExprKind::Error, span);
                }

                if let (Some(left_expr), Some(op), Some(right_expr)) =
                    (c.operands.first(), c.ops.first(), c.operands.get(1))
                {
                    let left = Box::new(self.lower_expr(left_expr));
                    let right = Box::new(self.lower_expr(right_expr));
                    let binary_op = match op {
                        ruff::CmpOp::Eq => Some(BinaryOp::Eq),
                        ruff::CmpOp::NotEq => Some(BinaryOp::NotEq),
                        ruff::CmpOp::Lt => Some(BinaryOp::Lt),
                        ruff::CmpOp::LtE => Some(BinaryOp::LtEq),
                        ruff::CmpOp::Gt => Some(BinaryOp::Gt),
                        ruff::CmpOp::GtE => Some(BinaryOp::GtEq),
                        other => {
                            self.diagnostics.report(
                                DiagnosticKind::UnsupportedOperator {
                                    op: format!("{:?}", other),
                                },
                                span,
                            );
                            None
                        }
                    };

                    if let Some(op) = binary_op {
                        ExprKind::Binary { left, op, right }
                    } else {
                        ExprKind::Error
                    }
                } else {
                    ExprKind::Error
                }
            }

            ruff::Expr::UnaryOp(u) => {
                let operand = Box::new(self.lower_expr(&u.operand));
                let op = match u.op {
                    ruff::UnaryOp::Not => UnaryOp::Not,
                    ruff::UnaryOp::USub => UnaryOp::Neg,
                    ruff::UnaryOp::UAdd => UnaryOp::Pos,
                    ruff::UnaryOp::Invert => UnaryOp::BitNot,
                };
                ExprKind::Unary { op, operand }
            }

            ruff::Expr::Yield(_) => {
                self.diagnostics
                    .report(DiagnosticKind::UnsupportedGenerator, span);
                ExprKind::Error
            }
            ruff::Expr::YieldFrom(_) => {
                self.diagnostics
                    .report(DiagnosticKind::UnsupportedYieldFrom, span);
                ExprKind::Error
            }
            ruff::Expr::Await(_) => {
                self.diagnostics
                    .report(DiagnosticKind::unsupported_async("await"), span);
                ExprKind::Error
            }

            other => {
                self.diagnostics.report(
                    DiagnosticKind::unsupported_expression(format!(
                        "{:?}",
                        std::mem::discriminant(other)
                    )),
                    span,
                );
                ExprKind::Error
            }
        };

        Expr::new(kind, span)
    }

    fn lower_comprehension(
        &mut self,
        elt: &ruff::Expr,
        generators: &[ruff::Comprehension],
        span: Span,
    ) -> ExprKind {
        let mut lowered_gens = Vec::with_capacity(generators.len());
        for gen_clause in generators {
            let clause_span = text_range_to_span(gen_clause.range());
            if gen_clause.is_async {
                self.diagnostics
                    .report(DiagnosticKind::unsupported_async("async for"), clause_span);
                return ExprKind::Error;
            }
            let Some(targets) = extract_comp_targets(&gen_clause.target) else {
                self.diagnostics.report(
                    DiagnosticKind::UnsupportedForTarget,
                    text_range_to_span(gen_clause.target.range()),
                );
                return ExprKind::Error;
            };
            let iter = self.lower_expr(&gen_clause.iter);
            let ifs = gen_clause
                .ifs
                .iter()
                .map(|cond| self.lower_expr(cond))
                .collect();
            lowered_gens.push(Comprehension {
                targets,
                iter,
                ifs,
                span: clause_span,
            });
        }
        let lowered_elt = Box::new(self.lower_expr(elt));
        let _ = span;
        ExprKind::ListComp {
            elt: lowered_elt,
            generators: lowered_gens,
        }
    }
}

fn extract_comp_targets(target: &ruff::Expr) -> Option<Vec<(String, Span)>> {
    match target {
        ruff::Expr::Name(n) => Some(vec![(n.id.to_string(), text_range_to_span(n.range()))]),
        ruff::Expr::Tuple(t) => {
            let mut out = Vec::with_capacity(t.elts.len());
            for elt in &t.elts {
                if let ruff::Expr::Name(n) = elt {
                    out.push((n.id.to_string(), text_range_to_span(n.range())));
                } else {
                    return None;
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        ruff::Expr::List(l) => {
            let mut out = Vec::with_capacity(l.elts.len());
            for elt in &l.elts {
                if let ruff::Expr::Name(n) = elt {
                    out.push((n.id.to_string(), text_range_to_span(n.range())));
                } else {
                    return None;
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        _ => None,
    }
}

fn lower_type_annotation(expr: &ruff::Expr) -> String {
    match expr {
        ruff::Expr::Name(n) => n.id.to_string(),
        ruff::Expr::StringLiteral(s) => s.value.to_str().to_string(),
        ruff::Expr::Attribute(a) => {
            if let ruff::Expr::Name(base) = a.value.as_ref() {
                if base.id.as_str() == "typing" {
                    a.attr.to_string()
                } else {
                    format!("{}.{}", base.id, a.attr)
                }
            } else {
                a.attr.to_string()
            }
        }
        ruff::Expr::Subscript(s) => {
            let base_name = match s.value.as_ref() {
                ruff::Expr::Name(base) => Some(base.id.as_str()),
                ruff::Expr::Attribute(a) => Some(a.attr.as_str()),
                _ => None,
            };
            if let Some(base) = base_name {
                match base {
                    "Optional" => return lower_type_annotation(&s.slice),
                    "Union" => {
                        if let ruff::Expr::Tuple(t) = s.slice.as_ref() {
                            for elt in &t.elts {
                                let lowered = lower_type_annotation(elt);
                                if lowered != "None" {
                                    return lowered;
                                }
                            }
                            return "None".to_string();
                        }
                        return lower_type_annotation(&s.slice);
                    }
                    "list" | "List" | "Sequence" | "Iterable" | "set" | "Set" => {
                        return format!("list[{}]", lower_type_annotation(&s.slice));
                    }
                    "dict" | "Dict" | "Mapping" => {
                        if let ruff::Expr::Tuple(t) = s.slice.as_ref()
                            && t.elts.len() == 2
                        {
                            return format!(
                                "dict[{}, {}]",
                                lower_type_annotation(&t.elts[0]),
                                lower_type_annotation(&t.elts[1])
                            );
                        }
                        return "dict".to_string();
                    }
                    "tuple" | "Tuple" => {
                        if let ruff::Expr::Tuple(t) = s.slice.as_ref() {
                            let elems: Vec<String> =
                                t.elts.iter().map(lower_type_annotation).collect();
                            return format!("({})", elems.join(", "));
                        }
                        return format!("({})", lower_type_annotation(&s.slice));
                    }
                    "type" | "Type" => {
                        return format!("type[{}]", lower_type_annotation(&s.slice));
                    }
                    _ => {}
                }
            }
            "Any".to_string()
        }
        ruff::Expr::BinOp(b) => {
            let left = lower_type_annotation(&b.left);
            let right = lower_type_annotation(&b.right);
            if left == "None" { right } else { left }
        }
        ruff::Expr::Tuple(t) => {
            let elems: Vec<String> = t.elts.iter().map(lower_type_annotation).collect();
            format!("({})", elems.join(", "))
        }
        ruff::Expr::NoneLiteral(_) => "None".to_string(),
        _ => "Any".to_string(),
    }
}
