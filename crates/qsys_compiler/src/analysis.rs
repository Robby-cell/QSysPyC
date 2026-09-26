use qsys_diagnostics::{FileId, SourceFile};
use qsys_model::ControlMember;
use qsys_parser::parse_module;
use qsys_sema::{TargetKind, Type, analyze};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LspHover {
    pub contents: Vec<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LspCompletion {
    pub label: String,
    pub kind: String,
    pub detail: String,
    pub documentation: String,
    pub insert_text: String,
    pub is_snippet: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LspInlayHint {
    pub line: usize,
    pub col: usize,
    pub label: String,
}

pub struct Analyzer;

impl Analyzer {
    /// Queries the Rust semantic model to produce hover documentation at (line, col).
    pub fn hover(source: &str, line: usize, col: usize) -> Option<LspHover> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let parsed = parse_module(file.clone()).ok()?;
        let model = analyze(&parsed).ok()?;

        let cursor_offset = file.offset(line, col);
        let (word, word_start, word_end) = get_word_at_offset(&file.src, cursor_offset)?;

        let (start_line, start_col) = file.location(word_start);
        let (end_line, end_col) = file.location(word_end);

        // Search for control references (self.Gain or Gain)
        if let TargetKind::Plugin(ref plugin) = model.target {
            for ctl in &plugin.controls {
                if word == ctl.ident {
                    return Some(LspHover {
                        contents: vec![
                            format!("*(control)* {}: ControlHandle", ctl.ident),
                            format!(
                                "**Q-SYS Configuration:**\n- **Hardware Name:** `\"{}\"`\n- **Widget Type:** `{}`\n- **Pins:** `{}`",
                                ctl.def.name,
                                ctl.def.kind.qsys_type_str(),
                                ctl.def.pin_style.as_str()
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

        // Search for runtime ControlMember properties (.Value, .String, .Boolean, etc.)
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

        // Search TypeMap for inferred variable types at the cursor span
        for (&span, ty) in model.type_map.types() {
            if cursor_offset >= span.start && cursor_offset <= span.end {
                let (s_line, s_col) = file.location(span.start);
                let (e_line, e_col) = file.location(span.end);
                let ty_desc = match ty {
                    Type::ControlHandle => "ControlHandle".to_string(),
                    Type::Float => "float".to_string(),
                    Type::Int => "int".to_string(),
                    Type::String => "str".to_string(),
                    Type::Bool => "bool".to_string(),
                    Type::List(inner) => format!("list[{inner:?}]"),
                    _ => continue,
                };

                return Some(LspHover {
                    contents: vec![format!("*(type)* {ty_desc}")],
                    start_line: s_line,
                    start_col: s_col,
                    end_line: e_line,
                    end_col: e_col,
                });
            }
        }

        None
    }

    /// Queries the Rust compiler AST & symbol tables for context-aware completions.
    pub fn completions(source: &str, line: usize, col: usize) -> Vec<LspCompletion> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let cursor_offset = file.offset(line, col);

        let prefix = get_prefix_before_cursor(&file.src, cursor_offset);
        let mut items = Vec::new();

        let parsed = parse_module(file.clone()).ok();
        let model = parsed.as_ref().and_then(|p| analyze(p).ok());

        // A. If after `self.` -> Suggest declared controls directly from the Rust Semantic Model
        if prefix.ends_with("self.") {
            if let Some(ref m) = model
                && let TargetKind::Plugin(ref plugin) = m.target
            {
                for ctl in &plugin.controls {
                    items.push(LspCompletion {
                        label: ctl.ident.clone(),
                        kind: "Field".to_string(),
                        detail: format!("(control) {}: ControlHandle", ctl.ident),
                        documentation: format!(
                            "Q-SYS Hardware Control\nHardware Name: \"{}\"\nType: {}",
                            ctl.def.name,
                            ctl.def.kind.qsys_type_str()
                        ),
                        insert_text: ctl.ident.clone(),
                        is_snippet: false,
                    });
                }
            }
            return items;
        }

        // B. If after `@` -> Suggest `@<Control>.EventHandler` from the Rust Semantic Model
        if prefix.ends_with('@') || prefix.trim_end().ends_with('@') {
            if let Some(ref m) = model
                && let TargetKind::Plugin(ref plugin) = m.target
            {
                for ctl in &plugin.controls {
                    items.push(LspCompletion {
                            label: format!("{}.EventHandler", ctl.ident),
                            kind: "Event".to_string(),
                            detail: format!("Event handler for {}", ctl.ident),
                            documentation: format!("Binds a handler method to changes on '{}'", ctl.def.name),
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

        // C. If after `Control.` -> Suggest factory methods from Rust qsys_model
        if prefix.ends_with("Control.") {
            items.push(LspCompletion {
                label: "Float".to_string(),
                kind: "Method".to_string(),
                detail: "Control.Float(name, min, max, default, pins, count)".to_string(),
                documentation: "Continuous floating-point knob or fader.".to_string(),
                insert_text:
                    "Float(name=\"${1:Gain}\", min=${2:-100.0}, max=${3:20.0}, default=${4:0.0})"
                        .to_string(),
                is_snippet: true,
            });
            items.push(LspCompletion {
                label: "Button".to_string(),
                kind: "Method".to_string(),
                detail: "Control.Button(name, type, pins, count)".to_string(),
                documentation: "Push button (Momentary, Toggle, or Trigger).".to_string(),
                insert_text: "Button(name=\"${1:Mute}\", type=\"${2|Momentary,Toggle,Trigger|}\")"
                    .to_string(),
                is_snippet: true,
            });
            items.push(LspCompletion {
                label: "Text".to_string(),
                kind: "Method".to_string(),
                detail: "Control.Text(name, default, pins)".to_string(),
                documentation: "Text box display or entry.".to_string(),
                insert_text: "Text(name=\"${1:Status}\", default=\"${2:Ready}\")".to_string(),
                is_snippet: true,
            });
            return items;
        }

        // D. If after `.` -> Suggest ControlMember runtime attributes from qsys_model
        if prefix.ends_with('.') {
            for &member_name in ControlMember::all_member_names() {
                if member_name == "EventHandler" {
                    continue;
                }
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
            return items;
        }

        // Default top-level keywords
        items.push(LspCompletion {
            label: "QSysPlugin".to_string(),
            kind: "Class".to_string(),
            detail: "class MyPlugin(QSysPlugin)".to_string(),
            documentation: "Base class for all Q-SYS Plugins.".to_string(),
            insert_text: "class ${1:MyPlugin}(QSysPlugin):\n    version = \"1.0.0\"\n    author = \"${2:Engineering}\"\n    \n    ${0}".to_string(),
            is_snippet: true,
        });

        items
    }

    /// Queries the Rust TypeMap to produce accurate inline type hints.
    pub fn inlay_hints(source: &str) -> Vec<LspInlayHint> {
        let file = SourceFile::new(FileId(1), "input.py", source);
        let parsed = match parse_module(file.clone()) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        let model = match analyze(&parsed) {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };

        let mut hints = Vec::new();

        if let TargetKind::Plugin(ref plugin) = model.target {
            for method in &plugin.methods {
                for (_sym_id, name) in &method.params {
                    if name == "ctl" {
                        let (line, col) = file.location(method.span.start);
                        hints.push(LspInlayHint {
                            line,
                            col: col + name.len() + 1,
                            label: ": ControlHandle".to_string(),
                        });
                    }
                }
            }
        }

        hints
    }
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
        .rfind(['\n', ' ', '\t'])
        .map(|idx| idx + 1)
        .unwrap_or(0);
    &slice[start..]
}
