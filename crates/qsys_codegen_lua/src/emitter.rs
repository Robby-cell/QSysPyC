use std::collections::HashSet;

use qsys_ir::*;
use qsys_model::{ControlDef, ControlKind, ControlMember};

use crate::context::CodegenContext;

pub struct LuaEmitter<'a> {
    pub buffer: String,
    pub indent_level: usize,
    pub ctx: &'a mut CodegenContext,
}

impl<'a> LuaEmitter<'a> {
    pub fn new(ctx: &'a mut CodegenContext) -> Self {
        Self {
            buffer: String::with_capacity(1024),
            indent_level: 0,
            ctx,
        }
    }

    pub fn indent(&mut self) {
        self.indent_level += 1;
    }

    pub fn dedent(&mut self) {
        self.indent_level = self.indent_level.saturating_sub(1);
    }

    pub fn write_indent(&mut self) {
        for _ in 0..self.indent_level {
            self.buffer.push_str("    ");
        }
    }

    pub fn write_line(&mut self, line: &str) {
        self.write_indent();
        self.buffer.push_str(line);
        self.buffer.push('\n');
    }

    pub fn emit_statements(&mut self, stmts: &[IRStmt]) {
        for stmt in stmts {
            if let IRStmt::FunctionDef(f) = stmt {
                self.ctx.functions.insert(f.id, f.name.clone());
            }
        }
        for stmt in stmts {
            self.emit_stmt(stmt);
        }
    }

    /// Emits a block of top-level or module-level statements with declaration hoisting:
    /// 1. Bundled module definitions (`IRStmt::ModuleDef`) and imports (`IRStmt::ImportBinding`)
    /// 2. Pure constant assignments (`CONST = 42`)
    /// 3. Forward `local` declarations for mutually-referenced functions/variables
    /// 4. Custom class definitions (`IRStmt::ClassDef`), topologically sorted by base class
    /// 5. Function definitions (`IRStmt::FunctionDef`)
    /// 6. Remaining executable statements in source order
    pub fn emit_hoisted_statements(&mut self, stmts: &[IRStmt]) {
        for stmt in stmts {
            if let IRStmt::FunctionDef(f) = stmt {
                self.ctx.functions.insert(f.id, f.name.clone());
            }
        }

        // 1. Emit bundled modules and import bindings first
        for stmt in stmts {
            if matches!(
                stmt,
                IRStmt::ModuleDef { .. } | IRStmt::ImportBinding { .. }
            ) {
                self.emit_stmt(stmt);
            }
        }

        // 2. Emit pure constant assignments (`CONST = 42`) before classes and functions
        let mut emitted_const_indices = HashSet::new();
        for (idx, stmt) in stmts.iter().enumerate() {
            if let IRStmt::Assign { target, value, .. } = stmt {
                let is_local_target = match target {
                    IRExpr::Local(_) => true,
                    IRExpr::Tuple(items) => {
                        !items.is_empty() && items.iter().all(|i| matches!(i, IRExpr::Local(_)))
                    }
                    _ => false,
                };
                if is_local_target && is_pure_const_expr(value) {
                    self.emit_stmt(stmt);
                    emitted_const_indices.insert(idx);
                }
            }
        }

        let classes: Vec<&IRClass> = stmts
            .iter()
            .filter_map(|s| match s {
                IRStmt::ClassDef(c) => Some(c),
                _ => None,
            })
            .collect();
        let sorted_classes = sort_classes_topologically(&classes);

        let functions: Vec<&IRFunction> = stmts
            .iter()
            .filter_map(|s| match s {
                IRStmt::FunctionDef(f) => Some(f),
                _ => None,
            })
            .collect();

        // 3. Determine which functions or non-constant locals need a forward `local` declaration
        //    (e.g. a function called from inside a class method or from an earlier function).
        let mut forward_declared_fns = HashSet::new();
        for (fn_idx, f) in functions.iter().enumerate() {
            let referenced_in_class = sorted_classes
                .iter()
                .any(|c| class_references_function(c, f.id, &f.name));
            let referenced_in_earlier_fn = functions[..fn_idx]
                .iter()
                .any(|earlier| stmts_reference_function(&earlier.body, f.id, &f.name));
            if referenced_in_class || referenced_in_earlier_fn {
                self.write_line(&format!("local {}", f.name));
                forward_declared_fns.insert(f.id);
            }
        }

        // Also forward-declare any non-constant local variable assigned later in `stmts`
        // if it is referenced inside a class or function body.
        for (idx, stmt) in stmts.iter().enumerate() {
            if emitted_const_indices.contains(&idx) {
                continue;
            }
            if let IRStmt::Assign { target, .. } = stmt {
                let mut sym_ids = Vec::new();
                match target {
                    IRExpr::Local(sym_id) => sym_ids.push(*sym_id),
                    IRExpr::Tuple(items) => {
                        for item in items {
                            if let IRExpr::Local(sym_id) = item {
                                sym_ids.push(*sym_id);
                            }
                        }
                    }
                    _ => {}
                }
                for sym_id in sym_ids {
                    if !self.ctx.declared_locals.contains(&sym_id) {
                        let referenced_in_decl = sorted_classes
                            .iter()
                            .any(|c| class_references_local(c, sym_id))
                            || functions
                                .iter()
                                .any(|f| stmts_reference_local(&f.body, sym_id));
                        if referenced_in_decl {
                            let var_name = self.ctx.get_symbol_name(sym_id);
                            self.write_line(&format!("local {var_name}"));
                            self.ctx.declared_locals.insert(sym_id);
                        }
                    }
                }
            }
        }

        // 4. Emit all classes (base classes before subclasses)
        for cls in &sorted_classes {
            self.emit_custom_class(cls);
        }

        // 5. Emit all functions
        for f in &functions {
            let is_forward = forward_declared_fns.contains(&f.id);
            self.emit_function_def(f, is_forward);
        }

        // 6. Emit remaining executable statements in source order
        for (idx, stmt) in stmts.iter().enumerate() {
            if emitted_const_indices.contains(&idx) {
                continue;
            }
            if matches!(
                stmt,
                IRStmt::ModuleDef { .. }
                    | IRStmt::ImportBinding { .. }
                    | IRStmt::ClassDef(_)
                    | IRStmt::FunctionDef(_)
            ) {
                continue;
            }
            self.emit_stmt(stmt);
        }
    }

    pub fn emit_stmt(&mut self, stmt: &IRStmt) {
        match stmt {
            IRStmt::Assign { target, value, .. } => {
                if let IRExpr::Local(sym_id) = target {
                    if let Some(ctl_id) = self.resolve_control_id_for_expr(value) {
                        self.ctx.local_controls.insert(*sym_id, ctl_id);
                    }
                    let var_name = self.ctx.get_symbol_name(*sym_id);
                    let val_str = self.emit_expr(value);
                    self.write_indent();
                    if self.ctx.declared_locals.insert(*sym_id) {
                        self.buffer
                            .push_str(&format!("local {var_name} = {val_str}\n"));
                    } else {
                        self.buffer.push_str(&format!("{var_name} = {val_str}\n"));
                    }
                } else if let IRExpr::Tuple(targets) = target {
                    let val_str = if let IRExpr::Tuple(vals) = value {
                        vals.iter()
                            .map(|v| self.emit_expr(v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        self.emit_expr(value)
                    };
                    let all_new_locals = !targets.is_empty()
                        && targets.iter().all(
                            |t| matches!(t, IRExpr::Local(id) if !self.ctx.declared_locals.contains(id)),
                        );
                    if all_new_locals {
                        let names: Vec<String> = targets
                            .iter()
                            .filter_map(|t| {
                                if let IRExpr::Local(id) = t {
                                    self.ctx.declared_locals.insert(*id);
                                    Some(self.ctx.get_symbol_name(*id))
                                } else {
                                    None
                                }
                            })
                            .collect();
                        self.write_line(&format!("local {} = {}", names.join(", "), val_str));
                    } else {
                        for t in targets {
                            if let IRExpr::Local(id) = t
                                && self.ctx.declared_locals.insert(*id)
                            {
                                let var_name = self.ctx.get_symbol_name(*id);
                                self.write_line(&format!("local {var_name}"));
                            }
                        }
                        let lhs = targets
                            .iter()
                            .map(|t| self.emit_expr(t))
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.write_line(&format!("{lhs} = {val_str}"));
                    }
                } else if let IRExpr::Attribute { object, attr } = target
                    && ControlMember::from_ident(attr).is_some_and(|m| m.is_generic_accessor())
                {
                    if let Some(def) = self.resolve_control_def_for_expr(object) {
                        let obj_str = self.emit_expr(object);
                        let val_str = self.emit_expr(value);
                        if let Some(ref gspec) = def.generic_spec {
                            self.ctx.needs_generic_data_helper = true;
                            let prop_str = format!(
                                "Properties[{}].Value",
                                escape_lua_string(&gspec.property_name)
                            );
                            self.write_line(&format!(
                                "__qsys_set_data({obj_str}, {val_str}, {prop_str})"
                            ));
                        } else {
                            let member = match def.kind {
                                ControlKind::Text { .. } => "String",
                                ControlKind::Button { .. } | ControlKind::Indicator { .. } => {
                                    "Boolean"
                                }
                                ControlKind::Knob { .. } | ControlKind::Fader { .. } => "Value",
                            };
                            self.write_line(&format!("{obj_str}.{member} = {val_str}"));
                        }
                    } else {
                        self.ctx.needs_generic_data_helper = true;
                        let obj_str = self.emit_expr(object);
                        let val_str = self.emit_expr(value);
                        let prop_arg = self.default_generic_prop_expr();
                        self.write_line(&format!(
                            "__qsys_set_data({obj_str}, {val_str}, {prop_arg})"
                        ));
                    }
                } else {
                    let target_str = self.emit_expr(target);
                    let val_str = self.emit_expr(value);
                    self.write_line(&format!("{target_str} = {val_str}"));
                }
            }

            IRStmt::If {
                condition,
                truthiness,
                then_branch,
                else_branch,
                ..
            } => {
                let cond_str = self.emit_truthy_condition(condition, *truthiness);
                self.write_line(&format!("if {cond_str} then"));
                self.indent();
                self.emit_statements(then_branch);
                self.dedent();

                self.emit_else_branch(else_branch);
                self.write_line("end");
            }

            IRStmt::While {
                condition,
                truthiness,
                body,
                ..
            } => {
                let cond_str = self.emit_truthy_condition(condition, *truthiness);
                self.write_line(&format!("while {cond_str} do"));
                self.indent();
                self.emit_statements(body);
                self.dedent();
                self.write_line("end");
            }

            IRStmt::For {
                target_id,
                target_name,
                iterable,
                body,
                ..
            } => {
                self.ctx
                    .symbol_names
                    .insert(*target_id, target_name.clone());
                self.ctx.declared_locals.insert(*target_id);
                let var_name = self.ctx.get_symbol_name(*target_id);

                if let IRExpr::Call {
                    callee,
                    args,
                    kwargs,
                } = iterable
                    && kwargs.is_empty()
                    && matches!(callee.as_ref(), IRExpr::Global(g) if g == "range")
                {
                    match args.as_slice() {
                        [stop] => {
                            let stop_str = self.emit_expr(stop);
                            self.write_line(&format!("for {var_name} = 0, ({stop_str}) - 1 do"));
                        }
                        [start, stop] => {
                            let start_str = self.emit_expr(start);
                            let stop_str = self.emit_expr(stop);
                            self.write_line(&format!(
                                "for {var_name} = {start_str}, ({stop_str}) - 1 do"
                            ));
                        }
                        [start, stop, step] => {
                            let start_str = self.emit_expr(start);
                            let stop_str = self.emit_expr(stop);
                            let step_str = self.emit_expr(step);
                            self.write_line(&format!(
                                "for {var_name} = {start_str}, ({stop_str}) - 1, {step_str} do"
                            ));
                        }
                        _ => {
                            let iter_str = self.emit_expr(iterable);
                            self.write_line(&format!("for _, {var_name} in ipairs({iter_str}) do"));
                        }
                    }
                } else if let IRExpr::Control(ctl_id) = iterable {
                    self.ctx.local_controls.insert(*target_id, *ctl_id);
                    if let Some(def) = self.ctx.controls.get(ctl_id)
                        && let Some(ref prop_name) = def.count_property
                    {
                        let ctl_str = format!("Controls[{}]", escape_lua_string(&def.name));
                        let prop_str = format_count_lua_expr(prop_name, "Properties");
                        self.write_line(&format!(
                            "for _, {var_name} in ipairs(({prop_str} == 1) and {{ {ctl_str} }} or {ctl_str}) do"
                        ));
                    } else {
                        let iter_str = self.emit_expr(iterable);
                        self.write_line(&format!("for _, {var_name} in ipairs({iter_str}) do"));
                    }
                } else {
                    let iter_str = self.emit_expr(iterable);
                    self.write_line(&format!("for _, {var_name} in ipairs({iter_str}) do"));
                }

                self.indent();
                self.emit_statements(body);
                self.dedent();
                self.write_line("end");
            }

            IRStmt::Return { value, .. } => {
                if let Some(val) = value {
                    if let IRExpr::Tuple(items) = val {
                        let rendered: Vec<_> =
                            items.iter().map(|item| self.emit_expr(item)).collect();
                        self.write_line(&format!("return {}", rendered.join(", ")));
                    } else {
                        let val_str = self.emit_expr(val);
                        self.write_line(&format!("return {val_str}"));
                    }
                } else {
                    self.write_line("return");
                }
            }

            IRStmt::Break { .. } => {
                self.write_line("break");
            }

            IRStmt::Expr { expr, .. } => {
                // Python docstrings and standalone string comments:
                // Convert to Lua comments (-- ...) instead of bare strings ("...")
                if let IRExpr::String(doc) = expr {
                    let lines: Vec<&str> = doc.lines().collect();

                    // Trim blank leading/trailing lines common in triple-quoted docstrings
                    let mut start = 0;
                    while start < lines.len() && lines[start].trim().is_empty() {
                        start += 1;
                    }
                    let mut end = lines.len();
                    while end > start && lines[end - 1].trim().is_empty() {
                        end -= 1;
                    }

                    if start >= end {
                        self.write_line("--");
                    } else {
                        for line in &lines[start..end] {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                self.write_line("--");
                            } else {
                                self.write_line(&format!("-- {trimmed}"));
                            }
                        }
                    }
                } else {
                    let expr_str = self.emit_expr(expr);
                    self.write_line(&expr_str);
                }
            }

            IRStmt::RawLua { code, .. } => {
                for line in code.lines() {
                    self.write_line(line);
                }
            }

            IRStmt::EmitPageLayout { page, .. } => {
                crate::plugin::emit_page_layout_body(self, page);
            }

            IRStmt::FunctionDef(f) => {
                self.emit_function_def(f, false);
            }

            IRStmt::ClassDef(cls) => {
                self.emit_custom_class(cls);
            }

            IRStmt::ModuleDef {
                name,
                body,
                exports,
                ..
            } => {
                self.write_line(&format!("local {name}"));
                self.write_line("do");
                self.indent();

                let saved_locals = self.ctx.declared_locals.clone();
                self.emit_hoisted_statements(body);
                self.ctx.declared_locals = saved_locals;

                self.write_line(&format!("{name} = {{"));
                self.indent();
                for exp in exports {
                    self.write_line(&format!("{exp} = {exp},"));
                }
                self.dedent();
                self.write_line("}");

                self.dedent();
                self.write_line("end\n");
            }

            IRStmt::ImportBinding {
                local_name, target, ..
            } => {
                let target_str = self.emit_expr(target);
                self.write_line(&format!("local {local_name} = {target_str}"));
            }
        }
    }

    pub fn emit_param_defaults(&mut self, params: &[IRParam]) {
        for param in params {
            if let Some(ref def_expr) = param.default {
                let def_str = self.emit_expr(def_expr);
                self.write_line(&format!(
                    "if {} == nil then {} = {def_str} end",
                    param.name, param.name
                ));
            }
        }
    }

    pub fn emit_function_def(&mut self, f: &IRFunction, already_declared: bool) {
        self.ctx.functions.insert(f.id, f.name.clone());
        for param in &f.params {
            self.ctx.symbol_names.insert(param.id, param.name.clone());
            self.ctx.declared_locals.insert(param.id);
        }

        let params: Vec<_> = f.params.iter().map(|p| p.name.as_str()).collect();
        let param_str = params.join(", ");

        if already_declared {
            self.write_line(&format!("function {}({})", f.name, param_str));
        } else {
            self.write_line(&format!("local function {}({})", f.name, param_str));
        }
        self.indent();
        self.emit_param_defaults(&f.params);
        self.emit_statements(&f.body);
        self.dedent();
        self.write_line("end\n");
    }

    fn emit_else_branch(&mut self, else_branch: &[IRStmt]) {
        if else_branch.is_empty() {
            return;
        }

        // Fold nested `if` into clean `elseif`
        if else_branch.len() == 1
            && let IRStmt::If {
                condition,
                truthiness,
                then_branch,
                else_branch: inner_else,
                ..
            } = &else_branch[0]
        {
            let cond_str = self.emit_truthy_condition(condition, *truthiness);
            self.write_line(&format!("elseif {cond_str} then"));
            self.indent();
            self.emit_statements(then_branch);
            self.dedent();
            self.emit_else_branch(inner_else);
            return;
        }

        self.write_line("else");
        self.indent();
        self.emit_statements(else_branch);
        self.dedent();
    }

    pub fn emit_truthy_condition(&mut self, expr: &IRExpr, truthiness: IRTruthiness) -> String {
        let raw = self.emit_expr(expr);
        match truthiness {
            IRTruthiness::Direct => raw,
            IRTruthiness::NumericNotZero => format!("({raw} ~= 0)"),
            IRTruthiness::StringNonEmpty => format!("(#{raw} > 0)"),
            IRTruthiness::CollectionNonEmpty => format!("(next({raw}) ~= nil)"),
            IRTruthiness::AlwaysFalse => "false".to_string(),
            IRTruthiness::Dynamic => {
                self.ctx.needs_truthy_helper = true;
                format!("__py_truthy({raw})")
            }
        }
    }

    pub fn emit_expr(&mut self, expr: &IRExpr) -> String {
        match expr {
            IRExpr::Int(i) => i.to_string(),
            IRExpr::Float(f) => format_lua_float(*f),
            IRExpr::String(s) => escape_lua_string(s),
            IRExpr::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            IRExpr::Nil => "nil".to_string(),

            IRExpr::Local(id) => self.ctx.get_symbol_name(*id),

            IRExpr::Control(id) => {
                if let Some(def) = self.ctx.controls.get(id) {
                    format!("Controls[{}]", escape_lua_string(&def.name))
                } else {
                    format!("Controls[_ctl_{}]", id.0)
                }
            }

            IRExpr::Property(id) => {
                if let Some(def) = self.ctx.properties.get(id) {
                    format!("Properties[{}].Value", escape_lua_string(&def.name))
                } else {
                    format!("Properties[_prop_{}].Value", id.0)
                }
            }

            IRExpr::Function(id) => {
                if let Some(name) = self.ctx.functions.get(id) {
                    name.clone()
                } else {
                    format!("_fn_{}", id.0)
                }
            }
            IRExpr::Global(name) => name.clone(),

            IRExpr::Tuple(items) | IRExpr::List(items) => {
                let rendered: Vec<_> = items.iter().map(|item| self.emit_expr(item)).collect();
                format!("{{ {} }}", rendered.join(", "))
            }

            IRExpr::Dict(pairs) => {
                let rendered: Vec<_> = pairs
                    .iter()
                    .map(|(k, v)| {
                        let k_str = self.emit_expr(k);
                        let v_str = self.emit_expr(v);
                        format!("[{k_str}] = {v_str}")
                    })
                    .collect();
                format!("{{ {} }}", rendered.join(", "))
            }

            IRExpr::Binary { left, op, right } => {
                if *op == IRBinaryOp::StringRep {
                    let str_val = self.emit_expr(left);
                    let count_val = self.emit_expr(right);
                    return format!("string.rep({str_val}, {count_val})");
                }

                let l_str = self.emit_expr(left);
                let r_str = self.emit_expr(right);
                let op_str = match op {
                    IRBinaryOp::StringConcat => "..",
                    IRBinaryOp::StringRep => unreachable!(),
                    IRBinaryOp::Add => "+",
                    IRBinaryOp::Sub => "-",
                    IRBinaryOp::Mul => "*",
                    IRBinaryOp::Div => "/",
                    IRBinaryOp::FloorDiv => "//",
                    IRBinaryOp::Mod => "%",
                    IRBinaryOp::Pow => "^",
                    IRBinaryOp::Concat => "..",
                    IRBinaryOp::BitAnd => "&",
                    IRBinaryOp::BitOr => "|",
                    IRBinaryOp::BitXor => "~",
                    IRBinaryOp::LShift => "<<",
                    IRBinaryOp::RShift => ">>",
                    IRBinaryOp::Eq => "==",
                    IRBinaryOp::NotEq => "~=",
                    IRBinaryOp::Lt => "<",
                    IRBinaryOp::LtEq => "<=",
                    IRBinaryOp::Gt => ">",
                    IRBinaryOp::GtEq => ">=",
                };
                format!("({l_str} {op_str} {r_str})")
            }

            IRExpr::LogicalAnd {
                left,
                right,
                left_truthiness,
            } => {
                let l_cond = self.emit_truthy_condition(left, *left_truthiness);
                let r_str = self.emit_expr(right);
                format!("({l_cond} and {r_str})")
            }

            IRExpr::LogicalOr {
                left,
                right,
                left_truthiness,
            } => {
                let l_cond = self.emit_truthy_condition(left, *left_truthiness);
                let r_str = self.emit_expr(right);
                format!("({l_cond} or {r_str})")
            }

            IRExpr::Unary {
                op,
                operand,
                truthiness,
            } => match op {
                IRUnaryOp::Not => match truthiness {
                    IRTruthiness::NumericNotZero => {
                        let op_str = self.emit_expr(operand);
                        format!("({op_str} == 0)")
                    }
                    IRTruthiness::StringNonEmpty => {
                        let op_str = self.emit_expr(operand);
                        format!("(#{op_str} == 0)")
                    }
                    IRTruthiness::CollectionNonEmpty => {
                        let op_str = self.emit_expr(operand);
                        format!("(next({op_str}) == nil)")
                    }
                    IRTruthiness::AlwaysFalse => "true".to_string(),
                    IRTruthiness::Direct => {
                        let op_str = self.emit_expr(operand);
                        format!("(not {op_str})")
                    }
                    IRTruthiness::Dynamic => {
                        self.ctx.needs_truthy_helper = true;
                        let op_str = self.emit_expr(operand);
                        format!("(not __py_truthy({op_str}))")
                    }
                },
                IRUnaryOp::Neg => format!("(-{})", self.emit_expr(operand)),
                IRUnaryOp::Pos => format!("(+{})", self.emit_expr(operand)),
                IRUnaryOp::BitNot => format!("(~{})", self.emit_expr(operand)),
                IRUnaryOp::Len => {
                    if let IRExpr::Control(ctl_id) = operand.as_ref()
                        && let Some(def) = self.ctx.controls.get(ctl_id)
                        && let Some(ref prop_name) = def.count_property
                    {
                        format_count_lua_expr(prop_name, "Properties")
                    } else {
                        format!("(#{})", self.emit_expr(operand))
                    }
                }
            },

            IRExpr::Call {
                callee,
                args,
                kwargs,
            } => {
                let raw_callee = self.emit_expr(callee);
                let callee_str = if matches!(callee.as_ref(), IRExpr::Lambda { .. }) {
                    format!("({raw_callee})")
                } else {
                    raw_callee
                };
                let mut rendered_args: Vec<_> =
                    args.iter().map(|arg| self.emit_expr(arg)).collect();

                if !kwargs.is_empty() {
                    let kw_fields: Vec<_> = kwargs
                        .iter()
                        .map(|(k, v)| format!("{k} = {}", self.emit_expr(v)))
                        .collect();
                    rendered_args.push(format!("{{ {} }}", kw_fields.join(", ")));
                }

                format!("{callee_str}({})", rendered_args.join(", "))
            }

            IRExpr::MethodCall {
                object,
                method,
                args,
                kwargs,
            } => {
                let obj_str = self.emit_expr(object);
                let mut rendered_args: Vec<_> =
                    args.iter().map(|arg| self.emit_expr(arg)).collect();

                if !kwargs.is_empty() {
                    let kw_fields: Vec<_> = kwargs
                        .iter()
                        .map(|(k, v)| format!("{k} = {}", self.emit_expr(v)))
                        .collect();
                    rendered_args.push(format!("{{ {} }}", kw_fields.join(", ")));
                }

                // Emitted with `:` for Q-SYS Lua userdata method dispatch
                format!("{obj_str}:{method}({})", rendered_args.join(", "))
            }

            IRExpr::Attribute { object, attr } => {
                if matches!(object.as_ref(), IRExpr::Property(_)) && attr == "Value" {
                    return self.emit_expr(object);
                }
                if ControlMember::from_ident(attr).is_some_and(|m| m.is_generic_accessor()) {
                    if let Some(def) = self.resolve_control_def_for_expr(object) {
                        let obj_str = self.emit_expr(object);
                        if let Some(ref gspec) = def.generic_spec {
                            self.ctx.needs_generic_data_helper = true;
                            let prop_str = format!(
                                "Properties[{}].Value",
                                escape_lua_string(&gspec.property_name)
                            );
                            return format!("__qsys_get_data({obj_str}, {prop_str})");
                        } else {
                            let member = match def.kind {
                                ControlKind::Text { .. } => "String",
                                ControlKind::Button { .. } | ControlKind::Indicator { .. } => {
                                    "Boolean"
                                }
                                ControlKind::Knob { .. } | ControlKind::Fader { .. } => "Value",
                            };
                            return format!("{obj_str}.{member}");
                        }
                    } else {
                        self.ctx.needs_generic_data_helper = true;
                        let obj_str = self.emit_expr(object);
                        let prop_arg = self.default_generic_prop_expr();
                        return format!("__qsys_get_data({obj_str}, {prop_arg})");
                    }
                }
                let obj_str = self.emit_expr(object);
                format!("{obj_str}.{attr}")
            }

            IRExpr::Index {
                base,
                index,
                is_zero_based,
            } => {
                let zero_based = *is_zero_based && !matches!(index.as_ref(), IRExpr::String(_));
                if zero_based
                    && let IRExpr::Control(ctl_id) = base.as_ref()
                    && let Some(def) = self.ctx.controls.get(ctl_id).cloned()
                    && let Some(ref prop_name) = def.count_property
                {
                    self.ctx.needs_ctl_index_helper = true;
                    let ctl_str = format!("Controls[{}]", escape_lua_string(&def.name));
                    let prop_str = format_count_lua_expr(prop_name, "Properties");
                    let idx_1 = self.format_one_based_index(index);
                    return format!("__qsys_ctl({ctl_str}, {idx_1}, {prop_str})");
                }

                let base_str = self.emit_expr(base);
                if zero_based {
                    let idx_1 = self.format_one_based_index(index);
                    format!("{base_str}[{idx_1}]")
                } else {
                    let idx_str = self.emit_expr(index);
                    format!("{base_str}[{idx_str}]")
                }
            }

            IRExpr::Lambda { params, body } => {
                for param in params {
                    self.ctx.symbol_names.insert(param.id, param.name.clone());
                    self.ctx.declared_locals.insert(param.id);
                }
                let param_list: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
                let body_str = self.emit_expr(body);
                format!(
                    "function({}) return {} end",
                    param_list.join(", "),
                    body_str
                )
            }

            IRExpr::IfExp {
                test,
                test_truthiness,
                body,
                orelse,
            } => {
                let cond_str = self.emit_truthy_condition(test, *test_truthiness);
                let body_str = self.emit_expr(body);
                let orelse_str = self.emit_expr(orelse);
                if matches!(body.as_ref(), IRExpr::Bool(false) | IRExpr::Nil) {
                    format!(
                        "(function() if {cond_str} then return {body_str} else return {orelse_str} end end)()"
                    )
                } else {
                    format!("({cond_str} and {body_str} or {orelse_str})")
                }
            }

            IRExpr::RawLua(code) => code.clone(),
        }
    }

    pub fn emit_custom_class(&mut self, cls: &IRClass) {
        let name = &cls.name;
        self.write_line(&format!("local {name} = {{}}"));
        self.write_line(&format!("{name}.__index = {name}"));
        if let Some(ref base) = cls.base {
            self.write_line(&format!("setmetatable({name}, {{ __index = {base} }})"));
        }
        for (attr_name, val_expr) in &cls.class_vars {
            let val_str = self.emit_expr(val_expr);
            self.write_line(&format!("{name}.{attr_name} = {val_str}"));
        }
        self.write_line("");

        // Emit constructor: `function ClassName.new(...)`
        let ctor_params = if let Some(ref ctor) = cls.constructor {
            for param in &ctor.params {
                self.ctx.symbol_names.insert(param.id, param.name.clone());
                self.ctx.declared_locals.insert(param.id);
            }
            ctor.params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            String::new()
        };

        self.write_line(&format!("function {name}.new({ctor_params})"));
        self.indent();
        self.write_line(&format!("local self = setmetatable({{}}, {name})"));
        if cls.emit_init_method {
            self.write_line(&format!("self:__init({ctor_params})"));
        } else if let Some(ref ctor) = cls.constructor {
            self.emit_param_defaults(&ctor.params);
            self.emit_statements(&ctor.body);
        }
        self.write_line("return self");
        self.dedent();
        self.write_line("end\n");

        if cls.emit_init_method {
            self.write_line(&format!("function {name}:__init({ctor_params})"));
            self.indent();
            if let Some(ref ctor) = cls.constructor {
                self.emit_param_defaults(&ctor.params);
                self.emit_statements(&ctor.body);
            }
            self.dedent();
            self.write_line("end\n");
        }

        // Emit instance methods: `function ClassName:method(...)`
        for method in &cls.methods {
            for param in &method.params {
                self.ctx.symbol_names.insert(param.id, param.name.clone());
                self.ctx.declared_locals.insert(param.id);
            }

            let params = method
                .params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let lua_method_name = match method.name.as_str() {
                "__str__" | "__repr__" => "__tostring",
                other => other,
            };
            self.write_line(&format!("function {name}:{lua_method_name}({params})"));
            self.indent();
            self.emit_param_defaults(&method.params);
            self.emit_statements(&method.body);
            self.dedent();
            self.write_line("end\n");
        }
    }

    pub fn resolve_control_id_for_expr(&self, expr: &IRExpr) -> Option<ControlId> {
        match expr {
            IRExpr::Control(ctl_id) => Some(*ctl_id),
            IRExpr::Index { base, .. } => self.resolve_control_id_for_expr(base),
            IRExpr::Local(sym_id) => self
                .ctx
                .local_controls
                .get(sym_id)
                .copied()
                .or(self.ctx.active_handler_control),
            _ => self.ctx.active_handler_control,
        }
    }

    pub fn resolve_control_def_for_expr(&self, expr: &IRExpr) -> Option<ControlDef> {
        self.resolve_control_id_for_expr(expr)
            .and_then(|ctl_id| self.ctx.controls.get(&ctl_id).cloned())
    }

    pub fn default_generic_prop_expr(&self) -> String {
        for def in self.ctx.controls.values() {
            if let Some(ref gspec) = def.generic_spec {
                return format!(
                    "Properties[{}].Value",
                    escape_lua_string(&gspec.property_name)
                );
            }
        }
        "nil".to_string()
    }

    pub fn format_one_based_index(&mut self, index: &IRExpr) -> String {
        match index {
            IRExpr::Int(n) => (n + 1).to_string(),
            IRExpr::Binary {
                left,
                op: IRBinaryOp::Add,
                right,
            } => {
                if let IRExpr::Int(n) = right.as_ref() {
                    let l_str = self.emit_expr(left);
                    if n + 1 == 0 {
                        return l_str;
                    } else if n + 1 > 0 {
                        return format!("{l_str} + {}", n + 1);
                    } else {
                        return format!("{l_str} - {}", -(n + 1));
                    }
                }
                let idx_str = self.emit_expr(index);
                format!("{idx_str} + 1")
            }
            IRExpr::Binary {
                left,
                op: IRBinaryOp::Sub,
                right,
            } => {
                if let IRExpr::Int(n) = right.as_ref() {
                    let l_str = self.emit_expr(left);
                    if *n == 1 {
                        return l_str;
                    } else if *n > 1 {
                        return format!("{l_str} - {}", n - 1);
                    } else {
                        return format!("{l_str} + {}", 1 - n);
                    }
                }
                let idx_str = self.emit_expr(index);
                format!("{idx_str} + 1")
            }
            _ => {
                let idx_str = self.emit_expr(index);
                if idx_str.starts_with('(') && idx_str.ends_with(')') {
                    format!("{idx_str} + 1")
                } else {
                    format!("({idx_str}) + 1")
                }
            }
        }
    }
}

pub fn format_count_lua_expr(spec: &str, table_var: &str) -> String {
    if spec.trim().parse::<i64>().is_ok() {
        return spec.trim().to_string();
    }
    if spec.contains("{PROP:") {
        let mut out = String::new();
        let mut rest = spec;
        while let Some(start) = rest.find("{PROP:") {
            out.push_str(&rest[..start]);
            let after_prefix = &rest[start + 6..];
            if let Some(end) = after_prefix.find('}') {
                let prop_name = &after_prefix[..end];
                out.push_str(&format!(
                    "{table_var}[{}].Value",
                    escape_lua_string(prop_name)
                ));
                rest = &after_prefix[end + 1..];
            } else {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
        out.push_str(rest);
        out
    } else {
        format!("{table_var}[{}].Value", escape_lua_string(spec))
    }
}

pub fn escape_lua_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\x00"),
            c if (c as u32) < 32 || (c as u32) == 127 => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if (c as u32) > 127 && (c as u32) <= 255 => {
                // Byte values in latin-1 / raw binary range (e.g. \xFF)
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

pub fn format_lua_float(f: f64) -> String {
    if f.is_nan() {
        "(0/0)".to_string()
    } else if f.is_infinite() {
        if f > 0.0 {
            "(1/0)".to_string()
        } else {
            "(-1/0)".to_string()
        }
    } else if f.fract() == 0.0 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn is_pure_const_expr(expr: &IRExpr) -> bool {
    match expr {
        IRExpr::Int(_) | IRExpr::Float(_) | IRExpr::String(_) | IRExpr::Bool(_) | IRExpr::Nil => {
            true
        }
        IRExpr::Tuple(items) | IRExpr::List(items) => items.iter().all(is_pure_const_expr),
        IRExpr::Dict(pairs) => pairs
            .iter()
            .all(|(k, v)| is_pure_const_expr(k) && is_pure_const_expr(v)),
        IRExpr::Binary { left, right, .. }
        | IRExpr::LogicalAnd { left, right, .. }
        | IRExpr::LogicalOr { left, right, .. } => {
            is_pure_const_expr(left) && is_pure_const_expr(right)
        }
        IRExpr::Unary { operand, .. } => is_pure_const_expr(operand),
        _ => false,
    }
}

fn sort_classes_topologically<'a>(classes: &[&'a IRClass]) -> Vec<&'a IRClass> {
    let mut visited = HashSet::new();
    let mut ordered = Vec::with_capacity(classes.len());

    fn visit<'a>(
        cls: &'a IRClass,
        all: &[&'a IRClass],
        visited: &mut HashSet<String>,
        ordered: &mut Vec<&'a IRClass>,
    ) {
        if !visited.insert(cls.name.clone()) {
            return;
        }
        if let Some(ref base_name) = cls.base
            && let Some(&base_cls) = all.iter().find(|c| &c.name == base_name)
        {
            visit(base_cls, all, visited, ordered);
        }
        ordered.push(cls);
    }

    for &cls in classes {
        visit(cls, classes, &mut visited, &mut ordered);
    }
    ordered
}

fn class_references_function(cls: &IRClass, fn_id: FunctionId, fn_name: &str) -> bool {
    cls.class_vars
        .iter()
        .any(|(_, e)| expr_references_function(e, fn_id, fn_name))
        || cls
            .constructor
            .as_ref()
            .is_some_and(|c| stmts_reference_function(&c.body, fn_id, fn_name))
        || cls
            .methods
            .iter()
            .any(|m| stmts_reference_function(&m.body, fn_id, fn_name))
}

pub(crate) fn stmts_reference_function(stmts: &[IRStmt], fn_id: FunctionId, fn_name: &str) -> bool {
    stmts
        .iter()
        .any(|s| stmt_references_function(s, fn_id, fn_name))
}

fn stmt_references_function(stmt: &IRStmt, fn_id: FunctionId, fn_name: &str) -> bool {
    match stmt {
        IRStmt::Assign { target, value, .. } => {
            expr_references_function(target, fn_id, fn_name)
                || expr_references_function(value, fn_id, fn_name)
        }
        IRStmt::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            expr_references_function(condition, fn_id, fn_name)
                || stmts_reference_function(then_branch, fn_id, fn_name)
                || stmts_reference_function(else_branch, fn_id, fn_name)
        }
        IRStmt::While {
            condition, body, ..
        } => {
            expr_references_function(condition, fn_id, fn_name)
                || stmts_reference_function(body, fn_id, fn_name)
        }
        IRStmt::For { iterable, body, .. } => {
            expr_references_function(iterable, fn_id, fn_name)
                || stmts_reference_function(body, fn_id, fn_name)
        }
        IRStmt::Return {
            value: Some(val), ..
        }
        | IRStmt::Expr { expr: val, .. } => expr_references_function(val, fn_id, fn_name),
        IRStmt::FunctionDef(f) => stmts_reference_function(&f.body, fn_id, fn_name),
        IRStmt::ClassDef(c) => class_references_function(c, fn_id, fn_name),
        _ => false,
    }
}

fn expr_references_function(expr: &IRExpr, fn_id: FunctionId, fn_name: &str) -> bool {
    match expr {
        IRExpr::Function(id) => *id == fn_id,
        IRExpr::Global(name) => name == fn_name,
        IRExpr::Tuple(items) | IRExpr::List(items) => items
            .iter()
            .any(|e| expr_references_function(e, fn_id, fn_name)),
        IRExpr::Dict(pairs) => pairs.iter().any(|(k, v)| {
            expr_references_function(k, fn_id, fn_name)
                || expr_references_function(v, fn_id, fn_name)
        }),
        IRExpr::Binary { left, right, .. }
        | IRExpr::LogicalAnd { left, right, .. }
        | IRExpr::LogicalOr { left, right, .. } => {
            expr_references_function(left, fn_id, fn_name)
                || expr_references_function(right, fn_id, fn_name)
        }
        IRExpr::Unary { operand, .. } => expr_references_function(operand, fn_id, fn_name),
        IRExpr::Call {
            callee,
            args,
            kwargs,
        } => {
            expr_references_function(callee, fn_id, fn_name)
                || args
                    .iter()
                    .any(|a| expr_references_function(a, fn_id, fn_name))
                || kwargs
                    .iter()
                    .any(|(_, v)| expr_references_function(v, fn_id, fn_name))
        }
        IRExpr::MethodCall {
            object,
            args,
            kwargs,
            ..
        } => {
            expr_references_function(object, fn_id, fn_name)
                || args
                    .iter()
                    .any(|a| expr_references_function(a, fn_id, fn_name))
                || kwargs
                    .iter()
                    .any(|(_, v)| expr_references_function(v, fn_id, fn_name))
        }
        IRExpr::Attribute { object, .. } => expr_references_function(object, fn_id, fn_name),
        IRExpr::Index { base, index, .. } => {
            expr_references_function(base, fn_id, fn_name)
                || expr_references_function(index, fn_id, fn_name)
        }
        IRExpr::Lambda { body, .. } => expr_references_function(body, fn_id, fn_name),
        IRExpr::IfExp {
            test, body, orelse, ..
        } => {
            expr_references_function(test, fn_id, fn_name)
                || expr_references_function(body, fn_id, fn_name)
                || expr_references_function(orelse, fn_id, fn_name)
        }
        _ => false,
    }
}

fn class_references_local(cls: &IRClass, sym_id: SymbolId) -> bool {
    cls.class_vars
        .iter()
        .any(|(_, e)| expr_references_local(e, sym_id))
        || cls
            .constructor
            .as_ref()
            .is_some_and(|c| stmts_reference_local(&c.body, sym_id))
        || cls
            .methods
            .iter()
            .any(|m| stmts_reference_local(&m.body, sym_id))
}

fn stmts_reference_local(stmts: &[IRStmt], sym_id: SymbolId) -> bool {
    stmts.iter().any(|s| stmt_references_local(s, sym_id))
}

fn stmt_references_local(stmt: &IRStmt, sym_id: SymbolId) -> bool {
    match stmt {
        IRStmt::Assign { target, value, .. } => {
            expr_references_local(target, sym_id) || expr_references_local(value, sym_id)
        }
        IRStmt::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            expr_references_local(condition, sym_id)
                || stmts_reference_local(then_branch, sym_id)
                || stmts_reference_local(else_branch, sym_id)
        }
        IRStmt::While {
            condition, body, ..
        } => expr_references_local(condition, sym_id) || stmts_reference_local(body, sym_id),
        IRStmt::For { iterable, body, .. } => {
            expr_references_local(iterable, sym_id) || stmts_reference_local(body, sym_id)
        }
        IRStmt::Return {
            value: Some(val), ..
        }
        | IRStmt::Expr { expr: val, .. } => expr_references_local(val, sym_id),
        IRStmt::FunctionDef(f) => stmts_reference_local(&f.body, sym_id),
        IRStmt::ClassDef(c) => class_references_local(c, sym_id),
        _ => false,
    }
}

fn expr_references_local(expr: &IRExpr, sym_id: SymbolId) -> bool {
    match expr {
        IRExpr::Local(id) => *id == sym_id,
        IRExpr::Tuple(items) | IRExpr::List(items) => {
            items.iter().any(|e| expr_references_local(e, sym_id))
        }
        IRExpr::Dict(pairs) => pairs
            .iter()
            .any(|(k, v)| expr_references_local(k, sym_id) || expr_references_local(v, sym_id)),
        IRExpr::Binary { left, right, .. }
        | IRExpr::LogicalAnd { left, right, .. }
        | IRExpr::LogicalOr { left, right, .. } => {
            expr_references_local(left, sym_id) || expr_references_local(right, sym_id)
        }
        IRExpr::Unary { operand, .. } => expr_references_local(operand, sym_id),
        IRExpr::Call {
            callee,
            args,
            kwargs,
        } => {
            expr_references_local(callee, sym_id)
                || args.iter().any(|a| expr_references_local(a, sym_id))
                || kwargs.iter().any(|(_, v)| expr_references_local(v, sym_id))
        }
        IRExpr::MethodCall {
            object,
            args,
            kwargs,
            ..
        } => {
            expr_references_local(object, sym_id)
                || args.iter().any(|a| expr_references_local(a, sym_id))
                || kwargs.iter().any(|(_, v)| expr_references_local(v, sym_id))
        }
        IRExpr::Attribute { object, .. } => expr_references_local(object, sym_id),
        IRExpr::Index { base, index, .. } => {
            expr_references_local(base, sym_id) || expr_references_local(index, sym_id)
        }
        IRExpr::Lambda { body, .. } => expr_references_local(body, sym_id),
        IRExpr::IfExp {
            test, body, orelse, ..
        } => {
            expr_references_local(test, sym_id)
                || expr_references_local(body, sym_id)
                || expr_references_local(orelse, sym_id)
        }
        _ => false,
    }
}
