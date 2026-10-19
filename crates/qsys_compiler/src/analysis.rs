use std::sync::Arc;

use serde::{Deserialize, Serialize};

use qsys_diagnostics::{FileId, SourceFile};
use qsys_model::{ControlKind, ControlMember, PropertyKind};
use qsys_parser::{ParsedModule, ast, parse_module_lossy};
use qsys_sema::{
    DeclaredControl, DeclaredProperty, ResolvedMethod, SemanticModel, TargetKind, Type,
    analyze_lossy,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHover {
    pub contents: Vec<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspCompletion {
    pub label: String,
    pub kind: String,
    pub detail: String,
    pub documentation: String,
    pub insert_text: String,
    pub is_snippet: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspInlayHint {
    pub line: usize,
    pub col: usize,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspParameterInfo {
    pub label: String,
    pub documentation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSignatureHelp {
    pub label: String,
    pub documentation: String,
    pub parameters: Vec<LspParameterInfo>,
    pub active_parameter: usize,
}

pub struct Analyzer;

impl Analyzer {
    /// Queries the Rust semantic model to produce hover documentation at (line, col).
    pub fn hover(source: &str, line: usize, col: usize) -> Option<LspHover> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let (parsed, model) = parse_and_analyze_for_ide(source, line);

        let cursor_offset = file.offset(line, col);
        let (word, word_start, word_end) = get_word_at_offset(&file.src, cursor_offset)?;

        let (start_line, start_col) = file.location(word_start);
        let (end_line, end_col) = file.location(word_end);

        let (controls, properties, methods) = extract_target_members(&model);

        // 1. Search for declared controls (self.Gain or Gain)
        for ctl in controls {
            if word == ctl.ident {
                let kind_details = match &ctl.def.kind {
                    ControlKind::Knob {
                        unit,
                        min,
                        max,
                        default,
                    }
                    | ControlKind::Fader {
                        unit,
                        min,
                        max,
                        default,
                    } => format!(
                        "- **Range:** `{min}` to `{max}` (default `{default}`, unit `{}`)",
                        unit.as_str()
                    ),
                    ControlKind::Button { button_type } => {
                        format!("- **Button Type:** `{}`", button_type.as_str())
                    }
                    ControlKind::Text { text_type, default } => format!(
                        "- **Text Type:** `{}` (default `\"{default}\"`)",
                        text_type.as_str()
                    ),
                    ControlKind::Indicator { unit } => {
                        format!("- **Unit:** `{}`", unit.as_str())
                    }
                };
                return Some(LspHover {
                    contents: vec![
                        format!("*(control)* {}: ControlHandle", ctl.ident),
                        format!(
                            "**Q-SYS Control Configuration:**\n- **Hardware Name:** `\"{}\"`\n- **Widget Type:** `{}`\n{}\n- **Pins:** `{}` (count: `{}`)\n- **Lua Target:** `Controls[\"{}\"]`",
                            ctl.def.name,
                            ctl.def.kind.qsys_type_str(),
                            kind_details,
                            ctl.def.pin_style.as_str(),
                            ctl.def.count,
                            ctl.def.name
                        ),
                    ],
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                });
            }
        }

        // 2. Search for declared properties (self.Channels or Channels)
        for prop in properties {
            if word == prop.ident {
                let prop_details = match &prop.def.kind {
                    PropertyKind::Integer { default, min, max } => {
                        format!("Integer (default: `{default}`, min: `{min:?}`, max: `{max:?}`)")
                    }
                    PropertyKind::Double { default, min, max } => {
                        format!("Double (default: `{default}`, min: `{min:?}`, max: `{max:?}`)")
                    }
                    PropertyKind::String { default } => {
                        format!("String (default: `\"{default}\"`)")
                    }
                    PropertyKind::Enum { choices, default } => {
                        format!("Enum (default: `\"{default}\"`, choices: `{choices:?}`)")
                    }
                    PropertyKind::Boolean { default } => {
                        format!("Boolean (default: `{default}`)")
                    }
                };
                return Some(LspHover {
                    contents: vec![
                        format!("*(property)* {}: PropertyHandle", prop.ident),
                        format!(
                            "**Q-SYS Inspector Property:**\n- **Inspector Name:** `\"{}\"`\n- **Type:** {}\n- **Lua Target:** `Properties[\"{}\"].Value`",
                            prop.def.name, prop_details, prop.def.name
                        ),
                    ],
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                });
            }
        }

        // 3. Search for runtime ControlMember properties (.Value, .String, .Boolean, etc.)
        for &member_name in ControlMember::all_member_names() {
            if word == member_name {
                let (ty, doc) = control_member_doc(member_name);
                return Some(LspHover {
                    contents: vec![
                        format!("*(property)* ControlHandle.{member_name}: {ty}"),
                        doc.to_string(),
                    ],
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                });
            }
        }

        // 4. Search for declared methods / functions
        for method in methods {
            if word == method.name {
                let params_str = method
                    .params
                    .iter()
                    .map(|(_, n)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Some(LspHover {
                    contents: vec![
                        format!("*(method)* def {}({})", method.name, params_str),
                        "Compiled Q-SYS runtime or design-time lifecycle function.".to_string(),
                    ],
                    start_line,
                    start_col,
                    end_line,
                    end_col,
                });
            }
        }

        // 4b. Search custom classes and their methods in the parsed AST
        for stmt in &parsed.module.statements {
            if let ast::StmtKind::ClassDef(cls) = &stmt.kind
                && !cls
                    .bases
                    .iter()
                    .any(|b| b == "QSysPlugin" || b == "TextController")
            {
                if word == cls.name {
                    let method_list: Vec<String> = cls
                        .body
                        .iter()
                        .filter_map(|s| {
                            if let ast::StmtKind::FunctionDef(f) = &s.kind {
                                let ret = f.return_type.as_deref().unwrap_or("Any");
                                Some(format!("- `def {}(...) -> {ret}`", f.name))
                            } else {
                                None
                            }
                        })
                        .collect();
                    let doc = if method_list.is_empty() {
                        format!(
                            "Custom class `{}` (compiled to Lua metatable `{}.new()`).",
                            cls.name, cls.name
                        )
                    } else {
                        format!(
                            "Custom class `{}` (compiled to Lua metatable `{}.new()`).\n\n**Methods:**\n{}",
                            cls.name,
                            cls.name,
                            method_list.join("\n")
                        )
                    };
                    return Some(LspHover {
                        contents: vec![format!("*(class)* `class {}`", cls.name), doc],
                        start_line,
                        start_col,
                        end_line,
                        end_col,
                    });
                }

                for body_stmt in &cls.body {
                    if let ast::StmtKind::FunctionDef(f) = &body_stmt.kind
                        && word == f.name
                    {
                        let params_str = f
                            .params
                            .iter()
                            .map(|p| {
                                if let Some(ref ann) = p.type_annotation {
                                    format!("{}: {ann}", p.name)
                                } else {
                                    p.name.clone()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let ret_str = f
                            .return_type
                            .as_deref()
                            .map(|r| format!(" -> {r}"))
                            .unwrap_or_default();
                        return Some(LspHover {
                            contents: vec![
                                format!(
                                    "*(method)* `def {}.{}({}){}`",
                                    cls.name, f.name, params_str, ret_str
                                ),
                                format!(
                                    "Method on custom class `{}` (emitted as `function {}:{}()` in Lua).",
                                    cls.name, cls.name, f.name
                                ),
                            ],
                            start_line,
                            start_col,
                            end_line,
                            end_col,
                        });
                    }
                }
            }
        }

        // 5. Built-in Q-SYS symbols & methods
        if let Some((sig, doc)) = builtin_symbol_doc(word) {
            return Some(LspHover {
                contents: vec![sig.to_string(), doc.to_string()],
                start_line,
                start_col,
                end_line,
                end_col,
            });
        }

        // 6. Search TypeMap for the narrowest inferred expression/variable type at cursor
        let mut best_span: Option<(qsys_diagnostics::Span, &Type)> = None;
        for (&span, ty) in model.type_map.types() {
            if cursor_offset >= span.start && cursor_offset <= span.end {
                if matches!(ty, Type::Any | Type::None) {
                    continue;
                }
                let span_len = span.end.saturating_sub(span.start);
                if best_span.is_none_or(|(b, _)| span_len <= b.end.saturating_sub(b.start)) {
                    best_span = Some((span, ty));
                }
            }
        }

        if let Some((span, ty)) = best_span {
            let (s_line, s_col) = file.location(span.start);
            let (e_line, e_col) = file.location(span.end);
            if let Some(ty_desc) = format_type_display(ty) {
                return Some(LspHover {
                    contents: vec![format!("*(type)* `{word}`: `{ty_desc}`")],
                    start_line: s_line,
                    start_col: s_col,
                    end_line: e_line,
                    end_col: e_col,
                });
            }
        }

        None
    }

    /// Queries the Rust compiler AST & symbol tables for fault-tolerant, context-aware completions.
    pub fn completions(source: &str, line: usize, col: usize) -> Vec<LspCompletion> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let cursor_offset = file.offset(line, col);
        let prefix = get_prefix_before_cursor(&file.src, cursor_offset);
        let line_prefix = get_line_before_cursor(&file.src, cursor_offset);

        let (parsed, model) = parse_and_analyze_for_ide(source, line);
        let (controls, properties, methods) = extract_target_members(&model);

        let mut items = Vec::new();

        // A. Decorator completions: `@`, `@Send`, `@SendButton.`, `@SendButton.Ev`
        let trimmed_line = line_prefix.trim_start();
        if let Some(after_at) = trimmed_line.strip_prefix('@') {
            if let Some((ctl_ident, partial_member)) = after_at.split_once('.') {
                if "EventHandler"
                    .to_ascii_lowercase()
                    .starts_with(&partial_member.to_ascii_lowercase())
                {
                    items.push(LspCompletion {
                        label: "EventHandler".to_string(),
                        kind: "Event".to_string(),
                        detail: format!("@{ctl_ident}.EventHandler"),
                        documentation: format!(
                            "Binds the following method as the Q-SYS runtime EventHandler for `{ctl_ident}`."
                        ),
                        insert_text: format!(
                            "EventHandler\ndef on_{}(self, ctl: ControlHandle):\n    ${{0:pass}}",
                            ctl_ident.to_lowercase()
                        ),
                        is_snippet: true,
                    });
                }
                return items;
            }

            let filter = after_at.trim();
            for ctl in controls {
                let candidate = format!("{}.EventHandler", ctl.ident);
                if filter.is_empty()
                    || candidate
                        .to_ascii_lowercase()
                        .starts_with(&filter.to_ascii_lowercase())
                    || ctl
                        .ident
                        .to_ascii_lowercase()
                        .starts_with(&filter.to_ascii_lowercase())
                {
                    items.push(LspCompletion {
                        label: candidate.clone(),
                        kind: "Event".to_string(),
                        detail: format!("Event handler for {} (\"{}\")", ctl.ident, ctl.def.name),
                        documentation: format!(
                            "Binds a handler method to changes on Q-SYS control `{}` (`Controls[\"{}\"].EventHandler`).",
                            ctl.ident, ctl.def.name
                        ),
                        insert_text: format!(
                            "{}.EventHandler\ndef on_{}(self, ctl: ControlHandle):\n    ${{0:pass}}",
                            ctl.ident,
                            ctl.ident.to_lowercase()
                        ),
                        is_snippet: true,
                    });
                }
            }
            return items;
        }

        // B. Dotted member access: `<receiver>.<partial>`
        if let Some((receiver, partial)) = prefix.rsplit_once('.') {
            let receiver =
                receiver.trim_start_matches(['(', '[', '{', ',', '=', ':', '+', '-', '*', '/']);

            // B1. `self.<partial>` -> Controls, Properties, Methods, Metadata
            if receiver == "self" {
                for ctl in controls {
                    if matches_prefix(&ctl.ident, partial) {
                        items.push(LspCompletion {
                            label: ctl.ident.clone(),
                            kind: "Field".to_string(),
                            detail: format!("(control) {}: ControlHandle", ctl.ident),
                            documentation: format!(
                                "Q-SYS Hardware Control\n- **Hardware Name:** `\"{}\"`\n- **Type:** `{}`\n- **Pins:** `{}`",
                                ctl.def.name,
                                ctl.def.kind.qsys_type_str(),
                                ctl.def.pin_style.as_str()
                            ),
                            insert_text: ctl.ident.clone(),
                            is_snippet: false,
                        });
                    }
                }

                for prop in properties {
                    if matches_prefix(&prop.ident, partial) {
                        items.push(LspCompletion {
                            label: prop.ident.clone(),
                            kind: "Property".to_string(),
                            detail: format!(
                                "(property) {}: {}",
                                prop.ident,
                                prop.def.kind.qsys_type_name()
                            ),
                            documentation: format!(
                                "Q-SYS Inspector Property `Properties[\"{}\"].Value`",
                                prop.def.name
                            ),
                            insert_text: prop.ident.clone(),
                            is_snippet: false,
                        });
                    }
                }

                for method in methods {
                    if matches_prefix(&method.name, partial) {
                        let params_str = method
                            .params
                            .iter()
                            .map(|(_, n)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let call_snippet = if method.params.is_empty() {
                            format!("{}()", method.name)
                        } else {
                            format!("{}(${{0}})", method.name)
                        };
                        items.push(LspCompletion {
                            label: method.name.clone(),
                            kind: "Method".to_string(),
                            detail: format!(
                                "def {}(self{})",
                                method.name,
                                if params_str.is_empty() {
                                    String::new()
                                } else {
                                    format!(", {params_str}")
                                }
                            ),
                            documentation: format!("Calls plugin method `{}`.", method.name),
                            insert_text: call_snippet,
                            is_snippet: !method.params.is_empty(),
                        });
                    }
                }

                return items;
            }

            // B2. `Control.<partial>` -> Control factory methods
            if receiver == "Control" {
                for item in control_factory_completions() {
                    if matches_prefix(&item.label, partial) {
                        items.push(item);
                    }
                }
                return items;
            }

            // B3. `Property.<partial>` -> Property factory methods
            if receiver == "Property" {
                for item in property_factory_completions() {
                    if matches_prefix(&item.label, partial) {
                        items.push(item);
                    }
                }
                return items;
            }

            // B4. `UdpSocket.<partial>`, `TcpSocket.<partial>`, `Timer.<partial>`, `qsys.<partial>`
            if matches!(receiver, "UdpSocket" | "TcpSocket" | "Timer") {
                if matches_prefix("New", partial) {
                    items.push(LspCompletion {
                        label: "New".to_string(),
                        kind: "Method".to_string(),
                        detail: format!("{receiver}.New() -> {receiver}Handle"),
                        documentation: format!(
                            "Allocates a new Q-SYS `{receiver}` runtime instance."
                        ),
                        insert_text: "New()".to_string(),
                        is_snippet: false,
                    });
                }
                return items;
            }

            if receiver == "qsys" {
                if matches_prefix("lua", partial) {
                    items.push(LspCompletion {
                        label: "lua".to_string(),
                        kind: "Function".to_string(),
                        detail: "qsys.lua(code: str) -> None".to_string(),
                        documentation:
                            "Injects raw, uncompiled Lua 5.3 code directly into the output."
                                .to_string(),
                        insert_text: "lua(\"${1:print('hello from lua')}\")".to_string(),
                        is_snippet: true,
                    });
                }
                return items;
            }

            // B5. Check receiver type (`self.<Control>.<partial>`, `ctl.<partial>`, `sock.<partial>`, `timer.<partial>`)
            let receiver_ty = infer_receiver_type(receiver, controls, properties, &parsed, &model);
            match receiver_ty {
                Some(Type::UdpSocketHandle) => {
                    for item in udp_socket_completions() {
                        if matches_prefix(&item.label, partial) {
                            items.push(item);
                        }
                    }
                    return items;
                }
                Some(Type::TcpSocketHandle) => {
                    for item in tcp_socket_completions() {
                        if matches_prefix(&item.label, partial) {
                            items.push(item);
                        }
                    }
                    return items;
                }
                Some(Type::TimerHandle) => {
                    for item in timer_completions() {
                        if matches_prefix(&item.label, partial) {
                            items.push(item);
                        }
                    }
                    return items;
                }
                Some(Type::PropertyHandle) => {
                    if matches_prefix("Value", partial) {
                        items.push(LspCompletion {
                            label: "Value".to_string(),
                            kind: "Property".to_string(),
                            detail: "(property) PropertyHandle.Value".to_string(),
                            documentation:
                                "Value of the property configured in the Q-SYS Properties panel."
                                    .to_string(),
                            insert_text: "Value".to_string(),
                            is_snippet: false,
                        });
                    }
                    return items;
                }
                Some(Type::UserInstance { ref name }) => {
                    let mut seen = Vec::new();
                    let mut current_cls = Some(name.as_str());
                    let mut depth = 0;
                    while let Some(target_cls_name) = current_cls {
                        if depth > 16 {
                            break;
                        }
                        depth += 1;
                        for stmt in &parsed.module.statements {
                            if let ast::StmtKind::ClassDef(cls) = &stmt.kind
                                && cls.name == target_cls_name
                            {
                                for body_stmt in &cls.body {
                                    if let ast::StmtKind::FunctionDef(f) = &body_stmt.kind {
                                        if matches!(
                                            f.name.as_str(),
                                            "__init__" | "__str__" | "__repr__"
                                        ) || seen.contains(&f.name)
                                        {
                                            continue;
                                        }
                                        if matches_prefix(&f.name, partial) {
                                            seen.push(f.name.clone());
                                            let user_params: Vec<_> = f
                                                .params
                                                .iter()
                                                .filter(|p| p.name != "self")
                                                .map(|p| p.name.as_str())
                                                .collect();
                                            let ret_str = f
                                                .return_type
                                                .as_deref()
                                                .map(|r| format!(" -> {r}"))
                                                .unwrap_or_default();
                                            let insert_text = if user_params.is_empty() {
                                                format!("{}()", f.name)
                                            } else {
                                                format!("{}(${{0}})", f.name)
                                            };
                                            items.push(LspCompletion {
                                                label: f.name.clone(),
                                                kind: "Method".to_string(),
                                                detail: format!(
                                                    "def {}.{}({}){}",
                                                    cls.name,
                                                    f.name,
                                                    user_params.join(", "),
                                                    ret_str
                                                ),
                                                documentation: format!(
                                                    "Calls `{}:{}(...)` on `{}` instance.",
                                                    cls.name, f.name, name
                                                ),
                                                insert_text,
                                                is_snippet: !user_params.is_empty(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        current_cls = model.type_map.get_class_base(target_cls_name);
                    }

                    for member_name in model.type_map.class_member_names(name) {
                        if !seen.contains(&member_name) && matches_prefix(&member_name, partial) {
                            let ty_str = model
                                .type_map
                                .get_class_member_type(name, &member_name)
                                .and_then(format_type_display)
                                .unwrap_or_else(|| "Any".to_string());
                            items.push(LspCompletion {
                                label: member_name.clone(),
                                kind: "Field".to_string(),
                                detail: format!("(field) {name}.{member_name}: {ty_str}"),
                                documentation: format!("Instance attribute on `{name}`."),
                                insert_text: member_name,
                                is_snippet: false,
                            });
                        }
                    }
                    return items;
                }
                _ => {
                    // Default to ControlHandle runtime members (covers `self.Gain.`, `ctl.`, `Controls["x"].`)
                    for &member_name in ControlMember::all_member_names() {
                        if member_name == "EventHandler" {
                            continue;
                        }
                        if matches_prefix(member_name, partial) {
                            let (ty, doc) = control_member_doc(member_name);
                            items.push(LspCompletion {
                                label: member_name.to_string(),
                                kind: "Property".to_string(),
                                detail: format!("(property) {member_name}: {ty}"),
                                documentation: doc.to_string(),
                                insert_text: member_name.to_string(),
                                is_snippet: false,
                            });
                        }
                    }
                    return items;
                }
            }
        }

        // C. Top-level / statement / identifier completions
        let ident_partial =
            prefix.trim_start_matches(['(', '[', '{', ',', '=', ':', '+', '-', '*', '/']);

        // Include custom classes and module-level variables from the AST
        for stmt in &parsed.module.statements {
            match &stmt.kind {
                ast::StmtKind::ClassDef(cls)
                    if !cls
                        .bases
                        .iter()
                        .any(|b| b == "QSysPlugin" || b == "TextController") =>
                {
                    if matches_prefix(&cls.name, ident_partial) {
                        items.push(LspCompletion {
                            label: cls.name.clone(),
                            kind: "Class".to_string(),
                            detail: format!("class {} -> {}.new()", cls.name, cls.name),
                            documentation: format!(
                                "Instantiates custom class `{}` (`{}.new()`).",
                                cls.name, cls.name
                            ),
                            insert_text: format!("{}()", cls.name),
                            is_snippet: false,
                        });
                    }
                }
                ast::StmtKind::Assign { target, .. } => {
                    if let ast::ExprKind::Ident(var_name) = &target.kind
                        && matches_prefix(var_name, ident_partial)
                    {
                        let ty_str = model
                            .type_map
                            .get_type(target.span)
                            .and_then(format_type_display)
                            .unwrap_or_else(|| "Any".to_string());
                        items.push(LspCompletion {
                            label: var_name.clone(),
                            kind: "Variable".to_string(),
                            detail: format!("(local) {var_name}: {ty_str}"),
                            documentation: format!("Module-scope variable `{var_name}`."),
                            insert_text: var_name.clone(),
                            is_snippet: false,
                        });
                    }
                }
                _ => {}
            }
        }

        for item in top_level_completions(controls, properties, methods) {
            if matches_prefix(&item.label, ident_partial) {
                items.push(item);
            }
        }

        items
    }

    /// Queries the Rust AST & TypeMap to produce accurate inline type hints.
    pub fn inlay_hints(source: &str) -> Vec<LspInlayHint> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let (parsed, model) = parse_and_analyze_for_ide(source, 0);

        let mut hints = Vec::new();

        collect_stmt_inlay_hints(&parsed.module.statements, &file, &model, &mut hints);

        for stmt in &parsed.module.statements {
            if let ast::StmtKind::ClassDef(cls) = &stmt.kind {
                for body_stmt in &cls.body {
                    if let ast::StmtKind::FunctionDef(f) = &body_stmt.kind {
                        let is_event_handler = !f.decorators.is_empty();
                        for (idx, param) in f.params.iter().enumerate() {
                            if idx == 1 && is_event_handler && param.type_annotation.is_none() {
                                let (line, col) = file.location(param.span.end);
                                hints.push(LspInlayHint {
                                    line,
                                    col,
                                    label: ": ControlHandle".to_string(),
                                });
                            }
                        }
                        collect_stmt_inlay_hints(&f.body, &file, &model, &mut hints);
                    }
                }
            } else if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                collect_stmt_inlay_hints(&f.body, &file, &model, &mut hints);
            }
        }

        hints
    }

    /// Produces signature help when the cursor is inside a function/method call `( ... )`.
    pub fn signature_help(source: &str, line: usize, col: usize) -> Option<LspSignatureHelp> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let cursor_offset = file.offset(line, col);
        let (callee_name, active_parameter) = find_enclosing_call(&file.src, cursor_offset)?;

        let (_, model) = parse_and_analyze_for_ide(source, line);
        let (_, _, methods) = extract_target_members(&model);

        if let Some(method_name) = callee_name.strip_prefix("self.") {
            for m in methods {
                if m.name == method_name {
                    let params: Vec<LspParameterInfo> = m
                        .params
                        .iter()
                        .map(|(_, name)| LspParameterInfo {
                            label: name.clone(),
                            documentation: format!("Parameter `{name}` of `{method_name}`."),
                        })
                        .collect();
                    let label_params = params
                        .iter()
                        .map(|p| p.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some(LspSignatureHelp {
                        label: format!("{}({})", m.name, label_params),
                        documentation: format!("Plugin method `{}`.", m.name),
                        parameters: params,
                        active_parameter,
                    });
                }
            }
        }

        builtin_signature_help(&callee_name, active_parameter)
    }
}

fn parse_and_analyze_for_ide(source: &str, cursor_line: usize) -> (ParsedModule, SemanticModel) {
    let file = SourceFile::new(FileId(1), "input.py", source);
    let (parsed, parse_diags) = parse_module_lossy(file);
    let (model, _) = analyze_lossy(&parsed);

    let has_class_keyword = source.contains("QSysPlugin") || source.contains("TextController");
    let lost_class_target =
        has_class_keyword && matches!(model.target, TargetKind::ProceduralScript(_));

    if (parse_diags.has_errors() || lost_class_target) && cursor_line > 0 {
        let sanitized = sanitize_line_for_recovery(source, cursor_line);
        let fallback_file = SourceFile::new(FileId(1), "input.py", sanitized);
        let (fallback_parsed, _) = parse_module_lossy(fallback_file);
        let (fallback_model, _) = analyze_lossy(&fallback_parsed);
        if !matches!(fallback_model.target, TargetKind::ProceduralScript(_)) || lost_class_target {
            return (fallback_parsed, fallback_model);
        }
    }

    (parsed, model)
}

fn sanitize_line_for_recovery(source: &str, cursor_line: usize) -> String {
    let mut lines: Vec<String> = source.lines().map(ToString::to_string).collect();
    let idx = cursor_line.saturating_sub(1);
    if idx < lines.len() {
        let line = &lines[idx];
        let indent_len = line.len() - line.trim_start().len();
        let indent = &line[..indent_len];
        let trimmed = line.trim();
        if trimmed.starts_with('@') {
            lines[idx] = String::new();
        } else {
            lines[idx] = format!("{indent}pass");
        }
    }
    lines.join("\n")
}

fn extract_target_members(
    model: &SemanticModel,
) -> (&[DeclaredControl], &[DeclaredProperty], &[ResolvedMethod]) {
    match &model.target {
        TargetKind::Plugin(p) => (&p.controls, &p.properties, &p.methods),
        TargetKind::TextController(c) => (&c.controls, &[], &c.methods),
        TargetKind::ProceduralScript(s) => (&[], &[], &s.methods),
    }
}

fn collect_stmt_inlay_hints(
    stmts: &[ast::Stmt],
    file: &Arc<SourceFile>,
    model: &SemanticModel,
    hints: &mut Vec<LspInlayHint>,
) {
    for stmt in stmts {
        match &stmt.kind {
            ast::StmtKind::Assign { target, value } => {
                let is_obvious_literal = matches!(
                    value.kind,
                    ast::ExprKind::Int(_)
                        | ast::ExprKind::Float(_)
                        | ast::ExprKind::String(_)
                        | ast::ExprKind::Bool(_)
                );
                if let ast::ExprKind::Ident(_) = &target.kind
                    && let Some(ty) = model.type_map.get_type(target.span)
                    && (matches!(
                        ty,
                        Type::UdpSocketHandle
                            | Type::TcpSocketHandle
                            | Type::TimerHandle
                            | Type::ControlHandle
                            | Type::PropertyHandle
                            | Type::UserInstance { .. }
                    ) || !is_obvious_literal)
                    && let Some(ty_str) = format_type_display(ty)
                {
                    let (line, col) = file.location(target.span.end);
                    hints.push(LspInlayHint {
                        line,
                        col,
                        label: format!(": {ty_str}"),
                    });
                }
            }
            ast::StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_stmt_inlay_hints(then_branch, file, model, hints);
                collect_stmt_inlay_hints(else_branch, file, model, hints);
            }
            ast::StmtKind::While { body, .. } | ast::StmtKind::For { body, .. } => {
                collect_stmt_inlay_hints(body, file, model, hints);
            }
            _ => {}
        }
    }
}

fn infer_receiver_type(
    receiver: &str,
    controls: &[DeclaredControl],
    properties: &[DeclaredProperty],
    parsed: &ParsedModule,
    model: &SemanticModel,
) -> Option<Type> {
    if let Some(member) = receiver.strip_prefix("self.") {
        if controls.iter().any(|c| c.ident == member) {
            return Some(Type::ControlHandle);
        }
        if properties.iter().any(|p| p.ident == member) {
            return Some(Type::PropertyHandle);
        }
    }

    if receiver.starts_with("Controls[") {
        return Some(Type::ControlHandle);
    }
    if receiver.starts_with("Properties[") {
        return Some(Type::PropertyHandle);
    }

    // Check TypeMap symbol names
    for (&sym_id, name) in model.type_map.symbol_names() {
        if name == receiver {
            for (&span, target) in model.type_map.resolutions() {
                if *target == qsys_sema::ResolvedTarget::Local(sym_id)
                    && let Some(ty) = model.type_map.get_type(span)
                    && *ty != Type::Any
                {
                    return Some(ty.clone());
                }
            }
        }
    }

    // Fallback: inspect AST assignments `receiver = UdpSocket.New()` or `receiver = Foo()`
    find_assigned_type_in_module(&parsed.module.statements, receiver)
}

fn find_assigned_type_in_module(stmts: &[ast::Stmt], var_name: &str) -> Option<Type> {
    for stmt in stmts {
        match &stmt.kind {
            ast::StmtKind::Assign { target, value } => {
                if let ast::ExprKind::Ident(name) = &target.kind
                    && name == var_name
                    && let ast::ExprKind::Call { callee, .. } = &value.kind
                {
                    if let ast::ExprKind::Attribute { value: recv, attr } = &callee.kind
                        && attr == "New"
                        && let ast::ExprKind::Ident(cls_name) = &recv.kind
                    {
                        match cls_name.as_str() {
                            "UdpSocket" => return Some(Type::UdpSocketHandle),
                            "TcpSocket" => return Some(Type::TcpSocketHandle),
                            "Timer" => return Some(Type::TimerHandle),
                            _ => {}
                        }
                    } else if let ast::ExprKind::Ident(cls_name) = &callee.kind
                        && cls_name.chars().next().is_some_and(char::is_uppercase)
                    {
                        return Some(Type::UserInstance {
                            name: cls_name.clone(),
                        });
                    }
                }
            }
            ast::StmtKind::ClassDef(cls) => {
                if let Some(ty) = find_assigned_type_in_module(&cls.body, var_name) {
                    return Some(ty);
                }
            }
            ast::StmtKind::FunctionDef(f) => {
                for (idx, p) in f.params.iter().enumerate() {
                    if p.name == var_name
                        && (p.type_annotation.as_deref() == Some("ControlHandle")
                            || (idx == 1 && !f.decorators.is_empty()))
                    {
                        return Some(Type::ControlHandle);
                    }
                }
                if let Some(ty) = find_assigned_type_in_module(&f.body, var_name) {
                    return Some(ty);
                }
            }
            _ => {}
        }
    }
    None
}

fn format_type_display(ty: &Type) -> Option<String> {
    match ty {
        Type::ControlHandle => Some("ControlHandle".to_string()),
        Type::PropertyHandle => Some("PropertyHandle".to_string()),
        Type::UdpSocketHandle => Some("UdpSocketHandle".to_string()),
        Type::TcpSocketHandle => Some("TcpSocketHandle".to_string()),
        Type::TimerHandle => Some("TimerHandle".to_string()),
        Type::UserInstance { name } => Some(name.clone()),
        Type::Float => Some("float".to_string()),
        Type::Int => Some("int".to_string()),
        Type::String => Some("str".to_string()),
        Type::Bool => Some("bool".to_string()),
        Type::List(inner) => Some(format!(
            "list[{}]",
            format_type_display(inner).unwrap_or_else(|| "Any".to_string())
        )),
        Type::Dict(k, v) => Some(format!(
            "dict[{}, {}]",
            format_type_display(k).unwrap_or_else(|| "Any".to_string()),
            format_type_display(v).unwrap_or_else(|| "Any".to_string())
        )),
        _ => None,
    }
}

fn matches_prefix(candidate: &str, partial: &str) -> bool {
    partial.is_empty()
        || candidate
            .to_ascii_lowercase()
            .starts_with(&partial.to_ascii_lowercase())
}

fn control_factory_completions() -> Vec<LspCompletion> {
    vec![
        LspCompletion {
            label: "Float".to_string(),
            kind: "Method".to_string(),
            detail: "Control.Float(name, min, max, default, unit, pins, count)".to_string(),
            documentation: "Continuous floating-point knob or fader control.".to_string(),
            insert_text:
                "Float(name=\"${1:Gain}\", min=${2:-100.0}, max=${3:20.0}, default=${4:0.0})"
                    .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Knob".to_string(),
            kind: "Method".to_string(),
            detail: "Control.Knob(name, min, max, default, unit, pins, count)".to_string(),
            documentation: "Rotary continuous knob control.".to_string(),
            insert_text:
                "Knob(name=\"${1:Trim}\", min=${2:-20.0}, max=${3:20.0}, default=${4:0.0})"
                    .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Fader".to_string(),
            kind: "Method".to_string(),
            detail: "Control.Fader(name, min, max, default, unit, pins, count)".to_string(),
            documentation: "Linear continuous fader control.".to_string(),
            insert_text:
                "Fader(name=\"${1:Level}\", min=${2:-100.0}, max=${3:12.0}, default=${4:0.0}, unit=\"dB\")"
                    .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Button".to_string(),
            kind: "Method".to_string(),
            detail: "Control.Button(name, type, pins, count)".to_string(),
            documentation: "Push button control (Momentary, Toggle, or Trigger).".to_string(),
            insert_text: "Button(name=\"${1:Mute}\", type=\"${2|Momentary,Toggle,Trigger|}\")"
                .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Text".to_string(),
            kind: "Method".to_string(),
            detail: "Control.Text(name, default, type, pins, count)".to_string(),
            documentation: "Text box display or entry control.".to_string(),
            insert_text: "Text(name=\"${1:Status}\", default=\"${2:Ready}\")".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Indicator".to_string(),
            kind: "Method".to_string(),
            detail: "Control.Indicator(name, unit, pins, count)".to_string(),
            documentation: "Read-only LED or status indicator control.".to_string(),
            insert_text: "Indicator(name=\"${1:SignalPresent}\", pins=\"Output\")".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "IPAddress".to_string(),
            kind: "Method".to_string(),
            detail: "Control.IPAddress(name, default, pins)".to_string(),
            documentation: "Pre-configured IP address text control.".to_string(),
            insert_text: "IPAddress(default=\"${1:255.255.255.255}\")".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "MACAddress".to_string(),
            kind: "Method".to_string(),
            detail: "Control.MACAddress(name, default, pins)".to_string(),
            documentation: "Pre-configured MAC address text control.".to_string(),
            insert_text: "MACAddress(default=\"${1:00:11:22:33:44:55}\")".to_string(),
            is_snippet: true,
        },
    ]
}

fn property_factory_completions() -> Vec<LspCompletion> {
    vec![
        LspCompletion {
            label: "Integer".to_string(),
            kind: "Method".to_string(),
            detail: "Property.Integer(name, default, min, max, description)".to_string(),
            documentation: "Integer property exposed in the Q-SYS Properties panel.".to_string(),
            insert_text: "Integer(name=\"${1:Port}\", default=${2:9}, min=${3:1}, max=${4:65535})"
                .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Float".to_string(),
            kind: "Method".to_string(),
            detail: "Property.Float(name, default, min, max, description)".to_string(),
            documentation: "Floating-point property exposed in the Q-SYS Properties panel."
                .to_string(),
            insert_text:
                "Float(name=\"${1:PollInterval}\", default=${2:1.0}, min=${3:0.1}, max=${4:60.0})"
                    .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "String".to_string(),
            kind: "Method".to_string(),
            detail: "Property.String(name, default, description)".to_string(),
            documentation: "String property exposed in the Q-SYS Properties panel.".to_string(),
            insert_text: "String(name=\"${1:Host}\", default=\"${2:192.168.1.100}\")".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Enum".to_string(),
            kind: "Method".to_string(),
            detail: "Property.Enum(name, choices, default, description)".to_string(),
            documentation: "Dropdown enumeration property exposed in the Q-SYS Properties panel."
                .to_string(),
            insert_text:
                "Enum(name=\"${1:Protocol}\", choices=[\"${2:UDP}\", \"${3:TCP}\"], default=\"${2:UDP}\")"
                    .to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Boolean".to_string(),
            kind: "Method".to_string(),
            detail: "Property.Boolean(name, default, description)".to_string(),
            documentation: "Boolean checkbox property exposed in the Q-SYS Properties panel."
                .to_string(),
            insert_text: "Boolean(name=\"${1:DebugMode}\", default=${2|False,True|})".to_string(),
            is_snippet: true,
        },
    ]
}

fn udp_socket_completions() -> Vec<LspCompletion> {
    vec![
        LspCompletion {
            label: "Open".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Open(ip?: str, port?: int) -> None".to_string(),
            documentation: "Opens the UDP socket (compiled to `sock:Open(...)`).".to_string(),
            insert_text: "Open(${0})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Send".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Send(ip: str, port: int, data: str) -> None".to_string(),
            documentation:
                "Sends a UDP datagram to `(ip, port)` (compiled to `sock:Send(ip, port, data)`)."
                    .to_string(),
            insert_text: "Send(${1:self.IPAddress.String}, ${2:9}, ${3:packet})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Close".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Close() -> None".to_string(),
            documentation: "Closes the UDP socket (compiled to `sock:Close()`).".to_string(),
            insert_text: "Close()".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "JoinMulticast".to_string(),
            kind: "Method".to_string(),
            detail: "sock.JoinMulticast(address: str, local_ip?: str) -> None".to_string(),
            documentation: "Joins a multicast group address.".to_string(),
            insert_text: "JoinMulticast(${1:\"224.0.0.1\"})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Data".to_string(),
            kind: "Event".to_string(),
            detail: "sock.Data: Callable[[UdpSocketHandle, UdpPacket], None]".to_string(),
            documentation: "Callback assigned to receive incoming UDP packets.".to_string(),
            insert_text: "Data".to_string(),
            is_snippet: false,
        },
    ]
}

fn tcp_socket_completions() -> Vec<LspCompletion> {
    vec![
        LspCompletion {
            label: "Connect".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Connect(ip: str, port: int) -> None".to_string(),
            documentation: "Initiates a TCP connection (compiled to `sock:Connect(ip, port)`)."
                .to_string(),
            insert_text: "Connect(${1:ip}, ${2:port})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Write".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Write(data: str) -> None".to_string(),
            documentation: "Writes data to the TCP socket (compiled to `sock:Write(data)`)."
                .to_string(),
            insert_text: "Write(${1:data})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Read".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Read(length: int) -> str".to_string(),
            documentation: "Reads up to `length` bytes from the TCP receive buffer.".to_string(),
            insert_text: "Read(${1:1024})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "ReadLine".to_string(),
            kind: "Method".to_string(),
            detail: "sock.ReadLine(eol: int) -> str | None".to_string(),
            documentation: "Reads a single line from the TCP receive buffer.".to_string(),
            insert_text: "ReadLine(${1:TcpSocket.EOL.Any})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Disconnect".to_string(),
            kind: "Method".to_string(),
            detail: "sock.Disconnect() -> None".to_string(),
            documentation: "Closes the active TCP connection.".to_string(),
            insert_text: "Disconnect()".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "IsConnected".to_string(),
            kind: "Property".to_string(),
            detail: "(property) sock.IsConnected: bool".to_string(),
            documentation: "True when the TCP socket is currently connected.".to_string(),
            insert_text: "IsConnected".to_string(),
            is_snippet: false,
        },
    ]
}

fn timer_completions() -> Vec<LspCompletion> {
    vec![
        LspCompletion {
            label: "Start".to_string(),
            kind: "Method".to_string(),
            detail: "timer.Start(period_seconds: float) -> None".to_string(),
            documentation: "Starts the Q-SYS timer (compiled to `timer:Start(period)`)."
                .to_string(),
            insert_text: "Start(${1:1.0})".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Stop".to_string(),
            kind: "Method".to_string(),
            detail: "timer.Stop() -> None".to_string(),
            documentation: "Stops the Q-SYS timer (compiled to `timer:Stop()`).".to_string(),
            insert_text: "Stop()".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "EventHandler".to_string(),
            kind: "Event".to_string(),
            detail: "timer.EventHandler: Callable[[TimerHandle], None]".to_string(),
            documentation: "Callback invoked every time the timer period elapses.".to_string(),
            insert_text: "EventHandler".to_string(),
            is_snippet: false,
        },
    ]
}

fn top_level_completions(
    controls: &[DeclaredControl],
    properties: &[DeclaredProperty],
    methods: &[ResolvedMethod],
) -> Vec<LspCompletion> {
    let mut items = vec![
        LspCompletion {
            label: "QSysPlugin".to_string(),
            kind: "Class".to_string(),
            detail: "class MyPlugin(QSysPlugin)".to_string(),
            documentation: "Base class for Q-SYS Designer `.qplug` plugins.".to_string(),
            insert_text: "class ${1:MyPlugin}(QSysPlugin):\n    version = \"1.0.0\"\n    author = \"${2:Engineering}\"\n\n    ${0:pass}".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "TextController".to_string(),
            kind: "Class".to_string(),
            detail: "class MyController(TextController)".to_string(),
            documentation: "Base class for Q-SYS Text Controller / Script components.".to_string(),
            insert_text: "class ${1:MyController}(TextController):\n    ${0:pass}".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "Control".to_string(),
            kind: "Class".to_string(),
            detail: "Q-SYS Control factory namespace".to_string(),
            documentation: "Declare hardware/UI controls (`Control.Float`, `Control.Button`, `Control.Text`, `Control.Indicator`, `Control.IPAddress`, `Control.MACAddress`).".to_string(),
            insert_text: "Control".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "Property".to_string(),
            kind: "Class".to_string(),
            detail: "Q-SYS Property factory namespace".to_string(),
            documentation: "Declare Q-SYS Properties panel settings (`Property.Integer`, `Property.Float`, `Property.String`, `Property.Enum`, `Property.Boolean`).".to_string(),
            insert_text: "Property".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "UdpSocket".to_string(),
            kind: "Class".to_string(),
            detail: "Q-SYS UdpSocket runtime API".to_string(),
            documentation: "Create UDP sockets via `UdpSocket.New()`.".to_string(),
            insert_text: "UdpSocket".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "TcpSocket".to_string(),
            kind: "Class".to_string(),
            detail: "Q-SYS TcpSocket runtime API".to_string(),
            documentation: "Create TCP client sockets via `TcpSocket.New()`.".to_string(),
            insert_text: "TcpSocket".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "Timer".to_string(),
            kind: "Class".to_string(),
            detail: "Q-SYS Timer runtime API".to_string(),
            documentation: "Create periodic or one-shot timers via `Timer.New()`.".to_string(),
            insert_text: "Timer".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "self".to_string(),
            kind: "Variable".to_string(),
            detail: "Plugin / Controller instance".to_string(),
            documentation: "Access declared controls, properties, and methods on `self`.".to_string(),
            insert_text: "self".to_string(),
            is_snippet: false,
        },
        LspCompletion {
            label: "def runtime(self):".to_string(),
            kind: "Snippet".to_string(),
            detail: "Q-SYS runtime entrypoint method".to_string(),
            documentation: "Invoked automatically inside `if Controls then` when the plugin starts on Q-SYS Core.".to_string(),
            insert_text: "def runtime(self):\n    ${0:pass}".to_string(),
            is_snippet: true,
        },
        LspCompletion {
            label: "def get_control_layout(self, props):".to_string(),
            kind: "Snippet".to_string(),
            detail: "Q-SYS GetControlLayout(props) lifecycle hook".to_string(),
            documentation: "Customizes control positions and graphics in Q-SYS Designer.".to_string(),
            insert_text: "def get_control_layout(self, props):\n    layout = {}\n    graphics = []\n    ${0}\n    return layout, graphics".to_string(),
            is_snippet: true,
        },
    ];

    for ctl in controls {
        items.push(LspCompletion {
            label: format!("self.{}", ctl.ident),
            kind: "Field".to_string(),
            detail: format!("Controls[\"{}\"]", ctl.def.name),
            documentation: format!("Access control `{}` on `self`.", ctl.ident),
            insert_text: format!("self.{}", ctl.ident),
            is_snippet: false,
        });
    }

    for prop in properties {
        items.push(LspCompletion {
            label: format!("self.{}", prop.ident),
            kind: "Property".to_string(),
            detail: format!("Properties[\"{}\"].Value", prop.def.name),
            documentation: format!("Access property `{}` on `self`.", prop.ident),
            insert_text: format!("self.{}", prop.ident),
            is_snippet: false,
        });
    }

    for m in methods {
        items.push(LspCompletion {
            label: format!("self.{}()", m.name),
            kind: "Method".to_string(),
            detail: format!("Call method `{}`", m.name),
            documentation: format!("Invoke `self.{}`.", m.name),
            insert_text: format!("self.{}()", m.name),
            is_snippet: false,
        });
    }

    items
}

fn control_member_doc(name: &str) -> (&'static str, &'static str) {
    match name {
        "Value" => ("float", "Current continuous numeric value of the control."),
        "String" => (
            "str",
            "Formatted text string representation of the control value.",
        ),
        "Boolean" => ("bool", "Binary state of the control (True / False)."),
        "Values" => (
            "list[float]",
            "Array of numeric values for multi-channel controls.",
        ),
        "Strings" => (
            "list[str]",
            "Array of text strings for multi-channel controls.",
        ),
        "EventHandler" => (
            "Callable[..., Any]",
            "Event callback binding for value changes.",
        ),
        "IsDisabled" => ("bool", "Whether user interaction is disabled in the UI."),
        "IsInvisible" => ("bool", "Whether the control widget is hidden in the UI."),
        "Color" => ("str", "Hex color or named color of the control widget."),
        "Legend" => ("str", "Text label displayed on the control widget."),
        "RampTime" => (
            "float",
            "Time in seconds over which the control ramps to new values.",
        ),
        _ => ("Any", "Control property."),
    }
}

fn builtin_symbol_doc(word: &str) -> Option<(&'static str, &'static str)> {
    match word {
        "QSysPlugin" => Some((
            "*(class)* QSysPlugin",
            "Base class for Q-SYS Designer `.qplug` plugins. Compiles into `PluginInfo`, `GetProperties`, `GetControls`, `GetControlLayout`, and the `if Controls then` runtime block.",
        )),
        "TextController" => Some((
            "*(class)* TextController",
            "Base class for Q-SYS Text Controller / Script components.",
        )),
        "Control" => Some((
            "*(namespace)* Control",
            "Factory declarations for Q-SYS controls (`Float`, `Knob`, `Fader`, `Button`, `Text`, `Indicator`, `IPAddress`, `MACAddress`).",
        )),
        "Property" => Some((
            "*(namespace)* Property",
            "Factory declarations for Q-SYS Designer properties (`Integer`, `Float`, `Double`, `String`, `Enum`, `Boolean`).",
        )),
        "ControlHandle" => Some((
            "*(class)* ControlHandle",
            "Runtime handle to a Q-SYS `Controls[...]` object providing `.Value`, `.String`, `.Boolean`, `.Legend`, `.Color`, `.IsDisabled`, `.IsInvisible`, and `.RampTime`.",
        )),
        "UdpSocket" => Some((
            "*(class)* UdpSocket",
            "Q-SYS native UDP networking class. Instantiate via `UdpSocket.New()`.",
        )),
        "TcpSocket" => Some((
            "*(class)* TcpSocket",
            "Q-SYS native TCP client networking class. Instantiate via `TcpSocket.New()`.",
        )),
        "Timer" => Some((
            "*(class)* Timer",
            "Q-SYS native event loop timer. Instantiate via `Timer.New()`.",
        )),
        "Open" => Some((
            "*(method)* UdpSocketHandle.Open(ip?: str, port?: int) -> None",
            "Binds and opens the UDP socket (`sock:Open(...)` in Lua).",
        )),
        "Send" => Some((
            "*(method)* UdpSocketHandle.Send(ip: str, port: int, data: str) -> None",
            "Sends a UDP packet to the target IP and port (`sock:Send(ip, port, data)` in Lua).",
        )),
        "Close" => Some((
            "*(method)* UdpSocketHandle.Close() -> None",
            "Closes the UDP socket (`sock:Close()` in Lua).",
        )),
        "Connect" => Some((
            "*(method)* TcpSocketHandle.Connect(ip: str, port: int) -> None",
            "Connects the TCP socket to the target host and port (`sock:Connect(ip, port)` in Lua).",
        )),
        "Write" => Some((
            "*(method)* TcpSocketHandle.Write(data: str) -> None",
            "Writes string or binary payload over the TCP socket (`sock:Write(data)` in Lua).",
        )),
        "Start" => Some((
            "*(method)* TimerHandle.Start(seconds: float) -> None",
            "Starts or restarts the Q-SYS timer with the given interval in seconds (`timer:Start(seconds)` in Lua).",
        )),
        "Stop" => Some((
            "*(method)* TimerHandle.Stop() -> None",
            "Stops the Q-SYS timer (`timer:Stop()` in Lua).",
        )),
        _ => None,
    }
}

fn builtin_signature_help(callee: &str, active_parameter: usize) -> Option<LspSignatureHelp> {
    let (label, doc, params): (&str, &str, &[(&str, &str)]) = match callee {
        "Control.Float" | "Control.Knob" | "Control.Fader" => (
            "Control.Float(name, min=-100.0, max=20.0, default=0.0, unit=\"Float\", pins=\"Both\", count=1)",
            "Declares a continuous numeric Q-SYS control (Knob or Fader).",
            &[
                ("name: str", "Hardware control name in `Controls[name]`."),
                ("min: float", "Minimum numeric value."),
                ("max: float", "Maximum numeric value."),
                ("default: float", "Initial default value."),
                (
                    "unit: str",
                    "Engineering unit (`\"Float\"`, `\"Integer\"`, `\"dB\"`, `\"Hz\"`, `\"Percent\"`, `\"Seconds\"`, `\"Pan\"`).",
                ),
                (
                    "pins: str",
                    "Pin exposure (`\"Both\"`, `\"Input\"`, `\"Output\"`, `\"None\"`).",
                ),
                ("count: int", "Number of control channels (default 1)."),
            ],
        ),
        "Control.Button" => (
            "Control.Button(name, type=\"Momentary\", pins=\"Both\", count=1)",
            "Declares a Q-SYS Button control.",
            &[
                ("name: str", "Hardware control name in `Controls[name]`."),
                (
                    "type: str",
                    "Button behavior (`\"Momentary\"`, `\"Toggle\"`, or `\"Trigger\"`).",
                ),
                (
                    "pins: str",
                    "Pin exposure (`\"Both\"`, `\"Input\"`, `\"Output\"`, `\"None\"`).",
                ),
                ("count: int", "Number of button channels (default 1)."),
            ],
        ),
        "Control.Text" => (
            "Control.Text(name, default=\"\", type=\"Text\", pins=\"Both\", count=1)",
            "Declares a Q-SYS Text box control.",
            &[
                ("name: str", "Hardware control name in `Controls[name]`."),
                ("default: str", "Initial default string."),
                ("type: str", "Text box mode (`\"Text\"` or `\"ReadOnly\"`)."),
                (
                    "pins: str",
                    "Pin exposure (`\"Both\"`, `\"Input\"`, `\"Output\"`, `\"None\"`).",
                ),
                ("count: int", "Number of text channels (default 1)."),
            ],
        ),
        "Control.Indicator" => (
            "Control.Indicator(name, unit=\"Float\", pins=\"Output\", count=1)",
            "Declares a Q-SYS status Indicator / LED / Meter control.",
            &[
                ("name: str", "Hardware control name in `Controls[name]`."),
                ("unit: str", "Indicator engineering unit."),
                (
                    "pins: str",
                    "Pin exposure (`\"Both\"`, `\"Input\"`, `\"Output\"`, `\"None\"`).",
                ),
                ("count: int", "Number of indicator channels (default 1)."),
            ],
        ),
        "Control.IPAddress" | "Control.MACAddress" => (
            "Control.IPAddress(name, default=\"\", pins=\"Both\")",
            "Declares a pre-configured network address Text control.",
            &[
                (
                    "name: str",
                    "Hardware control name (defaults to attribute name).",
                ),
                ("default: str", "Default network address string."),
                (
                    "pins: str",
                    "Pin exposure (`\"Both\"`, `\"Input\"`, `\"Output\"`, `\"None\"`).",
                ),
            ],
        ),
        "Property.Integer" | "Property.Float" | "Property.Double" => (
            "Property.Integer(name, default=0, min=None, max=None, description=None)",
            "Declares a numeric property in the Q-SYS Properties inspector.",
            &[
                ("name: str", "Inspector display name."),
                ("default: int | float", "Default property value."),
                ("min: int | float", "Minimum allowed value."),
                ("max: int | float", "Maximum allowed value."),
                ("description: str", "Tooltip description in Designer."),
            ],
        ),
        "Property.String" => (
            "Property.String(name, default=\"\", description=None)",
            "Declares a string property in the Q-SYS Properties inspector.",
            &[
                ("name: str", "Inspector display name."),
                ("default: str", "Default string value."),
                ("description: str", "Tooltip description in Designer."),
            ],
        ),
        "Property.Enum" => (
            "Property.Enum(name, choices=[...], default=\"\", description=None)",
            "Declares a dropdown enumeration property in the Q-SYS Properties inspector.",
            &[
                ("name: str", "Inspector display name."),
                ("choices: list[str]", "List of selectable string options."),
                ("default: str", "Initially selected option."),
                ("description: str", "Tooltip description in Designer."),
            ],
        ),
        "Property.Boolean" => (
            "Property.Boolean(name, default=False, description=None)",
            "Declares a boolean property in the Q-SYS Properties inspector.",
            &[
                ("name: str", "Inspector display name."),
                ("default: bool", "Default boolean state."),
                ("description: str", "Tooltip description in Designer."),
            ],
        ),
        "qsys.lua" => (
            "qsys.lua(code: str)",
            "Injects raw Lua 5.3 source directly into the compiled output.",
            &[("code: str", "Raw Lua 5.3 string literal.")],
        ),
        s if s.ends_with(".Send") => (
            "sock.Send(ip: str, port: int, data: str)",
            "Transmits a UDP datagram over the Q-SYS UdpSocket (`sock:Send(ip, port, data)`).",
            &[
                ("ip: str", "Destination IPv4/IPv6 or broadcast address."),
                ("port: int", "Destination UDP port number (1-65535)."),
                ("data: str", "Binary or text payload to send."),
            ],
        ),
        s if s.ends_with(".Open") => (
            "sock.Open(ip?: str, port?: int)",
            "Binds and opens the UDP socket (`sock:Open(ip, port)`).",
            &[
                ("ip?: str", "Optional local interface IP to bind."),
                ("port?: int", "Optional local UDP port to bind."),
            ],
        ),
        s if s.ends_with(".Connect") => (
            "sock.Connect(ip: str, port: int)",
            "Opens a TCP connection to `(ip, port)` (`sock:Connect(ip, port)`).",
            &[
                ("ip: str", "Remote host IP or hostname."),
                ("port: int", "Remote TCP port."),
            ],
        ),
        s if s.ends_with(".Write") => (
            "sock.Write(data: str)",
            "Writes data to the connected TCP socket (`sock:Write(data)`).",
            &[("data: str", "String or binary payload to transmit.")],
        ),
        s if s.ends_with(".Start") => (
            "timer.Start(seconds: float)",
            "Starts the Q-SYS timer with the specified interval in seconds (`timer:Start(seconds)`).",
            &[("seconds: float", "Timer interval in seconds.")],
        ),
        _ => return None,
    };

    Some(LspSignatureHelp {
        label: label.to_string(),
        documentation: doc.to_string(),
        parameters: params
            .iter()
            .map(|(l, d)| LspParameterInfo {
                label: (*l).to_string(),
                documentation: (*d).to_string(),
            })
            .collect(),
        active_parameter: active_parameter.min(params.len().saturating_sub(1)),
    })
}

fn find_enclosing_call(src: &str, cursor_offset: usize) -> Option<(String, usize)> {
    let safe_offset = cursor_offset.min(src.len());
    let bytes = &src.as_bytes()[..safe_offset];

    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut in_string: Option<u8> = None;

    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        let b = bytes[i];

        if let Some(q) = in_string {
            if b == q && (i == 0 || bytes[i - 1] != b'\\') {
                in_string = None;
            }
            continue;
        }

        match b {
            b'"' | b'\'' => in_string = Some(b),
            b')' | b']' | b'}' => depth += 1,
            b',' if depth == 0 => commas += 1,
            b'(' => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    let end = i;
                    let mut start = end;
                    while start > 0
                        && (bytes[start - 1].is_ascii_alphanumeric()
                            || bytes[start - 1] == b'_'
                            || bytes[start - 1] == b'.')
                    {
                        start -= 1;
                    }
                    if start < end {
                        return Some((src[start..end].to_string(), commas));
                    }
                    return None;
                }
            }
            b'\n' if depth == 0 => break,
            _ => {}
        }
    }
    None
}

/// Identifies the exact word boundaries around `offset`.
fn get_word_at_offset(src: &str, offset: usize) -> Option<(&str, usize, usize)> {
    if src.is_empty() {
        return None;
    }
    let safe_offset = offset.min(src.len());
    let bytes = src.as_bytes();

    let mut idx = safe_offset;
    if idx >= bytes.len() || (!bytes[idx].is_ascii_alphanumeric() && bytes[idx] != b'_') {
        if idx > 0 && (bytes[idx - 1].is_ascii_alphanumeric() || bytes[idx - 1] == b'_') {
            idx -= 1;
        } else {
            return None;
        }
    }

    let mut start = idx;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }

    let mut end = idx;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }

    if start < end {
        Some((&src[start..end], start, end))
    } else {
        None
    }
}

fn get_prefix_before_cursor(src: &str, offset: usize) -> &str {
    let safe_offset = offset.min(src.len());
    let slice = &src[..safe_offset];
    let start = slice
        .rfind(['\n', '\r', ' ', '\t'])
        .map(|idx| idx + 1)
        .unwrap_or(0);
    &slice[start..]
}

fn get_line_before_cursor(src: &str, offset: usize) -> &str {
    let safe_offset = offset.min(src.len());
    let slice = &src[..safe_offset];
    let start = slice.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    &slice[start..]
}
