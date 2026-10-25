use strsim::levenshtein;

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::{ControlId, FunctionId};
use qsys_model::ControlMember;
use qsys_parser::ast;

use crate::declarations::eval_constant;
use crate::symbols::{ScopeStack, SymbolKind};

#[derive(Debug, Clone)]
pub struct ResolvedBinding {
    pub control_id: ControlId,
    pub control_name: String,
    pub channel_index: Option<usize>,
    pub member: ControlMember,
    pub handler_id: FunctionId,
    pub handler_name: String,
    pub span: Span,
}

pub fn find_closest_member(target: &str) -> Option<&'static str> {
    let mut best_match = None;
    let mut min_distance = usize::MAX;

    for member_name in ControlMember::all_member_names() {
        let dist = levenshtein(target, member_name);
        if dist <= 3 && dist < min_distance {
            min_distance = dist;
            best_match = Some(*member_name);
        }
    }

    best_match
}

pub fn resolve_event_decorator(
    decorator: &ast::Decorator,
    method_id: FunctionId,
    method: &ast::FunctionDef,
    scope_stack: &ScopeStack,
    diagnostics: &mut Diagnostics,
) -> Option<ResolvedBinding> {
    let (ctl_name, subscript_expr, second_subscript_expr, member_name) = match &decorator.expr.kind
    {
        ast::ExprKind::Attribute { value, attr } => match &value.kind {
            ast::ExprKind::Ident(id) => (id.as_str(), None, None, attr.as_str()),
            ast::ExprKind::Subscript {
                value: sub_val,
                index: sub_idx,
            } => {
                if let ast::ExprKind::Ident(id) = &sub_val.kind {
                    (id.as_str(), Some(sub_idx.as_ref()), None, attr.as_str())
                } else if let ast::ExprKind::Attribute {
                    value: self_val,
                    attr: ctl_attr,
                } = &sub_val.kind
                    && matches!(&self_val.kind, ast::ExprKind::Ident(s) if s == "self")
                {
                    (
                        ctl_attr.as_str(),
                        Some(sub_idx.as_ref()),
                        None,
                        attr.as_str(),
                    )
                } else if let ast::ExprKind::Subscript {
                    value: inner_val,
                    index: first_idx,
                } = &sub_val.kind
                {
                    if let ast::ExprKind::Ident(id) = &inner_val.kind {
                        (
                            id.as_str(),
                            Some(first_idx.as_ref()),
                            Some(sub_idx.as_ref()),
                            attr.as_str(),
                        )
                    } else if let ast::ExprKind::Attribute {
                        value: self_val,
                        attr: ctl_attr,
                    } = &inner_val.kind
                        && matches!(&self_val.kind, ast::ExprKind::Ident(s) if s == "self")
                    {
                        (
                            ctl_attr.as_str(),
                            Some(first_idx.as_ref()),
                            Some(sub_idx.as_ref()),
                            attr.as_str(),
                        )
                    } else {
                        diagnostics.report(
                            DiagnosticKind::unsupported_expression(
                                "Complex decorator expressions are not supported on event handlers",
                            ),
                            decorator.span,
                        );
                        return None;
                    }
                } else {
                    diagnostics.report(
                        DiagnosticKind::unsupported_expression(
                            "Complex decorator expressions are not supported on event handlers",
                        ),
                        decorator.span,
                    );
                    return None;
                }
            }
            ast::ExprKind::Attribute {
                value: self_val,
                attr: ctl_attr,
            } if matches!(&self_val.kind, ast::ExprKind::Ident(s) if s == "self") => {
                (ctl_attr.as_str(), None, None, attr.as_str())
            }
            _ => {
                diagnostics.report(
                    DiagnosticKind::unsupported_expression(
                        "Complex decorator expressions are not supported on event handlers",
                    ),
                    decorator.span,
                );
                return None;
            }
        },
        _ => return None,
    };

    let (qsys_control_id, qsys_control_name, ctl_count, ctl_cols, has_prop_count) =
        match scope_stack.lookup(ctl_name) {
            Some(sym) => {
                if let SymbolKind::Control { id, def } = &sym.kind {
                    (
                        *id,
                        def.name.clone(),
                        def.count,
                        def.cols.unwrap_or(1),
                        def.count_property.is_some(),
                    )
                } else {
                    diagnostics.report(
                        DiagnosticKind::UnknownControlMember {
                            control_name: ctl_name.to_string(),
                            member: member_name.to_string(),
                            candidates: vec![],
                        },
                        decorator.span,
                    );
                    return None;
                }
            }
            None => {
                diagnostics.report(
                    DiagnosticKind::UnknownControlMember {
                        control_name: ctl_name.to_string(),
                        member: member_name.to_string(),
                        candidates: vec![],
                    },
                    decorator.span,
                );
                return None;
            }
        };

    let pair_exprs = match (subscript_expr, second_subscript_expr) {
        (Some(first), Some(second)) => Some((first, Some(second))),
        (Some(first), None) => {
            if let ast::ExprKind::Tuple(items) = &first.kind
                && items.len() == 2
            {
                Some((&items[0], Some(&items[1])))
            } else {
                Some((first, None))
            }
        }
        (None, _) => None,
    };

    let channel_index = if let Some((idx_expr, col_expr_opt)) = pair_exprs {
        let Some(idx_val) = eval_constant(idx_expr).and_then(|c| c.as_i64()) else {
            diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: ctl_name.to_string(),
                    message: "Event decorator channel index must be an integer literal".to_string(),
                },
                idx_expr.span,
            );
            return None;
        };
        let flat_idx = if let Some(col_expr) = col_expr_opt {
            let Some(col_val) = eval_constant(col_expr).and_then(|c| c.as_i64()) else {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: ctl_name.to_string(),
                        message: "Event decorator channel index must be an integer literal"
                            .to_string(),
                    },
                    col_expr.span,
                );
                return None;
            };
            if idx_val < 0 || col_val < 0 {
                -1
            } else {
                idx_val * (ctl_cols as i64) + col_val
            }
        } else {
            idx_val
        };
        if flat_idx < 0 || (!has_prop_count && (flat_idx as usize) >= ctl_count) {
            diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: ctl_name.to_string(),
                    message: format!(
                        "Control '{}' has count {}; index {} is out of bounds (expected 0..{})",
                        ctl_name,
                        ctl_count,
                        flat_idx,
                        ctl_count.saturating_sub(1)
                    ),
                },
                idx_expr.span,
            );
            return None;
        }
        Some(flat_idx as usize)
    } else {
        None
    };

    let member = match ControlMember::from_ident(member_name) {
        Some(m) => m,
        None => {
            let candidates = find_closest_member(member_name)
                .map(|c| vec![c.to_string()])
                .unwrap_or_default();

            diagnostics.report(
                DiagnosticKind::UnknownControlMember {
                    control_name: ctl_name.to_string(),
                    member: member_name.to_string(),
                    candidates,
                },
                decorator.span,
            );
            return None;
        }
    };

    if method.params.is_empty() || method.params[0].name != "self" {
        diagnostics.report(
            DiagnosticKind::MissingSelfArgument {
                method_name: method.name.clone(),
            },
            method.span,
        );
        return None;
    }

    Some(ResolvedBinding {
        control_id: qsys_control_id,
        control_name: qsys_control_name,
        channel_index,
        member,
        handler_id: method_id,
        handler_name: method.name.clone(),
        span: decorator.span,
    })
}
