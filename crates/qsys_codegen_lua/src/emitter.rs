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
                self.ctx.functions.insert(f.id, f.name.clone());
                for param in &f.params {
                    self.ctx.symbol_names.insert(param.id, param.name.clone());
                    self.ctx.declared_locals.insert(param.id);
                }

                let params: Vec<_> = f.params.iter().map(|p| p.name.as_str()).collect();
                let param_str = params.join(", ");

                self.write_line(&format!("local function {}({})", f.name, param_str));
                self.indent();
                for s in &f.body {
                    self.emit_stmt(s);
                }
                self.dedent();
                self.write_line("end\n");
            }

            IRStmt::ClassDef(cls) => {
                self.emit_custom_class(cls);
            }
        }
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
                let callee_str = self.emit_expr(callee);
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
                if *is_zero_based
                    && let IRExpr::Control(ctl_id) = base.as_ref()
                    && let Some(def) = self.ctx.controls.get(ctl_id).cloned()
                    && let Some(ref prop_name) = def.count_property
                {
                    let ctl_str = format!("Controls[{}]", escape_lua_string(&def.name));
                    let prop_str = format_count_lua_expr(prop_name, "Properties");
                    let idx_1 = if let IRExpr::Int(n) = index.as_ref() {
                        (n + 1).to_string()
                    } else {
                        let idx_str = self.emit_expr(index);
                        format!("({idx_str}) + 1")
                    };
                    return format!("(({prop_str} == 1) and {ctl_str} or {ctl_str}[{idx_1}])");
                }

                let base_str = self.emit_expr(base);
                if *is_zero_based {
                    if let IRExpr::Int(n) = index.as_ref() {
                        format!("{base_str}[{}]", n + 1)
                    } else {
                        let idx_str = self.emit_expr(index);
                        format!("{base_str}[({idx_str}) + 1]")
                    }
                } else {
                    let idx_str = self.emit_expr(index);
                    format!("{base_str}[{idx_str}]")
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
            self.emit_statements(&ctor.body);
        }
        self.write_line("return self");
        self.dedent();
        self.write_line("end\n");

        if cls.emit_init_method {
            self.write_line(&format!("function {name}:__init({ctor_params})"));
            self.indent();
            if let Some(ref ctor) = cls.constructor {
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
}

pub fn format_count_lua_expr(spec: &str, table_var: &str) -> String {
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
