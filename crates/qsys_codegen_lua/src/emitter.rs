use qsys_ir::*;

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
                    let var_name = self.ctx.get_symbol_name(*sym_id);
                    let val_str = self.emit_expr(value);
                    self.write_indent();
                    if self.ctx.declared_locals.insert(*sym_id) {
                        self.buffer
                            .push_str(&format!("local {var_name} = {val_str}\n"));
                    } else {
                        self.buffer.push_str(&format!("{var_name} = {val_str}\n"));
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
                let iter_str = self.emit_expr(iterable);

                self.write_line(&format!("for _, {var_name} in ipairs({iter_str}) do"));
                self.indent();
                self.emit_statements(body);
                self.dedent();
                self.write_line("end");
            }

            IRStmt::Return { value, .. } => {
                if let Some(val) = value {
                    let val_str = self.emit_expr(val);
                    self.write_line(&format!("return {val_str}"));
                } else {
                    self.write_line("return");
                }
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

            IRStmt::FunctionDef(f) => {
                // 1. Register parameter IDs into context so the body knows their names!
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
                self.write_line("end");
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
                    name.clone() // <-- Resolves to "PluginName_function_name"
                } else {
                    format!("_fn_{}", id.0)
                }
            }
            IRExpr::Global(name) => name.clone(),

            IRExpr::List(items) => {
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
                let obj_str = self.emit_expr(object);
                format!("{obj_str}.{attr}")
            }

            IRExpr::Index {
                base,
                index,
                is_zero_based,
            } => {
                let base_str = self.emit_expr(base);
                let idx_str = self.emit_expr(index);
                if *is_zero_based {
                    format!("{base_str}[({idx_str}) + 1]")
                } else {
                    format!("{base_str}[{idx_str}]")
                }
            }

            IRExpr::RawLua(code) => code.clone(),
        }
    }

    pub fn emit_custom_class(&mut self, cls: &IRClass) {
        let name = &cls.name;
        self.write_line(&format!("local {name} = {{}}"));
        self.write_line(&format!("{name}.__index = {name}\n"));

        // Emit constructor: `function ClassName.new(...)`
        let ctor_params = if let Some(ref ctor) = cls.constructor {
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
        if let Some(ref ctor) = cls.constructor {
            self.emit_statements(&ctor.body);
        }
        self.write_line("return self");
        self.dedent();
        self.write_line("end\n");

        // Emit instance methods: `function ClassName:method(...)`
        for method in &cls.methods {
            // Register method parameters into context!
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
            self.write_line(&format!("function {name}:{}({params})", method.name));
            self.indent();
            self.emit_statements(&method.body);
            self.dedent();
            self.write_line("end\n");
        }
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
