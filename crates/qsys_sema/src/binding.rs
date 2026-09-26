use strsim::levenshtein;

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::{ControlId, FunctionId};
use qsys_model::ControlMember;
use qsys_parser::ast;

use crate::symbols::{ScopeStack, SymbolKind};

#[derive(Debug, Clone)]
pub struct ResolvedBinding {
    pub control_id: ControlId,
    pub control_name: String,
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
    let (ctl_name, member_name) = match &decorator.expr.kind {
        ast::ExprKind::Attribute { value, attr } => {
            if let ast::ExprKind::Ident(ref id) = value.kind {
                (id.as_str(), attr.as_str())
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
        _ => return None,
    };

    let (qsys_control_id, qsys_control_name) = match scope_stack.lookup(ctl_name) {
        Some(sym) => {
            if let SymbolKind::Control { id, def } = &sym.kind {
                (*id, def.name.clone())
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
        member,
        handler_id: method_id,
        handler_name: method.name.clone(),
        span: decorator.span,
    })
}
