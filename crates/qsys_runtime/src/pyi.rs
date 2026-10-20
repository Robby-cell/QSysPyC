//! Renders a [`RuntimeEnv`] as PEP 484 stub (`.pyi`) source, so editor tooling always
//! matches what the compiler accepts.

use std::fmt::Write as _;

use crate::defs::{CallStyle, Function, GlobalKind, ObjectType, TypeRef};
use crate::env::RuntimeEnv;

/// Renders the object types and globals whose `origin` plugin satisfies `include`.
///
/// The output references `Any` and `Callable`; the caller is responsible for importing them
/// from `typing`.
pub fn render_pyi(env: &RuntimeEnv, include: impl Fn(&str) -> bool) -> String {
    let mut out = String::new();

    for ty in env.object_types().iter().filter(|t| include(&t.origin)) {
        render_class(&mut out, ty);
    }

    for global in env.globals().iter().filter(|g| include(&g.origin)) {
        match &global.kind {
            GlobalKind::Value(TypeRef::Object(type_name))
                if *type_name == global.name && env.object_type(type_name).is_some() =>
            {
                // The class rendered above doubles as the global.
            }
            GlobalKind::Value(ty) => {
                let _ = writeln!(out, "{}: {}", global.name, ty.python_annotation());
                write_attribute_doc(&mut out, "", &global.doc);
            }
            GlobalKind::Function(function) => {
                render_function(&mut out, "", function, false);
                out.push('\n');
            }
        }
    }

    out
}

fn render_class(out: &mut String, ty: &ObjectType) {
    let _ = writeln!(out, "class {}:", ty.name);
    let mut has_body = false;

    if !ty.doc.is_empty() {
        let _ = writeln!(out, "    \"\"\"{}\"\"\"", escape_doc(&ty.doc));
        has_body = true;
    }

    for field in &ty.fields {
        let _ = writeln!(out, "    {}: {}", field.name, field.ty.python_annotation());
        write_attribute_doc(out, "    ", &field.doc);
        has_body = true;
    }

    for method in &ty.methods {
        let is_static = method.call_style == CallStyle::Dot;
        if is_static {
            out.push_str("    @staticmethod\n");
        }
        render_function(out, "    ", method, !is_static);
        has_body = true;
    }

    if !has_body {
        out.push_str("    ...\n");
    }
    out.push('\n');
}

fn render_function(out: &mut String, indent: &str, function: &Function, with_self: bool) {
    let mut params: Vec<String> = Vec::new();
    if with_self {
        params.push("self".to_string());
    }
    for param in &function.params {
        let default = if param.optional { " = ..." } else { "" };
        params.push(format!(
            "{}: {}{default}",
            param.name,
            param.ty.python_annotation()
        ));
    }
    if function.variadic {
        params.push("*args: Any".to_string());
    }

    let _ = writeln!(
        out,
        "{indent}def {}({}) -> {}:",
        function.name,
        params.join(", "),
        function.return_annotation()
    );
    if !function.doc.is_empty() {
        let _ = writeln!(out, "{indent}    \"\"\"{}\"\"\"", escape_doc(&function.doc));
    }
    let _ = writeln!(out, "{indent}    ...");
}

fn write_attribute_doc(out: &mut String, indent: &str, doc: &str) {
    if !doc.is_empty() {
        let _ = writeln!(out, "{indent}\"\"\"{}\"\"\"", escape_doc(doc));
    }
}

fn escape_doc(doc: &str) -> String {
    doc.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"")
}
