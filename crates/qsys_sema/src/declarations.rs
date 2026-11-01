use rustc_hash::{FxHashMap, FxHashSet};

use qsys_diagnostics::{DiagnosticKind, Diagnostics, Span};
use qsys_ir::{ControlId, PropertyId};
use qsys_model::{
    ButtonType, ControlDef, ControlKind, ControlUnit, GenericControlSpec, PageLayout, PinStyle,
    PluginInfo, PropertyDef, PropertyKind, TextType,
};
use qsys_parser::ast;

use crate::symbols::ScopeStack;
use crate::ui_layout::{
    UiResolveEnv, collect_inline_ui_pages_from_stmts, extract_declarative_layout_from_method,
    is_ui_layout_expr, parse_ui_layout_expr_with_env,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    List(Vec<ConstValue>),
    Dict(Vec<(ConstValue, ConstValue)>),
    Symbol(String),
}

impl ConstValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Int(i) => *i != 0,
            Self::Float(f) => *f != 0.0,
            Self::String(s) => !s.is_empty(),
            Self::List(items) => !items.is_empty(),
            Self::Dict(pairs) => !pairs.is_empty(),
            Self::Symbol(_) => true,
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::String(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
            Self::List(items) => {
                let parts: Vec<String> = items.iter().map(Self::to_display_string).collect();
                format!("[{}]", parts.join(", "))
            }
            Self::Dict(pairs) => {
                let parts: Vec<String> = pairs
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.to_display_string(), v.to_display_string()))
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            Self::Symbol(name) => name.clone(),
        }
    }
}

pub fn eval_constant_with_env(
    expr: &ast::Expr,
    env: &rustc_hash::FxHashMap<String, ConstValue>,
) -> Option<ConstValue> {
    match &expr.kind {
        ast::ExprKind::Int(i) => Some(ConstValue::Int(*i)),
        ast::ExprKind::Float(f) => Some(ConstValue::Float(*f)),
        ast::ExprKind::String(s) => Some(ConstValue::String(s.clone())),
        ast::ExprKind::Bool(b) => Some(ConstValue::Bool(*b)),
        ast::ExprKind::Ident(name) => env.get(name).cloned(),
        ast::ExprKind::Attribute { value, attr } => {
            if let ast::ExprKind::Ident(obj) = &value.kind {
                if obj == "self" {
                    return env.get(attr).cloned();
                }
                if let Some(cval) = env.get(&format!("{obj}.{attr}")) {
                    return Some(cval.clone());
                }
            }
            if attr == "Value" {
                return eval_constant_with_env(value, env);
            }
            None
        }
        ast::ExprKind::List(items) | ast::ExprKind::Tuple(items) => {
            let mut list = Vec::with_capacity(items.len());
            for item in items {
                list.push(eval_constant_with_env(item, env)?);
            }
            Some(ConstValue::List(list))
        }
        ast::ExprKind::Dict(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                let k_val = eval_constant_with_env(k, env)?;
                let v_val = eval_constant_with_env(v, env)
                    .unwrap_or_else(|| ConstValue::Symbol("<expr>".to_string()));
                if let Some(existing) = out.iter_mut().find(|(ek, _)| *ek == k_val) {
                    existing.1 = v_val;
                } else {
                    out.push((k_val, v_val));
                }
            }
            Some(ConstValue::Dict(out))
        }
        ast::ExprKind::ListComp { elt, generators } => {
            let mut out = Vec::new();
            eval_const_comprehension(elt, generators, env, &mut out)?;
            Some(ConstValue::List(out))
        }
        ast::ExprKind::Subscript { value, index } => match eval_constant_with_env(value, env)? {
            ConstValue::List(items) => {
                let idx = eval_constant_with_env(index, env)?.as_i64()?;
                let resolved = if idx < 0 {
                    items.len() as i64 + idx
                } else {
                    idx
                };
                if resolved >= 0 {
                    items.get(resolved as usize).cloned()
                } else {
                    None
                }
            }
            ConstValue::Dict(pairs) => {
                let key = eval_constant_with_env(index, env)?;
                pairs
                    .into_iter()
                    .find_map(|(k, v)| if k == key { Some(v) } else { None })
            }
            _ => None,
        },
        ast::ExprKind::Unary { op, operand } => {
            let val = eval_constant_with_env(operand, env)?;
            match op {
                ast::UnaryOp::Neg => match val {
                    ConstValue::Int(i) => Some(ConstValue::Int(i.saturating_neg())),
                    ConstValue::Float(f) => Some(ConstValue::Float(-f)),
                    _ => None,
                },
                ast::UnaryOp::Pos => match val {
                    ConstValue::Int(_) | ConstValue::Float(_) => Some(val),
                    _ => None,
                },
                ast::UnaryOp::Not => Some(ConstValue::Bool(!val.is_truthy())),
                ast::UnaryOp::BitNot => match val {
                    ConstValue::Int(i) => Some(ConstValue::Int(!i)),
                    _ => None,
                },
            }
        }
        ast::ExprKind::Binary { left, op, right } => {
            let l = eval_constant_with_env(left, env)?;
            let r = eval_constant_with_env(right, env)?;
            eval_const_binary_op(l, *op, r)
        }
        ast::ExprKind::BoolOp { op, left, right } => {
            let l = eval_constant_with_env(left, env)?;
            match op {
                ast::LogicalOp::And => {
                    if !l.is_truthy() {
                        Some(l)
                    } else {
                        eval_constant_with_env(right, env)
                    }
                }
                ast::LogicalOp::Or => {
                    if l.is_truthy() {
                        Some(l)
                    } else {
                        eval_constant_with_env(right, env)
                    }
                }
            }
        }
        ast::ExprKind::IfExp { test, body, orelse } => {
            let cond = eval_constant_with_env(test, env)?;
            if cond.is_truthy() {
                eval_constant_with_env(body, env)
            } else {
                eval_constant_with_env(orelse, env)
            }
        }
        ast::ExprKind::Call { callee, args, .. } => {
            if let ast::ExprKind::Attribute { value: recv, attr } = &callee.kind {
                let recv_val = eval_constant_with_env(recv, env)?;
                let mut eval_args = Vec::with_capacity(args.len());
                for a in args {
                    eval_args.push(eval_constant_with_env(a, env)?);
                }
                match recv_val {
                    ConstValue::String(s) => {
                        return match attr.as_str() {
                            "upper" if eval_args.is_empty() => {
                                Some(ConstValue::String(s.to_uppercase()))
                            }
                            "lower" if eval_args.is_empty() => {
                                Some(ConstValue::String(s.to_lowercase()))
                            }
                            "strip" if eval_args.is_empty() => {
                                Some(ConstValue::String(s.trim().to_string()))
                            }
                            "replace" if eval_args.len() == 2 => {
                                let from = eval_args[0].as_str()?;
                                let to = eval_args[1].as_str()?;
                                Some(ConstValue::String(s.replace(from, to)))
                            }
                            _ => None,
                        };
                    }
                    ConstValue::Dict(pairs) => {
                        return match attr.as_str() {
                            "keys" if eval_args.is_empty() => Some(ConstValue::List(
                                pairs.into_iter().map(|(k, _)| k).collect(),
                            )),
                            "values" if eval_args.is_empty() => Some(ConstValue::List(
                                pairs.into_iter().map(|(_, v)| v).collect(),
                            )),
                            "items" if eval_args.is_empty() => Some(ConstValue::List(
                                pairs
                                    .into_iter()
                                    .map(|(k, v)| ConstValue::List(vec![k, v]))
                                    .collect(),
                            )),
                            "get" if (1..=2).contains(&eval_args.len()) => {
                                let key = &eval_args[0];
                                pairs
                                    .into_iter()
                                    .find_map(|(k, v)| if &k == key { Some(v) } else { None })
                                    .or_else(|| eval_args.get(1).cloned())
                            }
                            _ => None,
                        };
                    }
                    ConstValue::List(items) => {
                        return match attr.as_str() {
                            "copy" if eval_args.is_empty() => Some(ConstValue::List(items)),
                            _ => None,
                        };
                    }
                    _ => return None,
                }
            }
            let ast::ExprKind::Ident(fn_name) = &callee.kind else {
                return None;
            };
            if fn_name == "list"
                && args.len() == 1
                && let Some(items) = eval_const_iterable(&args[0], env)
            {
                return Some(ConstValue::List(items));
            }
            let mut eval_args = Vec::with_capacity(args.len());
            for a in args {
                eval_args.push(eval_constant_with_env(a, env)?);
            }
            match fn_name.as_str() {
                "len" if eval_args.len() == 1 => match &eval_args[0] {
                    ConstValue::List(items) => Some(ConstValue::Int(items.len() as i64)),
                    ConstValue::Dict(pairs) => Some(ConstValue::Int(pairs.len() as i64)),
                    ConstValue::String(s) => Some(ConstValue::Int(s.chars().count() as i64)),
                    _ => None,
                },
                "str" if eval_args.len() == 1 => {
                    Some(ConstValue::String(eval_args[0].to_display_string()))
                }
                "int" if eval_args.len() == 1 => match &eval_args[0] {
                    ConstValue::Int(i) => Some(ConstValue::Int(*i)),
                    ConstValue::Float(f) => Some(ConstValue::Int(*f as i64)),
                    ConstValue::Bool(b) => Some(ConstValue::Int(i64::from(*b))),
                    ConstValue::String(s) => s.trim().parse::<i64>().ok().map(ConstValue::Int),
                    ConstValue::List(_) | ConstValue::Dict(_) | ConstValue::Symbol(_) => None,
                },
                "float" if eval_args.len() == 1 => match &eval_args[0] {
                    ConstValue::Float(f) => Some(ConstValue::Float(*f)),
                    ConstValue::Int(i) => Some(ConstValue::Float(*i as f64)),
                    ConstValue::Bool(b) => Some(ConstValue::Float(if *b { 1.0 } else { 0.0 })),
                    ConstValue::String(s) => s.trim().parse::<f64>().ok().map(ConstValue::Float),
                    ConstValue::List(_) | ConstValue::Dict(_) | ConstValue::Symbol(_) => None,
                },
                "bool" if eval_args.len() == 1 => Some(ConstValue::Bool(eval_args[0].is_truthy())),
                "abs" if eval_args.len() == 1 => match eval_args[0] {
                    ConstValue::Int(i) => Some(ConstValue::Int(i.abs())),
                    ConstValue::Float(f) => Some(ConstValue::Float(f.abs())),
                    _ => None,
                },
                "round" if eval_args.len() == 1 => match eval_args[0] {
                    ConstValue::Int(i) => Some(ConstValue::Int(i)),
                    ConstValue::Float(f) => Some(ConstValue::Int(f.round() as i64)),
                    _ => None,
                },
                "min" if !eval_args.is_empty() => {
                    if eval_args.iter().all(|v| matches!(v, ConstValue::Int(_))) {
                        eval_args
                            .iter()
                            .filter_map(ConstValue::as_i64)
                            .min()
                            .map(ConstValue::Int)
                    } else {
                        eval_args
                            .iter()
                            .filter_map(ConstValue::as_f64)
                            .reduce(f64::min)
                            .map(ConstValue::Float)
                    }
                }
                "max" if !eval_args.is_empty() => {
                    if eval_args.iter().all(|v| matches!(v, ConstValue::Int(_))) {
                        eval_args
                            .iter()
                            .filter_map(ConstValue::as_i64)
                            .max()
                            .map(ConstValue::Int)
                    } else {
                        eval_args
                            .iter()
                            .filter_map(ConstValue::as_f64)
                            .reduce(f64::max)
                            .map(ConstValue::Float)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

pub fn eval_const_binary_op(l: ConstValue, op: ast::BinaryOp, r: ConstValue) -> Option<ConstValue> {
    match (l, op, r) {
        (ConstValue::List(mut a), ast::BinaryOp::Add, ConstValue::List(b)) => {
            a.extend(b);
            Some(ConstValue::List(a))
        }
        (ConstValue::List(a), ast::BinaryOp::Mul, ConstValue::Int(n))
        | (ConstValue::Int(n), ast::BinaryOp::Mul, ConstValue::List(a)) => {
            if n <= 0 {
                return Some(ConstValue::List(Vec::new()));
            }
            let count = (n as usize).min(10_000);
            let mut out = Vec::with_capacity(a.len().saturating_mul(count));
            for _ in 0..count {
                out.extend(a.iter().cloned());
            }
            Some(ConstValue::List(out))
        }
        (ConstValue::String(a), ast::BinaryOp::Mul, ConstValue::Int(n))
        | (ConstValue::Int(n), ast::BinaryOp::Mul, ConstValue::String(a)) => {
            if n <= 0 {
                return Some(ConstValue::String(String::new()));
            }
            let count = (n as usize).min(10_000);
            Some(ConstValue::String(a.repeat(count)))
        }
        (ConstValue::String(a), ast::BinaryOp::Add, b) => {
            Some(ConstValue::String(format!("{a}{}", b.to_display_string())))
        }
        (a, ast::BinaryOp::Add, ConstValue::String(b)) => {
            Some(ConstValue::String(format!("{}{b}", a.to_display_string())))
        }
        (ConstValue::Int(a), ast::BinaryOp::Add, ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.saturating_add(b)))
        }
        (ConstValue::Int(a), ast::BinaryOp::Sub, ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.saturating_sub(b)))
        }
        (ConstValue::Int(a), ast::BinaryOp::Mul, ConstValue::Int(b)) => {
            Some(ConstValue::Int(a.saturating_mul(b)))
        }
        (ConstValue::Int(a), ast::BinaryOp::FloorDiv, ConstValue::Int(b)) if b != 0 => {
            Some(ConstValue::Int(a.div_euclid(b)))
        }
        (ConstValue::Int(a), ast::BinaryOp::Mod, ConstValue::Int(b)) if b != 0 => {
            Some(ConstValue::Int(a.rem_euclid(b)))
        }
        (ConstValue::Int(a), ast::BinaryOp::Pow, ConstValue::Int(b)) if b >= 0 => {
            Some(ConstValue::Int(a.saturating_pow(b as u32)))
        }
        (ConstValue::Int(a), ast::BinaryOp::Div, ConstValue::Int(b)) if b != 0 => {
            Some(ConstValue::Float(a as f64 / b as f64))
        }
        (ConstValue::Int(a), ast::BinaryOp::BitAnd, ConstValue::Int(b)) => {
            Some(ConstValue::Int(a & b))
        }
        (ConstValue::Int(a), ast::BinaryOp::BitOr, ConstValue::Int(b)) => {
            Some(ConstValue::Int(a | b))
        }
        (ConstValue::Int(a), ast::BinaryOp::BitXor, ConstValue::Int(b)) => {
            Some(ConstValue::Int(a ^ b))
        }
        (ConstValue::Int(a), ast::BinaryOp::LShift, ConstValue::Int(b)) if (0..64).contains(&b) => {
            Some(ConstValue::Int(a << b))
        }
        (ConstValue::Int(a), ast::BinaryOp::RShift, ConstValue::Int(b)) if (0..64).contains(&b) => {
            Some(ConstValue::Int(a >> b))
        }
        (ConstValue::Int(a), ast::BinaryOp::Eq, ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a == b))
        }
        (ConstValue::Int(a), ast::BinaryOp::NotEq, ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a != b))
        }
        (ConstValue::Int(a), ast::BinaryOp::Lt, ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a < b))
        }
        (ConstValue::Int(a), ast::BinaryOp::LtEq, ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a <= b))
        }
        (ConstValue::Int(a), ast::BinaryOp::Gt, ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a > b))
        }
        (ConstValue::Int(a), ast::BinaryOp::GtEq, ConstValue::Int(b)) => {
            Some(ConstValue::Bool(a >= b))
        }
        (ConstValue::String(a), ast::BinaryOp::Eq, ConstValue::String(b)) => {
            Some(ConstValue::Bool(a == b))
        }
        (ConstValue::String(a), ast::BinaryOp::NotEq, ConstValue::String(b)) => {
            Some(ConstValue::Bool(a != b))
        }
        (ConstValue::Bool(a), ast::BinaryOp::Eq, ConstValue::Bool(b)) => {
            Some(ConstValue::Bool(a == b))
        }
        (ConstValue::Bool(a), ast::BinaryOp::NotEq, ConstValue::Bool(b)) => {
            Some(ConstValue::Bool(a != b))
        }
        (l_val, op, r_val) => {
            let a = l_val.as_f64()?;
            let b = r_val.as_f64()?;
            match op {
                ast::BinaryOp::Add => Some(ConstValue::Float(a + b)),
                ast::BinaryOp::Sub => Some(ConstValue::Float(a - b)),
                ast::BinaryOp::Mul => Some(ConstValue::Float(a * b)),
                ast::BinaryOp::Div if b != 0.0 => Some(ConstValue::Float(a / b)),
                ast::BinaryOp::FloorDiv if b != 0.0 => Some(ConstValue::Float((a / b).floor())),
                ast::BinaryOp::Mod if b != 0.0 => Some(ConstValue::Float(a.rem_euclid(b))),
                ast::BinaryOp::Pow => Some(ConstValue::Float(a.powf(b))),
                ast::BinaryOp::Eq => Some(ConstValue::Bool((a - b).abs() < f64::EPSILON)),
                ast::BinaryOp::NotEq => Some(ConstValue::Bool((a - b).abs() >= f64::EPSILON)),
                ast::BinaryOp::Lt => Some(ConstValue::Bool(a < b)),
                ast::BinaryOp::LtEq => Some(ConstValue::Bool(a <= b)),
                ast::BinaryOp::Gt => Some(ConstValue::Bool(a > b)),
                ast::BinaryOp::GtEq => Some(ConstValue::Bool(a >= b)),
                _ => None,
            }
        }
    }
}

pub fn eval_const_iterable(
    iter_expr: &ast::Expr,
    env: &FxHashMap<String, ConstValue>,
) -> Option<Vec<ConstValue>> {
    eval_const_iterable_with_arity(iter_expr, 1, env)
}

pub fn eval_const_iterable_with_arity(
    iter_expr: &ast::Expr,
    target_arity: usize,
    env: &FxHashMap<String, ConstValue>,
) -> Option<Vec<ConstValue>> {
    if let ast::ExprKind::Call {
        callee,
        args,
        kwargs,
    } = &iter_expr.kind
        && kwargs.is_empty()
        && let ast::ExprKind::Ident(fn_name) = &callee.kind
    {
        match fn_name.as_str() {
            "range" => {
                let (start, end, step) = match args.as_slice() {
                    [end] => (0, eval_constant_with_env(end, env)?.as_i64()?, 1),
                    [start, end] => (
                        eval_constant_with_env(start, env)?.as_i64()?,
                        eval_constant_with_env(end, env)?.as_i64()?,
                        1,
                    ),
                    [start, end, step] => (
                        eval_constant_with_env(start, env)?.as_i64()?,
                        eval_constant_with_env(end, env)?.as_i64()?,
                        eval_constant_with_env(step, env)?.as_i64()?,
                    ),
                    _ => return None,
                };
                if step == 0 {
                    return None;
                }
                let mut out = Vec::new();
                let mut cur = start;
                while (step > 0 && cur < end) || (step < 0 && cur > end) {
                    if out.len() >= 10_000 {
                        return None;
                    }
                    out.push(ConstValue::Int(cur));
                    cur = cur.saturating_add(step);
                }
                return Some(out);
            }
            "enumerate" if (1..=2).contains(&args.len()) => {
                let inner = eval_const_iterable(&args[0], env)?;
                let start = if args.len() == 2 {
                    eval_constant_with_env(&args[1], env)?.as_i64()?
                } else {
                    0
                };
                let mut out = Vec::with_capacity(inner.len());
                for (idx, item) in inner.into_iter().enumerate() {
                    out.push(ConstValue::List(vec![
                        ConstValue::Int(start.saturating_add(idx as i64)),
                        item,
                    ]));
                }
                return Some(out);
            }
            "reversed" if args.len() == 1 => {
                let mut inner = eval_const_iterable(&args[0], env)?;
                inner.reverse();
                return Some(inner);
            }
            _ => {}
        }
    }

    match eval_constant_with_env(iter_expr, env)? {
        ConstValue::List(items) => Some(items),
        ConstValue::Dict(pairs) => {
            if target_arity >= 2 {
                Some(
                    pairs
                        .into_iter()
                        .map(|(k, v)| ConstValue::List(vec![k, v]))
                        .collect(),
                )
            } else {
                Some(pairs.into_iter().map(|(k, _)| k).collect())
            }
        }
        ConstValue::String(s) => Some(
            s.chars()
                .map(|c| ConstValue::String(c.to_string()))
                .collect(),
        ),
        _ => None,
    }
}

pub fn bind_const_targets(
    targets: &[(String, Span)],
    val: ConstValue,
    env: &mut FxHashMap<String, ConstValue>,
) -> bool {
    if targets.len() == 1 {
        env.insert(targets[0].0.clone(), val);
        return true;
    }
    if let ConstValue::List(items) = val
        && items.len() == targets.len()
    {
        for ((name, _), item) in targets.iter().zip(items) {
            env.insert(name.clone(), item);
        }
        return true;
    }
    false
}

fn eval_const_comprehension(
    elt: &ast::Expr,
    generators: &[ast::Comprehension],
    env: &FxHashMap<String, ConstValue>,
    out: &mut Vec<ConstValue>,
) -> Option<()> {
    let Some((first, rest)) = generators.split_first() else {
        out.push(eval_constant_with_env(elt, env)?);
        return Some(());
    };
    let iter_items = eval_const_iterable_with_arity(&first.iter, first.targets.len(), env)?;
    let mut local_env = env.clone();
    for item in iter_items {
        if !bind_const_targets(&first.targets, item, &mut local_env) {
            return None;
        }
        let mut cond_ok = true;
        for cond in &first.ifs {
            if !eval_constant_with_env(cond, &local_env)?.is_truthy() {
                cond_ok = false;
                break;
            }
        }
        if cond_ok {
            eval_const_comprehension(elt, rest, &local_env, out)?;
        }
    }
    Some(())
}

fn extract_const_var_name(expr: &ast::Expr) -> Option<String> {
    match &expr.kind {
        ast::ExprKind::Ident(name) => Some(name.clone()),
        ast::ExprKind::Attribute { value, attr } if matches!(&value.kind, ast::ExprKind::Ident(obj) if obj == "self") => {
            Some(attr.clone())
        }
        _ => None,
    }
}

/// Evaluates a statement at compile time into `env`, supporting variable assignments,
/// list/dict mutations (`append`, `extend`, `insert`, `pop`, `remove`, `clear`, `reverse`, `+=`, `list[i] = v`, `dict[k] = v`),
/// `for` loops, `if`/`else` branches, bounded `while` loops, and class/function/module symbols.
/// Returns `true` if a `break` statement was encountered inside a loop.
pub fn eval_const_stmt(stmt: &ast::Stmt, env: &mut FxHashMap<String, ConstValue>) -> bool {
    match &stmt.kind {
        ast::StmtKind::Break => return true,
        ast::StmtKind::ClassDef(c) => {
            env.insert(c.name.clone(), ConstValue::Symbol(c.name.clone()));
        }
        ast::StmtKind::FunctionDef(f) => {
            env.insert(f.name.clone(), ConstValue::Symbol(f.name.clone()));
        }
        ast::StmtKind::ModuleDef { name, body, .. } => {
            let mut mod_env = FxHashMap::default();
            for s in body {
                eval_const_stmt(s, &mut mod_env);
            }
            for (k, v) in mod_env {
                env.insert(format!("{name}.{k}"), v);
            }
        }
        ast::StmtKind::ImportFrom {
            module, aliases, ..
        } => {
            if module != "qsys" && !module.starts_with("qsys.") {
                let mod_stem = module
                    .trim_start_matches('.')
                    .rsplit('.')
                    .next()
                    .unwrap_or(module.as_str());
                let prefix = format!("{mod_stem}.");
                for (orig_name, alias_opt) in aliases {
                    if orig_name == "*" {
                        let exported: Vec<(String, ConstValue)> = env
                            .iter()
                            .filter_map(|(k, v)| {
                                k.strip_prefix(&prefix)
                                    .map(|member| (member.to_string(), v.clone()))
                            })
                            .collect();
                        for (member, val) in exported {
                            env.insert(member, val);
                        }
                    } else {
                        let bound = alias_opt.clone().unwrap_or_else(|| orig_name.clone());
                        let key = format!("{mod_stem}.{orig_name}");
                        let val = env
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| ConstValue::Symbol(bound.clone()));
                        env.insert(bound, val);
                    }
                }
            }
        }
        ast::StmtKind::Assign { target, value, .. } => {
            if let Some(name) = extract_const_var_name(target) {
                if let Some(cval) = eval_constant_with_env(value, env) {
                    env.insert(name, cval);
                }
            } else if let ast::ExprKind::Tuple(targets) | ast::ExprKind::List(targets) =
                &target.kind
            {
                if let Some(ConstValue::List(items)) = eval_constant_with_env(value, env)
                    && items.len() == targets.len()
                {
                    for (t, item) in targets.iter().zip(items) {
                        if let Some(name) = extract_const_var_name(t) {
                            env.insert(name, item);
                        }
                    }
                }
            } else if let ast::ExprKind::Subscript {
                value: container_expr,
                index,
            } = &target.kind
                && let Some(var_name) = extract_const_var_name(container_expr)
                && let Some(idx_val) = eval_constant_with_env(index, env)
            {
                let rhs = eval_constant_with_env(value, env)
                    .unwrap_or_else(|| ConstValue::Symbol("<expr>".to_string()));
                match env.get_mut(&var_name) {
                    Some(ConstValue::List(items)) => {
                        if let Some(idx) = idx_val.as_i64() {
                            let resolved = if idx < 0 {
                                items.len() as i64 + idx
                            } else {
                                idx
                            };
                            if resolved >= 0 && (resolved as usize) < items.len() {
                                items[resolved as usize] = rhs;
                            }
                        }
                    }
                    Some(ConstValue::Dict(pairs)) => {
                        if let Some(existing) = pairs.iter_mut().find(|(k, _)| *k == idx_val) {
                            existing.1 = rhs;
                        } else {
                            pairs.push((idx_val, rhs));
                        }
                    }
                    _ => {}
                }
            }
        }
        ast::StmtKind::AugAssign { target, op, value } => {
            if let Some(var_name) = extract_const_var_name(target)
                && let Some(rhs) = eval_constant_with_env(value, env)
                && let Some(lhs) = env.get(&var_name).cloned()
                && let Some(res) = eval_const_binary_op(lhs, *op, rhs)
            {
                env.insert(var_name, res);
            }
        }
        ast::StmtKind::Expr(expr) => {
            if let ast::ExprKind::Call {
                callee,
                args,
                kwargs,
            } = &expr.kind
                && kwargs.is_empty()
                && let ast::ExprKind::Attribute { value: recv, attr } = &callee.kind
                && let Some(var_name) = extract_const_var_name(recv)
            {
                let mut eval_args = Vec::with_capacity(args.len());
                for a in args {
                    let Some(v) = eval_constant_with_env(a, env) else {
                        return false;
                    };
                    eval_args.push(v);
                }
                match env.get_mut(&var_name) {
                    Some(ConstValue::List(items)) => match attr.as_str() {
                        "append" if eval_args.len() == 1 => {
                            items.push(eval_args.into_iter().next().unwrap());
                        }
                        "extend" if eval_args.len() == 1 => {
                            if let ConstValue::List(more) = eval_args.into_iter().next().unwrap() {
                                items.extend(more);
                            }
                        }
                        "insert" if eval_args.len() == 2 => {
                            if let Some(idx) = eval_args[0].as_i64() {
                                let len = items.len() as i64;
                                let pos = if idx < 0 {
                                    (len + idx).max(0) as usize
                                } else {
                                    (idx as usize).min(items.len())
                                };
                                items.insert(pos, eval_args[1].clone());
                            }
                        }
                        "pop" => {
                            if eval_args.is_empty() {
                                items.pop();
                            } else if eval_args.len() == 1
                                && let Some(idx) = eval_args[0].as_i64()
                            {
                                let len = items.len() as i64;
                                let resolved = if idx < 0 { len + idx } else { idx };
                                if resolved >= 0 && (resolved as usize) < items.len() {
                                    items.remove(resolved as usize);
                                }
                            }
                        }
                        "remove" if eval_args.len() == 1 => {
                            if let Some(pos) = items.iter().position(|x| x == &eval_args[0]) {
                                items.remove(pos);
                            }
                        }
                        "clear" if eval_args.is_empty() => {
                            items.clear();
                        }
                        "reverse" if eval_args.is_empty() => {
                            items.reverse();
                        }
                        _ => {}
                    },
                    Some(ConstValue::Dict(pairs)) => match attr.as_str() {
                        "clear" if eval_args.is_empty() => {
                            pairs.clear();
                        }
                        "pop" if !eval_args.is_empty() => {
                            if let Some(pos) = pairs.iter().position(|(k, _)| k == &eval_args[0]) {
                                pairs.remove(pos);
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        ast::StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            if let Some(cond) = eval_constant_with_env(condition, env) {
                let branch = if cond.is_truthy() {
                    then_branch
                } else {
                    else_branch
                };
                for s in branch {
                    if eval_const_stmt(s, env) {
                        return true;
                    }
                }
            }
        }
        ast::StmtKind::For {
            target,
            iterable,
            body,
        } => {
            let targets: Vec<&str> = target
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let arity = targets.len().max(1);
            if let Some(items) = eval_const_iterable_with_arity(iterable, arity, env) {
                for item in items {
                    if targets.len() <= 1 {
                        env.insert(target.clone(), item);
                    } else if let ConstValue::List(parts) = item
                        && parts.len() == targets.len()
                    {
                        for (t_name, part) in targets.iter().zip(parts) {
                            env.insert((*t_name).to_string(), part);
                        }
                    } else {
                        continue;
                    }
                    let mut broke = false;
                    for s in body {
                        if eval_const_stmt(s, env) {
                            broke = true;
                            break;
                        }
                    }
                    if broke {
                        break;
                    }
                }
            }
        }
        ast::StmtKind::While { condition, body } => {
            let mut iters = 0usize;
            while iters < 1024 {
                let Some(cond) = eval_constant_with_env(condition, env) else {
                    break;
                };
                if !cond.is_truthy() {
                    break;
                }
                iters += 1;
                let mut broke = false;
                for s in body {
                    if eval_const_stmt(s, env) {
                        broke = true;
                        break;
                    }
                }
                if broke {
                    break;
                }
            }
        }
        _ => {}
    }
    false
}

pub fn eval_constant(expr: &ast::Expr) -> Option<ConstValue> {
    eval_constant_with_env(expr, &FxHashMap::default())
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredControl {
    pub id: ControlId,
    pub ident: String,
    pub def: ControlDef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredProperty {
    pub id: PropertyId,
    pub ident: String,
    pub def: PropertyDef,
}

#[derive(Debug, Clone, Default)]
pub struct ExtractedDeclarations {
    pub controls: Vec<DeclaredControl>,
    pub properties: Vec<DeclaredProperty>,
    pub plugin_info: PluginInfo,
    pub pages: Vec<PageLayout>,
    pub ui_control_resolutions: Vec<(Span, ControlId)>,
    pub control_property_resolutions: Vec<(Span, PropertyId)>,
    pub remaining_statements: Vec<ast::Stmt>,
}

pub fn extract_class_declarations(
    class: &ast::ClassDef,
    scope_stack: &mut ScopeStack,
    diagnostics: &mut Diagnostics,
) -> ExtractedDeclarations {
    extract_class_declarations_with_module(class, &[], scope_stack, diagnostics)
}

pub fn extract_class_declarations_with_module(
    class: &ast::ClassDef,
    module_statements: &[ast::Stmt],
    scope_stack: &mut ScopeStack,
    diagnostics: &mut Diagnostics,
) -> ExtractedDeclarations {
    let mut declarations = ExtractedDeclarations::default();
    let mut seen_controls = FxHashSet::default();
    let mut seen_properties = FxHashSet::default();
    let mut pending_controls: Vec<(String, ast::Expr, Span)> = Vec::new();
    let mut pending_ui_exprs: Vec<ast::Expr> = Vec::new();
    let mut constants: FxHashMap<String, ConstValue> = FxHashMap::default();
    let mut scope_types = scope_stack.collect_variable_types();

    for stmt in module_statements {
        match &stmt.kind {
            ast::StmtKind::Assign {
                target,
                type_annotation: Some(ann),
                ..
            } => {
                if let ast::ExprKind::Ident(name) = &target.kind {
                    scope_types.insert(name.clone(), crate::parse_simple_type_annotation(ann));
                }
            }
            ast::StmtKind::FunctionDef(f) => {
                if let Some(ret_ann) = &f.return_type {
                    scope_types
                        .insert(f.name.clone(), crate::parse_simple_type_annotation(ret_ann));
                }
            }
            ast::StmtKind::ClassDef(cls) => {
                scope_types.insert(
                    cls.name.clone(),
                    crate::types::Type::UserClass {
                        name: cls.name.clone(),
                    },
                );
            }
            _ => {}
        }
        eval_const_stmt(stmt, &mut constants);
    }

    declarations.plugin_info.name = class.name.clone();

    for stmt in &class.body {
        match &stmt.kind {
            ast::StmtKind::Assign {
                target,
                value,
                type_annotation,
            } => {
                let var_name = match &target.kind {
                    ast::ExprKind::Ident(name) => name.clone(),
                    _ => {
                        eval_const_stmt(stmt, &mut constants);
                        declarations.remaining_statements.push(stmt.clone());
                        continue;
                    }
                };
                if let Some(ann) = type_annotation {
                    scope_types.insert(var_name.clone(), crate::parse_simple_type_annotation(ann));
                }

                if var_name == "layout" || var_name == "pages" || is_ui_layout_expr(value) {
                    pending_ui_exprs.push(value.clone());
                    continue;
                }

                if is_control_decl_expr(value) {
                    pending_controls.push((var_name, value.clone(), stmt.span));
                    continue;
                }

                if let Some(property_def) = try_parse_property(
                    &class.name,
                    &var_name,
                    value,
                    stmt.span,
                    &constants,
                    &scope_types,
                    diagnostics,
                ) {
                    if !seen_properties.insert(var_name.clone()) {
                        diagnostics.report(
                            DiagnosticKind::syntax_error(format!(
                                "Property '{var_name}' is declared multiple times"
                            )),
                            stmt.span,
                        );
                    } else {
                        let default_cval = match &property_def.kind {
                            PropertyKind::Integer { default, .. } => ConstValue::Int(*default),
                            PropertyKind::Double { default, .. } => ConstValue::Float(*default),
                            PropertyKind::Boolean { default } => ConstValue::Bool(*default),
                            PropertyKind::String { default }
                            | PropertyKind::Enum { default, .. } => {
                                ConstValue::String(default.clone())
                            }
                        };
                        constants.entry(var_name.clone()).or_insert(default_cval);
                        let id = scope_stack.alloc_property_id();
                        declarations.properties.push(DeclaredProperty {
                            id,
                            ident: var_name.clone(),
                            def: property_def,
                        });
                    }
                    continue;
                }

                if try_parse_metadata(&var_name, value, &constants, &mut declarations.plugin_info) {
                    continue;
                }

                eval_const_stmt(stmt, &mut constants);

                declarations.remaining_statements.push(stmt.clone());
            }

            ast::StmtKind::FunctionDef(f) => {
                if let Some(ret_ann) = &f.return_type {
                    scope_types
                        .insert(f.name.clone(), crate::parse_simple_type_annotation(ret_ann));
                }
                if f.name == "get_control_layout"
                    && let Some(ui_expr) = extract_declarative_layout_from_method(&f.body)
                {
                    pending_ui_exprs.push(ui_expr);
                } else {
                    declarations.remaining_statements.push(stmt.clone());
                }
            }

            _ => {
                eval_const_stmt(stmt, &mut constants);
                declarations.remaining_statements.push(stmt.clone());
            }
        }
    }

    let mut ui_env = UiResolveEnv::from_class_and_module(
        &class.body,
        module_statements,
        &declarations.properties,
    );
    for (k, v) in scope_types {
        ui_env.scope_types.entry(k).or_insert(v);
    }

    // Parse Control.* declarations now that all Property.* declarations and constants/methods are available
    for (var_name, value, span) in pending_controls {
        if let Some(control_def) = try_parse_control(
            &class.name,
            &var_name,
            &value,
            span,
            &ui_env,
            &mut declarations.control_property_resolutions,
            diagnostics,
        ) {
            if !seen_controls.insert(var_name.clone()) {
                diagnostics.report(
                    DiagnosticKind::DuplicateControlDeclaration {
                        name: var_name.clone(),
                    },
                    span,
                );
            } else {
                let id = scope_stack.alloc_control_id();
                declarations.controls.push(DeclaredControl {
                    id,
                    ident: var_name,
                    def: control_def,
                });
            }
        }
    }

    // Second pass: resolve declarative UI.* trees now that all controls are registered
    for ui_expr in &pending_ui_exprs {
        let solved_pages = parse_ui_layout_expr_with_env(
            &class.name,
            ui_expr,
            &declarations.controls,
            &ui_env,
            diagnostics,
            &mut declarations.ui_control_resolutions,
        );
        declarations.pages.extend(solved_pages);
    }

    // Also validate and collect any inline UI.* calls inside hybrid `get_control_layout` methods
    for stmt in &declarations.remaining_statements {
        if let ast::StmtKind::FunctionDef(f) = &stmt.kind
            && f.name == "get_control_layout"
        {
            let mut local_ui_vars = FxHashMap::default();
            let mut method_ui_env = ui_env.clone();
            let inline_pages = collect_inline_ui_pages_from_stmts(
                &class.name,
                &f.body,
                &declarations.controls,
                &mut method_ui_env,
                diagnostics,
                &mut declarations.ui_control_resolutions,
                &mut local_ui_vars,
            );
            if declarations.pages.is_empty() {
                declarations.pages.extend(inline_pages);
            }
        }
    }

    declarations
}

fn is_control_decl_expr(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ast::ExprKind::Call { callee, .. } => match &callee.kind {
            ast::ExprKind::Attribute { value, .. } | ast::ExprKind::Subscript { value, .. } => {
                matches!(&value.kind, ast::ExprKind::Ident(obj) if obj == "Control")
            }
            _ => false,
        },
        _ => false,
    }
}

fn resolve_type_property_arg(
    val: &ast::Expr,
    control_name: &str,
    properties: &[DeclaredProperty],
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<(String, Vec<String>, String)> {
    let mut prop_ident_opt: Option<&str> = None;
    let mut prop_key_opt: Option<String> = None;

    let mut target = val;
    if let ast::ExprKind::Attribute { value, attr } = &target.kind
        && attr == "Value"
    {
        target = value;
    }

    match &target.kind {
        ast::ExprKind::Ident(id) => {
            prop_ident_opt = Some(id.as_str());
        }
        ast::ExprKind::Attribute { value, attr } if matches!(&value.kind, ast::ExprKind::Ident(s) if s == "self") =>
        {
            prop_ident_opt = Some(attr.as_str());
        }
        ast::ExprKind::Subscript { value, index } if matches!(&value.kind, ast::ExprKind::Ident(s) if s == "props" || s == "Properties") => {
            if let Some(s) = eval_constant(index).and_then(|c| c.as_str().map(String::from)) {
                prop_key_opt = Some(s);
            }
        }
        _ => {}
    }

    let default_choices = || {
        vec![
            "Integer".to_string(),
            "Float".to_string(),
            "String".to_string(),
            "Boolean".to_string(),
        ]
    };

    if let Some(prop_ident) = prop_ident_opt
        && let Some(prop) = properties
            .iter()
            .find(|p| p.ident == prop_ident || p.def.name == prop_ident)
    {
        prop_resolutions.push((val.span, prop.id));
        match &prop.def.kind {
            qsys_model::PropertyKind::Enum { choices, default } => {
                return Some((prop.def.name.clone(), choices.clone(), default.clone()));
            }
            qsys_model::PropertyKind::String { default } => {
                let def_choice = if default.is_empty() {
                    "String".to_string()
                } else {
                    default.clone()
                };
                return Some((prop.def.name.clone(), default_choices(), def_choice));
            }
            _ => {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.to_string(),
                        message: format!(
                            "Property '{}' used for generic Control[Type] must be a Property.Enum or Property.String",
                            prop.ident
                        ),
                    },
                    val.span,
                );
                return None;
            }
        }
    }

    if let Some(prop_key) = prop_key_opt {
        if let Some(prop) = properties
            .iter()
            .find(|p| p.def.name == prop_key || p.ident == prop_key)
        {
            prop_resolutions.push((val.span, prop.id));
            match &prop.def.kind {
                qsys_model::PropertyKind::Enum { choices, default } => {
                    return Some((prop.def.name.clone(), choices.clone(), default.clone()));
                }
                qsys_model::PropertyKind::String { default } => {
                    let def_choice = if default.is_empty() {
                        "String".to_string()
                    } else {
                        default.clone()
                    };
                    return Some((prop.def.name.clone(), default_choices(), def_choice));
                }
                _ => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: control_name.to_string(),
                            message: format!(
                                "Property '{}' used for generic Control[Type] must be a Property.Enum or Property.String",
                                prop.def.name
                            ),
                        },
                        val.span,
                    );
                    return None;
                }
            }
        }
        return Some((prop_key, default_choices(), "String".to_string()));
    }

    diagnostics.report(
        DiagnosticKind::InvalidDeclarationArgument {
            name: control_name.to_string(),
            message:
                "Generic Control[Type] requires a declared Property.Enum (e.g. Control[Type](...))"
                    .to_string(),
        },
        val.span,
    );
    None
}

fn eval_count_expr(
    class_name: &str,
    expr: &ast::Expr,
    control_name: &str,
    ui_env: &UiResolveEnv<'_>,
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
    reported_error: &mut bool,
) -> Option<(i64, String, bool)> {
    let properties = ui_env.properties;
    match &expr.kind {
        ast::ExprKind::Int(i) => return Some((*i, i.to_string(), false)),
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Pos,
            operand,
        } => {
            return eval_count_expr(
                class_name,
                operand,
                control_name,
                ui_env,
                prop_resolutions,
                diagnostics,
                reported_error,
            );
        }
        ast::ExprKind::Unary {
            op: ast::UnaryOp::Neg,
            operand,
        } => {
            let (v, s, has_p) = eval_count_expr(
                class_name,
                operand,
                control_name,
                ui_env,
                prop_resolutions,
                diagnostics,
                reported_error,
            )?;
            return Some((v.saturating_neg(), format!("(-{s})"), has_p));
        }
        ast::ExprKind::Binary { left, op, right }
            if matches!(
                op,
                ast::BinaryOp::Add
                    | ast::BinaryOp::Sub
                    | ast::BinaryOp::Mul
                    | ast::BinaryOp::FloorDiv
                    | ast::BinaryOp::Div
                    | ast::BinaryOp::Mod
                    | ast::BinaryOp::Pow
            ) =>
        {
            let (lv, ls, lp) = eval_count_expr(
                class_name,
                left,
                control_name,
                ui_env,
                prop_resolutions,
                diagnostics,
                reported_error,
            )?;
            let (rv, rs, rp) = eval_count_expr(
                class_name,
                right,
                control_name,
                ui_env,
                prop_resolutions,
                diagnostics,
                reported_error,
            )?;
            let has_p = lp || rp;
            return match op {
                ast::BinaryOp::Add => {
                    Some((lv.saturating_add(rv), format!("({ls} + {rs})"), has_p))
                }
                ast::BinaryOp::Sub => {
                    Some((lv.saturating_sub(rv), format!("({ls} - {rs})"), has_p))
                }
                ast::BinaryOp::Mul => Some((lv.saturating_mul(rv), format!("{ls} * {rs}"), has_p)),
                ast::BinaryOp::FloorDiv | ast::BinaryOp::Div => {
                    let v = if rv != 0 { lv / rv } else { 1 };
                    Some((v, format!("({ls} // {rs})"), has_p))
                }
                ast::BinaryOp::Mod => {
                    let v = if rv != 0 { lv % rv } else { 0 };
                    Some((v, format!("({ls} % {rs})"), has_p))
                }
                ast::BinaryOp::Pow => {
                    let v = lv.saturating_pow(rv.max(0) as u32);
                    Some((v, format!("({ls} ^ {rs})"), has_p))
                }
                _ => None,
            };
        }
        _ => {}
    }

    let mut prop_ident_opt: Option<&str> = None;
    let mut prop_key_opt: Option<String> = None;

    let mut target = expr;
    if let ast::ExprKind::Attribute { value, attr } = &target.kind
        && attr == "Value"
    {
        target = value;
    }

    match &target.kind {
        ast::ExprKind::Ident(id) => {
            prop_ident_opt = Some(id.as_str());
        }
        ast::ExprKind::Attribute { value, attr }
            if matches!(
                &value.kind,
                ast::ExprKind::Ident(s) if s == "self" || s == class_name
            ) =>
        {
            prop_ident_opt = Some(attr.as_str());
        }
        ast::ExprKind::Subscript { value, index }
            if matches!(
                &value.kind,
                ast::ExprKind::Ident(s) if s == "props" || s == "Properties"
            ) =>
        {
            if let Some(s) = eval_constant_with_env(index, &ui_env.constants)
                .and_then(|c| c.as_str().map(String::from))
            {
                prop_key_opt = Some(s);
            }
        }
        _ => {}
    }

    if let Some(prop_ident) = prop_ident_opt
        && let Some(prop) = properties
            .iter()
            .find(|p| p.ident == prop_ident || p.def.name == prop_ident)
    {
        prop_resolutions.push((expr.span, prop.id));
        if let qsys_model::PropertyKind::Integer { default, .. } = &prop.def.kind {
            return Some((
                (*default).max(1),
                format!("{{PROP:{}}}", prop.def.name),
                true,
            ));
        }
        *reported_error = true;
        diagnostics.report(
            DiagnosticKind::InvalidDeclarationArgument {
                name: control_name.to_string(),
                message: format!(
                    "Property '{}' used for 'count' must be a Property.Integer",
                    prop.ident
                ),
            },
            expr.span,
        );
        return None;
    }

    if let Some(prop_key) = prop_key_opt {
        if let Some(prop) = properties
            .iter()
            .find(|p| p.def.name == prop_key || p.ident == prop_key)
        {
            prop_resolutions.push((expr.span, prop.id));
            if let qsys_model::PropertyKind::Integer { default, .. } = &prop.def.kind {
                return Some((
                    (*default).max(1),
                    format!("{{PROP:{}}}", prop.def.name),
                    true,
                ));
            }
            *reported_error = true;
            diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: control_name.to_string(),
                    message: format!(
                        "Property '{}' used for 'count' must be a Property.Integer",
                        prop.def.name
                    ),
                },
                expr.span,
            );
            return None;
        }
        return Some((1, format!("{{PROP:{prop_key}}}"), true));
    }

    if !ui_env.is_dynamic_design_expr(class_name, expr) {
        if let Some(i) = eval_constant_with_env(expr, &ui_env.constants).and_then(|c| c.as_i64()) {
            return Some((i, i.to_string(), false));
        }
    } else if let Some(pv) = ui_env.eval_design_expr_preview(class_name, expr)
        && let Some(i) = pv.as_i64()
    {
        ui_env.collect_property_resolutions(class_name, expr, prop_resolutions);
        let tmpl = ui_env.lower_design_expr_to_prop_template(class_name, expr);
        let spec = if tmpl.contains("{PROP:") {
            tmpl
        } else {
            format!("{{LUA:{tmpl}}}")
        };
        return Some((i, spec, true));
    }

    if matches!(
        ui_env.infer_expr_type(class_name, expr),
        Some(crate::types::Type::Int)
    ) {
        ui_env.collect_property_resolutions(class_name, expr, prop_resolutions);
        let tmpl = ui_env.lower_design_expr_to_prop_template(class_name, expr);
        let spec = if tmpl.contains("{PROP:") {
            tmpl
        } else {
            format!("{{LUA:{tmpl}}}")
        };
        return Some((1, spec, true));
    }

    None
}

pub(crate) fn combine_2d_count_specs(
    r_val: usize,
    r_prop: &Option<String>,
    c_val: usize,
    c_prop: &Option<String>,
) -> Option<String> {
    let to_token = |val: usize, prop: &Option<String>| -> String {
        if let Some(p) = prop {
            if let Some(inner) = p.strip_prefix("{LUA:").and_then(|s| s.strip_suffix('}')) {
                format!("({inner})")
            } else if p.contains("{PROP:") {
                if p.contains(' ') {
                    format!("({p})")
                } else {
                    p.clone()
                }
            } else {
                format!("{{PROP:{p}}}")
            }
        } else {
            val.to_string()
        }
    };

    match (r_prop.is_some(), c_prop.is_some()) {
        (false, false) => None,
        (true, false) if c_val == 1 => r_prop.clone(),
        (false, true) if r_val == 1 => c_prop.clone(),
        _ => {
            let combined = format!("{} * {}", to_token(r_val, r_prop), to_token(c_val, c_prop));
            if combined.contains("{PROP:") {
                Some(combined)
            } else {
                Some(format!("{{LUA:{combined}}}"))
            }
        }
    }
}

fn expr_ident_hint(expr: &ast::Expr) -> String {
    match &expr.kind {
        ast::ExprKind::Ident(id) => id.clone(),
        ast::ExprKind::Attribute { value, attr } => {
            if attr == "Value" {
                expr_ident_hint(value)
            } else {
                attr.clone()
            }
        }
        _ => String::new(),
    }
}

fn should_swap_mul_dims(
    left: &ast::Expr,
    l_prop: &Option<String>,
    right: &ast::Expr,
    r_prop: &Option<String>,
) -> bool {
    let l_str = format!(
        "{} {}",
        expr_ident_hint(left),
        l_prop.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();
    let r_str = format!(
        "{} {}",
        expr_ident_hint(right),
        r_prop.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();

    let is_col_like = |s: &str| s.contains("per") || s.contains("col");
    let is_row_like = |s: &str| {
        !is_col_like(s)
            && (s.contains("row")
                || s.contains("track")
                || s.contains("group")
                || s.contains("bank")
                || s.contains("bus")
                || s.contains("line"))
    };

    (is_col_like(&l_str) && !is_col_like(&r_str)) || (is_row_like(&r_str) && !is_row_like(&l_str))
}

pub(crate) fn resolve_count_arg(
    class_name: &str,
    val: &ast::Expr,
    control_name: &str,
    ui_env: &UiResolveEnv<'_>,
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<(usize, Option<String>)> {
    let mut reported_error = false;
    if let Some((evaluated_val, template, has_prop)) = eval_count_expr(
        class_name,
        val,
        control_name,
        ui_env,
        prop_resolutions,
        diagnostics,
        &mut reported_error,
    ) {
        if !has_prop {
            if evaluated_val > 0 {
                return Some((evaluated_val as usize, None));
            }
            diagnostics.report(
                DiagnosticKind::InvalidDeclarationArgument {
                    name: control_name.to_string(),
                    message: "'count' must be a positive integer or a Property.Integer".to_string(),
                },
                val.span,
            );
            return None;
        }

        let preview_count = evaluated_val.clamp(1, 128) as usize;
        let normalized_spec = if let Some(inner) = template
            .strip_prefix("{PROP:")
            .and_then(|s| s.strip_suffix('}'))
            && !inner.contains("{PROP:")
        {
            inner.to_string()
        } else {
            template
        };
        return Some((preview_count, Some(normalized_spec)));
    }

    if !reported_error {
        diagnostics.report(
            DiagnosticKind::InvalidDeclarationArgument {
                name: control_name.to_string(),
                message: "'count' must be a positive integer or a Property.Integer".to_string(),
            },
            val.span,
        );
    }
    None
}

fn resolve_design_num_arg(
    class_name: &str,
    expr: &ast::Expr,
    control_name: &str,
    arg_label: &str,
    ui_env: &UiResolveEnv<'_>,
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<(f64, Option<String>)> {
    ui_env.collect_property_resolutions(class_name, expr, prop_resolutions);
    if !ui_env.is_dynamic_design_expr(class_name, expr) {
        if let Some(m) = eval_constant_with_env(expr, &ui_env.constants).and_then(|c| c.as_f64()) {
            return Some((m, None));
        }
    } else if let Some(pv) = ui_env.eval_design_expr_preview(class_name, expr)
        && let Some(m) = pv.as_f64()
    {
        let lua = ui_env.lower_design_expr_to_lua(class_name, expr);
        return Some((m, Some(lua)));
    }
    if matches!(
        ui_env.infer_expr_type(class_name, expr),
        Some(crate::types::Type::Int | crate::types::Type::Float)
    ) {
        let lua = ui_env.lower_design_expr_to_lua(class_name, expr);
        return Some((0.0, Some(lua)));
    }
    diagnostics.report(
        DiagnosticKind::InvalidDeclarationArgument {
            name: control_name.to_string(),
            message: format!("'{arg_label}' must be a numeric literal"),
        },
        expr.span,
    );
    None
}

fn resolve_design_str_arg(
    class_name: &str,
    expr: &ast::Expr,
    ui_env: &UiResolveEnv<'_>,
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
) -> Option<(String, Option<String>)> {
    ui_env.collect_property_resolutions(class_name, expr, prop_resolutions);
    if !ui_env.is_dynamic_design_expr(class_name, expr) {
        if let Some(s) = eval_constant_with_env(expr, &ui_env.constants)
            .and_then(|c| c.as_str().map(String::from))
        {
            return Some((s, None));
        }
    } else if let Some(pv) = ui_env.eval_design_expr_preview(class_name, expr) {
        let s = pv.to_display_string();
        let lua = ui_env.lower_design_expr_to_lua(class_name, expr);
        return Some((s, Some(lua)));
    }
    if matches!(
        ui_env.infer_expr_type(class_name, expr),
        Some(crate::types::Type::String)
    ) {
        let lua = ui_env.lower_design_expr_to_lua(class_name, expr);
        return Some((String::new(), Some(lua)));
    }
    None
}

fn resolve_design_bool_arg(
    class_name: &str,
    expr: &ast::Expr,
    ui_env: &UiResolveEnv<'_>,
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
) -> Option<(bool, Option<String>)> {
    ui_env.collect_property_resolutions(class_name, expr, prop_resolutions);
    if !ui_env.is_dynamic_design_expr(class_name, expr) {
        if let Some(b) = eval_constant_with_env(expr, &ui_env.constants).and_then(|c| c.as_bool()) {
            return Some((b, None));
        }
    } else if let Some(pv) = ui_env.eval_design_expr_preview(class_name, expr) {
        let b = pv.is_truthy();
        let lua = ui_env.lower_design_expr_to_lua(class_name, expr);
        return Some((b, Some(lua)));
    }
    if matches!(
        ui_env.infer_expr_type(class_name, expr),
        Some(crate::types::Type::Bool)
    ) {
        let lua = ui_env.lower_design_expr_to_lua(class_name, expr);
        return Some((false, Some(lua)));
    }
    None
}

fn try_parse_control(
    class_name: &str,
    var_name: &str,
    expr: &ast::Expr,
    span: Span,
    ui_env: &UiResolveEnv<'_>,
    prop_resolutions: &mut Vec<(Span, PropertyId)>,
    diagnostics: &mut Diagnostics,
) -> Option<ControlDef> {
    let properties = ui_env.properties;
    let (kind_str, subscript_type_expr, args, kwargs) = match &expr.kind {
        ast::ExprKind::Call {
            callee,
            args,
            kwargs,
        } => match &callee.kind {
            ast::ExprKind::Attribute { value, attr } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Control" => {
                    (attr.as_str(), None, args, kwargs)
                }
                _ => return None,
            },
            ast::ExprKind::Subscript { value, index } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Control" => {
                    ("Generic", Some(index.as_ref()), args, kwargs)
                }
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };

    let is_generic_method =
        matches!(kind_str, "Generic" | "Dynamic") && subscript_type_expr.is_none();
    let has_kwarg_type = kwargs.iter().any(|(k, _)| k == "type" || k == "property");
    let positional_name_arg = if is_generic_method && !has_kwarg_type {
        args.get(1)
    } else {
        args.first()
    };

    // 1. Check keyword: name="..."
    // 2. Check positional: Control.<Type>("...")
    // 3. Fall back to built-in Q-SYS standard names ("IPAddress", "MACAddress")
    // 4. Fall back to variable identifier name (e.g. `Gain = ...` -> "Gain")
    let control_name = match kwargs.iter().find(|(k, _)| k == "name") {
        Some((_, val)) => match eval_constant_with_env(val, &ui_env.constants)
            .and_then(|c| c.as_str().map(String::from))
        {
            Some(s) => s,
            None => {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: var_name.to_string(),
                        message: "Control 'name' argument must be a string".to_string(),
                    },
                    val.span,
                );
                return None;
            }
        },
        None => match positional_name_arg {
            Some(val) => match eval_constant_with_env(val, &ui_env.constants)
                .and_then(|c| c.as_str().map(String::from))
            {
                Some(s) => s,
                None => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: var_name.to_string(),
                            message: "Control positional name must be a string".to_string(),
                        },
                        val.span,
                    );
                    return None;
                }
            },
            None => match kind_str {
                "IPAddress" | "IpAddress" => "IPAddress".to_string(),
                "MACAddress" | "MacAddress" => "MACAddress".to_string(),
                _ => var_name.to_string(),
            },
        },
    };

    let mut rows: Option<usize> = None;
    let mut cols: Option<usize> = None;
    let mut rows_property: Option<String> = None;
    let mut cols_property: Option<String> = None;
    let mut explicit_2d = false;

    let count_kwarg = kwargs.iter().find(|(k, _)| k == "count");
    let (mut count, mut count_property) = match count_kwarg {
        Some((_, val)) => match &val.kind {
            ast::ExprKind::Tuple(items) | ast::ExprKind::List(items) if items.len() == 2 => {
                let (r_val, r_prop) = resolve_count_arg(
                    class_name,
                    &items[0],
                    &control_name,
                    ui_env,
                    prop_resolutions,
                    diagnostics,
                )?;
                let (c_val, c_prop) = resolve_count_arg(
                    class_name,
                    &items[1],
                    &control_name,
                    ui_env,
                    prop_resolutions,
                    diagnostics,
                )?;
                let total = r_val.saturating_mul(c_val).clamp(1, 256);
                let total_prop = combine_2d_count_specs(r_val, &r_prop, c_val, &c_prop);
                rows = Some(r_val);
                cols = Some(c_val);
                rows_property = r_prop;
                cols_property = c_prop;
                explicit_2d = true;
                (total, total_prop)
            }
            _ => {
                let (total, total_prop) = resolve_count_arg(
                    class_name,
                    val,
                    &control_name,
                    ui_env,
                    prop_resolutions,
                    diagnostics,
                )?;
                if let ast::ExprKind::Binary {
                    left,
                    op: ast::BinaryOp::Mul,
                    right,
                } = &val.kind
                {
                    let mut dummy_res = Vec::new();
                    let mut dummy_diags = Diagnostics::new();
                    if let (Some((l_val, l_prop)), Some((r_val, r_prop))) = (
                        resolve_count_arg(
                            class_name,
                            left,
                            &control_name,
                            ui_env,
                            &mut dummy_res,
                            &mut dummy_diags,
                        ),
                        resolve_count_arg(
                            class_name,
                            right,
                            &control_name,
                            ui_env,
                            &mut dummy_res,
                            &mut dummy_diags,
                        ),
                    ) {
                        if should_swap_mul_dims(left, &l_prop, right, &r_prop) {
                            rows = Some(r_val);
                            rows_property = r_prop;
                            cols = Some(l_val);
                            cols_property = l_prop;
                        } else {
                            rows = Some(l_val);
                            rows_property = l_prop;
                            cols = Some(r_val);
                            cols_property = r_prop;
                        }
                    }
                }
                (total, total_prop)
            }
        },
        None => (1, None),
    };

    if let Some((_, r_expr)) = kwargs.iter().find(|(k, _)| k == "rows") {
        let (r_val, r_prop) = resolve_count_arg(
            class_name,
            r_expr,
            &control_name,
            ui_env,
            prop_resolutions,
            diagnostics,
        )?;
        rows = Some(r_val);
        rows_property = r_prop;
        explicit_2d = true;
    }
    if let Some((_, c_expr)) = kwargs.iter().find(|(k, _)| k == "cols" || k == "columns") {
        let (c_val, c_prop) = resolve_count_arg(
            class_name,
            c_expr,
            &control_name,
            ui_env,
            prop_resolutions,
            diagnostics,
        )?;
        cols = Some(c_val);
        cols_property = c_prop;
        explicit_2d = true;
    }
    if explicit_2d && count_kwarg.is_none() {
        let r_val = rows.unwrap_or(1);
        let c_val = cols.unwrap_or(1);
        rows = Some(r_val);
        cols = Some(c_val);
        count = r_val.saturating_mul(c_val).clamp(1, 256);
        count_property = combine_2d_count_specs(r_val, &rows_property, c_val, &cols_property);
    }

    let (pin_style, pin_style_lua_expr) = match kwargs.iter().find(|(k, _)| k == "pins") {
        Some((_, val)) => {
            let Some((s, lua_opt)) =
                resolve_design_str_arg(class_name, val, ui_env, prop_resolutions)
            else {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name.clone(),
                        message:
                            "Invalid 'pins' value. Expected 'None', 'Input', 'Output', or 'Both'"
                                .to_string(),
                    },
                    val.span,
                );
                return None;
            };
            let parsed = match s.as_str() {
                "None" => Some(PinStyle::None),
                "Input" => Some(PinStyle::Input),
                "Output" => Some(PinStyle::Output),
                "Both" => Some(PinStyle::Both),
                _ => None,
            };
            match (parsed, lua_opt) {
                (Some(ps), lua) => (ps, lua),
                (None, Some(lua)) => (PinStyle::Both, Some(lua)),
                (None, None) => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: control_name.clone(),
                            message: "Invalid 'pins' value. Expected 'None', 'Input', 'Output', or 'Both'".to_string(),
                        },
                        val.span,
                    );
                    return None;
                }
            }
        }
        None => (PinStyle::Both, None),
    };

    let (user_pin, user_pin_lua_expr) = match kwargs.iter().find(|(k, _)| k == "user_pin") {
        Some((_, val)) => {
            match resolve_design_bool_arg(class_name, val, ui_env, prop_resolutions) {
                Some(pair) => pair,
                None => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: control_name.clone(),
                            message: "'user_pin' must be a boolean literal".to_string(),
                        },
                        val.span,
                    );
                    return None;
                }
            }
        }
        None => (true, None),
    };

    let parse_unit = |prop_resolutions: &mut Vec<(Span, PropertyId)>,
                      diagnostics: &mut Diagnostics|
     -> Option<(ControlUnit, Option<String>)> {
        match kwargs.iter().find(|(k, _)| k == "unit") {
            Some((_, val)) => {
                let Some((s, lua_opt)) =
                    resolve_design_str_arg(class_name, val, ui_env, prop_resolutions)
                else {
                    diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "Invalid 'unit'. Expected 'Float', 'Integer', 'dB', 'Hz', 'Percent', 'Seconds', or 'Pan'".to_string(),
                            },
                            val.span,
                        );
                    return None;
                };
                let parsed = match s.as_str() {
                    "Float" => Some(ControlUnit::Float),
                    "Integer" => Some(ControlUnit::Integer),
                    "dB" | "Decibels" => Some(ControlUnit::Decibels),
                    "Hz" | "Hertz" => Some(ControlUnit::Hertz),
                    "Percent" => Some(ControlUnit::Percent),
                    "Seconds" => Some(ControlUnit::Seconds),
                    "Pan" => Some(ControlUnit::Pan),
                    _ => None,
                };
                match (parsed, lua_opt) {
                    (Some(u), lua) => Some((u, lua)),
                    (None, Some(lua)) => Some((ControlUnit::Float, Some(lua))),
                    (None, None) => {
                        diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Invalid 'unit'. Expected 'Float', 'Integer', 'dB', 'Hz', 'Percent', 'Seconds', or 'Pan'".to_string(),
                                },
                                val.span,
                            );
                        None
                    }
                }
            }
            None => Some((ControlUnit::Float, None)),
        }
    };

    let mut min_lua_expr: Option<String> = None;
    let mut max_lua_expr: Option<String> = None;
    let mut default_lua_expr: Option<String> = None;
    let mut unit_lua_expr: Option<String> = None;
    let mut button_type_lua_expr: Option<String> = None;
    let mut text_type_lua_expr: Option<String> = None;

    let kind = match kind_str {
        "Float" | "Knob" | "Fader" => {
            let (unit, u_lua) = parse_unit(prop_resolutions, diagnostics)?;
            unit_lua_expr = u_lua;

            let (min, m_lua) = match kwargs.iter().find(|(k, _)| k == "min") {
                Some((_, val)) => resolve_design_num_arg(
                    class_name,
                    val,
                    &control_name,
                    "min",
                    ui_env,
                    prop_resolutions,
                    diagnostics,
                )?,
                None => (0.0, None),
            };
            min_lua_expr = m_lua;

            let (mut max, mx_lua) = match kwargs.iter().find(|(k, _)| k == "max") {
                Some((_, val)) => resolve_design_num_arg(
                    class_name,
                    val,
                    &control_name,
                    "max",
                    ui_env,
                    prop_resolutions,
                    diagnostics,
                )?,
                None => (1.0, None),
            };
            max_lua_expr = mx_lua;

            let (mut default, def_lua) = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => resolve_design_num_arg(
                    class_name,
                    val,
                    &control_name,
                    "default",
                    ui_env,
                    prop_resolutions,
                    diagnostics,
                )?,
                None => (min, min_lua_expr.clone()),
            };
            default_lua_expr = def_lua;

            let has_dynamic_range = min_lua_expr.is_some() || max_lua_expr.is_some();
            if !has_dynamic_range {
                if min > max {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: control_name.clone(),
                            message: format!("'min' ({min}) cannot be greater than 'max' ({max})"),
                        },
                        span,
                    );
                    return None;
                }

                if default_lua_expr.is_none() && (default < min || default > max) {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: control_name.clone(),
                            message: format!(
                                "'default' ({default}) is outside range [{min}, {max}]"
                            ),
                        },
                        span,
                    );
                    return None;
                }
            } else {
                if max < min {
                    max = min;
                }
                default = default.clamp(min, max);
            }

            if kind_str == "Fader" {
                ControlKind::Fader {
                    unit,
                    min,
                    max,
                    default,
                }
            } else {
                ControlKind::Knob {
                    unit,
                    min,
                    max,
                    default,
                }
            }
        }

        "Boolean" | "Button" => {
            let default_btn_type = if kind_str == "Boolean" {
                ButtonType::Toggle
            } else {
                ButtonType::Momentary
            };
            let btn_type = match kwargs.iter().find(|(k, _)| k == "type") {
                Some((_, val)) => {
                    let Some((s, lua_opt)) =
                        resolve_design_str_arg(class_name, val, ui_env, prop_resolutions)
                    else {
                        diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "Invalid button 'type'. Expected 'Momentary', 'Toggle', or 'Trigger'".to_string(),
                            },
                            val.span,
                        );
                        return None;
                    };
                    let parsed = match s.as_str() {
                        "Momentary" => Some(ButtonType::Momentary),
                        "Trigger" => Some(ButtonType::Trigger),
                        "Toggle" => Some(ButtonType::Toggle),
                        _ => None,
                    };
                    match (parsed, lua_opt) {
                        (Some(bt), lua) => {
                            button_type_lua_expr = lua;
                            bt
                        }
                        (None, Some(lua)) => {
                            button_type_lua_expr = Some(lua);
                            default_btn_type
                        }
                        (None, None) => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Invalid button 'type'. Expected 'Momentary', 'Toggle', or 'Trigger'".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => default_btn_type,
            };
            ControlKind::Button {
                button_type: btn_type,
            }
        }

        "Text" => {
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => {
                    match resolve_design_str_arg(class_name, val, ui_env, prop_resolutions) {
                        Some((s, lua_opt)) => {
                            default_lua_expr = lua_opt;
                            s
                        }
                        None => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Text 'default' must be a string".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => String::new(),
            };
            let text_type = match kwargs.iter().find(|(k, _)| k == "type") {
                Some((_, val)) => {
                    let Some((s, lua_opt)) =
                        resolve_design_str_arg(class_name, val, ui_env, prop_resolutions)
                    else {
                        diagnostics.report(
                            DiagnosticKind::InvalidDeclarationArgument {
                                name: control_name.clone(),
                                message: "Invalid text 'type'. Expected 'Text' or 'ReadOnly'"
                                    .to_string(),
                            },
                            val.span,
                        );
                        return None;
                    };
                    let parsed = match s.as_str() {
                        "Text" => Some(TextType::Text),
                        "ReadOnly" => Some(TextType::ReadOnly),
                        _ => None,
                    };
                    match (parsed, lua_opt) {
                        (Some(tt), lua) => {
                            text_type_lua_expr = lua;
                            tt
                        }
                        (None, Some(lua)) => {
                            text_type_lua_expr = Some(lua);
                            TextType::Text
                        }
                        (None, None) => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: control_name.clone(),
                                    message: "Invalid text 'type'. Expected 'Text' or 'ReadOnly'"
                                        .to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => TextType::Text,
            };
            ControlKind::Text { text_type, default }
        }

        "Indicator" => {
            let (unit, u_lua) = parse_unit(prop_resolutions, diagnostics)?;
            unit_lua_expr = u_lua;
            ControlKind::Indicator { unit }
        }

        // Built-in Q-SYS Reserved Control: IPAddress
        "IPAddress" | "IpAddress" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| {
                    let (s, lua_opt) =
                        resolve_design_str_arg(class_name, v, ui_env, prop_resolutions)?;
                    default_lua_expr = lua_opt;
                    Some(s)
                })
                .unwrap_or_else(|| "255.255.255.255".to_string());

            ControlKind::Text {
                text_type: TextType::Text,
                default,
            }
        }

        // Built-in Q-SYS Reserved Control: MACAddress
        "MACAddress" | "MacAddress" => {
            let default = kwargs
                .iter()
                .find(|(k, _)| k == "default")
                .and_then(|(_, v)| {
                    let (s, lua_opt) =
                        resolve_design_str_arg(class_name, v, ui_env, prop_resolutions)?;
                    default_lua_expr = lua_opt;
                    Some(s)
                })
                .unwrap_or_else(|| "00:00:00:00:00:00".to_string());

            ControlKind::Text {
                text_type: TextType::Text,
                default,
            }
        }

        // Property-typed generic control: `Control[Type](...)`, `Control.[Type](...)`, or `Control.Generic(Type, ...)`
        "Generic" | "Dynamic" => {
            let type_expr = subscript_type_expr
                .or_else(|| {
                    kwargs
                        .iter()
                        .find(|(k, _)| k == "type" || k == "property")
                        .map(|(_, v)| v)
                })
                .or_else(|| args.first());

            let Some(type_expr) = type_expr else {
                diagnostics.report(
                    DiagnosticKind::InvalidDeclarationArgument {
                        name: control_name,
                        message: "Control.Generic requires a Property reference (e.g. Control[Type]() or Control.Generic(Type))".to_string(),
                    },
                    span,
                );
                return None;
            };

            let (prop_name, choices, default_choice) = resolve_type_property_arg(
                type_expr,
                &control_name,
                properties,
                prop_resolutions,
                diagnostics,
            )?;

            let (unit, u_lua) = parse_unit(prop_resolutions, diagnostics)?;
            unit_lua_expr = u_lua;

            let min = match kwargs.iter().find(|(k, _)| k == "min") {
                Some((_, val)) => {
                    let (m, m_lua) = resolve_design_num_arg(
                        class_name,
                        val,
                        &control_name,
                        "min",
                        ui_env,
                        prop_resolutions,
                        diagnostics,
                    )?;
                    min_lua_expr = m_lua;
                    m
                }
                None => 0.0,
            };
            let max = match kwargs.iter().find(|(k, _)| k == "max") {
                Some((_, val)) => {
                    let (m, mx_lua) = resolve_design_num_arg(
                        class_name,
                        val,
                        &control_name,
                        "max",
                        ui_env,
                        prop_resolutions,
                        diagnostics,
                    )?;
                    max_lua_expr = mx_lua;
                    m
                }
                None => 100.0,
            };

            let mut has_explicit_default = false;
            let (default_num, default_str) = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => {
                    has_explicit_default = true;
                    ui_env.collect_property_resolutions(class_name, val, prop_resolutions);
                    if !ui_env.is_dynamic_design_expr(class_name, val) {
                        match eval_constant_with_env(val, &ui_env.constants) {
                            Some(ConstValue::Float(f)) => (f, f.to_string()),
                            Some(ConstValue::Int(i)) => (i as f64, i.to_string()),
                            Some(ConstValue::String(s)) => {
                                let num = s.parse::<f64>().unwrap_or(min);
                                (num, s)
                            }
                            Some(ConstValue::Bool(b)) => (if b { 1.0 } else { 0.0 }, b.to_string()),
                            _ => (min, String::new()),
                        }
                    } else {
                        default_lua_expr = Some(ui_env.lower_design_expr_to_lua(class_name, val));
                        if let Some(pv) = ui_env.eval_design_expr_preview(class_name, val) {
                            let s = pv.to_display_string();
                            let num = pv.as_f64().or_else(|| s.parse::<f64>().ok()).unwrap_or(min);
                            (num, s)
                        } else {
                            (min, String::new())
                        }
                    }
                }
                None => (min, String::new()),
            };

            let gspec = GenericControlSpec {
                property_name: prop_name,
                choices,
                default_choice: default_choice.clone(),
                unit,
                min,
                max,
                default_num,
                default_str,
                has_explicit_default,
                button_type: ButtonType::Toggle,
                text_type: TextType::Text,
            };
            let resolved_kind = gspec.resolve_choice_kind(&default_choice);

            let mut def = ControlDef::new(control_name, resolved_kind)
                .with_pin_style(pin_style)
                .with_user_pin(user_pin)
                .with_count_property(count_property)
                .with_grid_dimensions(rows, cols, rows_property, cols_property, explicit_2d)
                .with_generic_spec(Some(gspec));
            def.min_lua_expr = min_lua_expr;
            def.max_lua_expr = max_lua_expr;
            def.default_lua_expr = default_lua_expr;
            def.unit_lua_expr = unit_lua_expr;
            def.pin_style_lua_expr = pin_style_lua_expr;
            def.user_pin_lua_expr = user_pin_lua_expr;
            def.button_type_lua_expr = button_type_lua_expr;
            def.text_type_lua_expr = text_type_lua_expr;
            if count > 1 {
                def = def.with_count(count);
            }
            return Some(def);
        }

        _ => {
            diagnostics.report(
                DiagnosticKind::syntax_error(format!("Unknown Control type 'Control.{kind_str}'")),
                span,
            );
            return None;
        }
    };

    let mut def = ControlDef::new(control_name, kind)
        .with_pin_style(pin_style)
        .with_user_pin(user_pin)
        .with_count_property(count_property)
        .with_grid_dimensions(rows, cols, rows_property, cols_property, explicit_2d);
    def.min_lua_expr = min_lua_expr;
    def.max_lua_expr = max_lua_expr;
    def.default_lua_expr = default_lua_expr;
    def.unit_lua_expr = unit_lua_expr;
    def.pin_style_lua_expr = pin_style_lua_expr;
    def.user_pin_lua_expr = user_pin_lua_expr;
    def.button_type_lua_expr = button_type_lua_expr;
    def.text_type_lua_expr = text_type_lua_expr;
    if count > 1 {
        def = def.with_count(count);
    }
    Some(def)
}

fn lower_prop_expr_to_lua(
    class_name: &str,
    expr: &ast::Expr,
    prop_env: &UiResolveEnv<'_>,
) -> String {
    if let ast::ExprKind::Ident(name) = &expr.kind {
        return name.clone();
    }
    let mut env_no_empty_list = prop_env.clone();
    env_no_empty_list
        .constants
        .retain(|_, v| !matches!(v, ConstValue::List(items) if items.is_empty()));
    env_no_empty_list.lower_design_expr_to_lua(class_name, expr)
}

fn try_parse_property(
    class_name: &str,
    var_name: &str,
    expr: &ast::Expr,
    span: Span,
    constants: &FxHashMap<String, ConstValue>,
    scope_types: &FxHashMap<String, crate::types::Type>,
    diagnostics: &mut Diagnostics,
) -> Option<PropertyDef> {
    let (kind_str, args, kwargs) = match &expr.kind {
        ast::ExprKind::Call {
            callee,
            args,
            kwargs,
        } => match &callee.kind {
            ast::ExprKind::Attribute { value, attr } => match &value.kind {
                ast::ExprKind::Ident(obj) if obj == "Property" => (attr.as_str(), args, kwargs),
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };

    let mut prop_env = UiResolveEnv::empty();
    prop_env.constants = constants.clone();
    prop_env.scope_types = scope_types.clone();

    let prop_name = kwargs
        .iter()
        .find(|(k, _)| k == "name")
        .and_then(|(_, v)| eval_constant_with_env(v, constants))
        .and_then(|c| c.as_str().map(String::from))
        .or_else(|| {
            args.first()
                .and_then(|e| eval_constant_with_env(e, constants))
                .and_then(|c| c.as_str().map(String::from))
        })
        .unwrap_or_else(|| var_name.to_string());

    let description = kwargs
        .iter()
        .find(|(k, _)| k == "description")
        .and_then(|(_, v)| eval_constant_with_env(v, constants))
        .and_then(|c| c.as_str().map(String::from));

    let prop = match kind_str {
        "Integer" => {
            let mut default_lua_expr = None;
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, v)) => {
                    if let Some(i) = eval_constant_with_env(v, constants).and_then(|c| c.as_i64()) {
                        i
                    } else if matches!(
                        prop_env.infer_expr_type(class_name, v),
                        Some(crate::types::Type::Int)
                    ) {
                        default_lua_expr = Some(lower_prop_expr_to_lua(class_name, v, &prop_env));
                        0
                    } else {
                        0
                    }
                }
                None => 0,
            };
            let min = kwargs
                .iter()
                .find(|(k, _)| k == "min")
                .and_then(|(_, v)| eval_constant_with_env(v, constants))
                .and_then(|c| c.as_i64());
            let max = kwargs
                .iter()
                .find(|(k, _)| k == "max")
                .and_then(|(_, v)| eval_constant_with_env(v, constants))
                .and_then(|c| c.as_i64());
            PropertyDef::integer(prop_name, default, min, max)
                .with_default_lua_expr(default_lua_expr)
        }

        "Double" | "Float" => {
            let mut default_lua_expr = None;
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, v)) => {
                    if let Some(f) = eval_constant_with_env(v, constants).and_then(|c| c.as_f64()) {
                        f
                    } else if matches!(
                        prop_env.infer_expr_type(class_name, v),
                        Some(crate::types::Type::Int | crate::types::Type::Float)
                    ) {
                        default_lua_expr = Some(lower_prop_expr_to_lua(class_name, v, &prop_env));
                        0.0
                    } else {
                        0.0
                    }
                }
                None => 0.0,
            };
            let min = kwargs
                .iter()
                .find(|(k, _)| k == "min")
                .and_then(|(_, v)| eval_constant_with_env(v, constants))
                .and_then(|c| c.as_f64());
            let max = kwargs
                .iter()
                .find(|(k, _)| k == "max")
                .and_then(|(_, v)| eval_constant_with_env(v, constants))
                .and_then(|c| c.as_f64());
            PropertyDef::double(prop_name, default, min, max)
                .with_default_lua_expr(default_lua_expr)
        }

        "String" => {
            let mut default_lua_expr = None;
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, v)) => {
                    if let Some(s) = eval_constant_with_env(v, constants)
                        .and_then(|c| c.as_str().map(String::from))
                    {
                        s
                    } else if matches!(
                        prop_env.infer_expr_type(class_name, v),
                        Some(crate::types::Type::String)
                    ) {
                        default_lua_expr = Some(lower_prop_expr_to_lua(class_name, v, &prop_env));
                        String::new()
                    } else {
                        String::new()
                    }
                }
                None => String::new(),
            };
            PropertyDef::string(prop_name, default).with_default_lua_expr(default_lua_expr)
        }

        "Boolean" => {
            let mut default_lua_expr = None;
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, v)) => {
                    if let Some(b) = eval_constant_with_env(v, constants).and_then(|c| c.as_bool())
                    {
                        b
                    } else if matches!(
                        prop_env.infer_expr_type(class_name, v),
                        Some(crate::types::Type::Bool)
                    ) {
                        default_lua_expr = Some(lower_prop_expr_to_lua(class_name, v, &prop_env));
                        false
                    } else {
                        false
                    }
                }
                None => false,
            };
            PropertyDef::boolean(prop_name, default).with_default_lua_expr(default_lua_expr)
        }

        "Enum" => {
            let mut choices_lua_expr: Option<String> = None;
            let choices = match kwargs.iter().find(|(k, _)| k == "choices") {
                Some((_, val)) => {
                    let is_annotated_str_list = matches!(
                        prop_env.infer_expr_type(class_name, val),
                        Some(crate::types::Type::List(ref inner))
                            if matches!(**inner, crate::types::Type::String | crate::types::Type::Any)
                    );
                    match eval_constant_with_env(val, constants) {
                        Some(ConstValue::List(items)) => {
                            let mut strings = Vec::new();
                            for item in items {
                                if let Some(s) = item.as_str() {
                                    strings.push(s.to_string());
                                } else {
                                    diagnostics.report(
                                        DiagnosticKind::InvalidDeclarationArgument {
                                            name: prop_name.clone(),
                                            message:
                                                "All choices in 'choices' must be string literals"
                                                    .to_string(),
                                        },
                                        val.span,
                                    );
                                    return None;
                                }
                            }
                            if strings.is_empty() {
                                if is_annotated_str_list
                                    && !matches!(val.kind, ast::ExprKind::List(_))
                                {
                                    choices_lua_expr =
                                        Some(lower_prop_expr_to_lua(class_name, val, &prop_env));
                                    vec![String::new()]
                                } else {
                                    diagnostics.report(
                                        DiagnosticKind::InvalidDeclarationArgument {
                                            name: prop_name.clone(),
                                            message: "'choices' list cannot be empty".to_string(),
                                        },
                                        val.span,
                                    );
                                    return None;
                                }
                            } else {
                                strings
                            }
                        }
                        _ if is_annotated_str_list => {
                            choices_lua_expr =
                                Some(lower_prop_expr_to_lua(class_name, val, &prop_env));
                            vec![String::new()]
                        }
                        _ => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: prop_name.clone(),
                                    message: "'choices' must be a list of strings".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => {
                    diagnostics.report(
                        DiagnosticKind::InvalidDeclarationArgument {
                            name: prop_name.clone(),
                            message: "Property.Enum requires 'choices'".to_string(),
                        },
                        span,
                    );
                    return None;
                }
            };

            let mut default_lua_expr: Option<String> = None;
            let default = match kwargs.iter().find(|(k, _)| k == "default") {
                Some((_, val)) => {
                    match eval_constant_with_env(val, constants)
                        .and_then(|c| c.as_str().map(String::from))
                    {
                        Some(d) => {
                            if choices_lua_expr.is_some() {
                                d
                            } else if d.is_empty() && !choices.contains(&d) {
                                choices.first().cloned().unwrap_or_default()
                            } else if !choices.contains(&d) {
                                diagnostics.report(
                                    DiagnosticKind::InvalidDeclarationArgument {
                                        name: prop_name.clone(),
                                        message: format!(
                                            "'default' choice ('{d}') must be one of {choices:?}"
                                        ),
                                    },
                                    val.span,
                                );
                                return None;
                            } else {
                                d
                            }
                        }
                        None if matches!(
                            prop_env.infer_expr_type(class_name, val),
                            Some(crate::types::Type::String)
                        ) =>
                        {
                            default_lua_expr =
                                Some(lower_prop_expr_to_lua(class_name, val, &prop_env));
                            choices.first().cloned().unwrap_or_default()
                        }
                        None => {
                            diagnostics.report(
                                DiagnosticKind::InvalidDeclarationArgument {
                                    name: prop_name.clone(),
                                    message: "'default' choice must be a string".to_string(),
                                },
                                val.span,
                            );
                            return None;
                        }
                    }
                }
                None => choices.first().cloned().unwrap_or_default(),
            };

            PropertyDef::enumeration(prop_name, choices, default)
                .with_choices_lua_expr(choices_lua_expr)
                .with_default_lua_expr(default_lua_expr)
        }

        _ => {
            diagnostics.report(
                DiagnosticKind::syntax_error(format!(
                    "Unknown Property type 'Property.{kind_str}'"
                )),
                span,
            );
            return None;
        }
    };

    Some(match description {
        Some(d) => prop.with_description(d),
        None => prop,
    })
}

fn try_parse_metadata(
    var_name: &str,
    value: &ast::Expr,
    constants: &FxHashMap<String, ConstValue>,
    info: &mut PluginInfo,
) -> bool {
    let const_val = eval_constant_with_env(value, constants);
    match var_name {
        "name" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.name = s;
                return true;
            }
        }
        "version" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.version = s;
                return true;
            }
        }
        "author" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.author = s;
                return true;
            }
        }
        "id" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.id = s;
                return true;
            }
        }
        "description" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.description = Some(s);
                return true;
            }
        }
        "manufacturer" => {
            if let Some(s) = const_val.and_then(|c| c.as_str().map(String::from)) {
                info.manufacturer = Some(s);
                return true;
            }
        }
        "show_debug" => {
            if let Some(b) = const_val.and_then(|c| c.as_bool()) {
                info.show_debug = b;
                return true;
            }
        }
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_parser::parse_module;

    fn parse_class_body(src: &str) -> (ExtractedDeclarations, Diagnostics) {
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).expect("Should parse");
        let cls = match &parsed.module.statements[0].kind {
            ast::StmtKind::ClassDef(c) => c,
            _ => panic!("Expected ClassDef"),
        };
        let mut diags = Diagnostics::new();
        let mut scope_stack = ScopeStack::new();
        let decls = extract_class_declarations(cls, &mut scope_stack, &mut diags);
        (decls, diags)
    }

    #[test]
    fn test_extract_controls_with_negative_ranges() {
        let src = r#"
class Mixer(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)
    Mute = Control.Boolean()
"#;
        let (decls, diags) = parse_class_body(src);
        assert!(!diags.has_errors());
        assert_eq!(decls.controls.len(), 2);

        let gain = &decls.controls[0];
        assert_eq!(gain.ident, "Gain");
        assert_eq!(gain.def.name, "MasterGain");
        match &gain.def.kind {
            ControlKind::Knob {
                min, max, default, ..
            } => {
                assert_eq!(*min, -100.0);
                assert_eq!(*max, 20.0);
                assert_eq!(*default, 0.0);
            }
            _ => panic!("Expected Knob/Float"),
        }

        let mute = &decls.controls[1];
        assert_eq!(mute.ident, "Mute");
        assert_eq!(mute.def.name, "Mute");
        assert!(matches!(mute.def.kind, ControlKind::Button { .. }));
    }
}
