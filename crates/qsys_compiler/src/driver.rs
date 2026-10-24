use std::sync::Arc;

use qsys_codegen_lua::LuaBackend;
use qsys_diagnostics::{DiagnosticKind, Diagnostics, FileId, SourceFile, Span};
use qsys_ir::{Backend, CodegenConfig};
use qsys_model::{
    ControlKind, ControlStyle, FlexDirection, LabelPlacement, UiControlSpec, UiNode, UiPageSpec,
};
use qsys_parser::parse_module;
use qsys_runtime::RuntimeEnv;
use qsys_sema::{DeclaredControl, TargetKind, analyze_with_env};

use crate::options::{
    CheckResult, CompiledArtifact, CompilerOptions, ProgramKind, UiControlMetadata,
    UiPreviewPayload,
};

pub struct Compiler;

impl Compiler {
    /// Compiles raw Python source text into a ready-to-deploy Q-SYS Lua artifact.
    pub fn compile(
        source: &str,
        options: &CompilerOptions,
    ) -> Result<CompiledArtifact, Diagnostics> {
        let file = SourceFile::new(FileId(1), &options.filename, source);
        Self::compile_file(file, options)
    }

    /// Compiles an in-memory `SourceFile` into a Q-SYS Lua artifact.
    pub fn compile_file(
        file: Arc<SourceFile>,
        options: &CompilerOptions,
    ) -> Result<CompiledArtifact, Diagnostics> {
        let mut all_diagnostics = Diagnostics::new();

        // Parsing (Python text -> AST)
        let parsed = parse_module(file.clone())?;

        // Semantic Analysis (AST -> Semantic Model with stable IDs)
        let model = analyze_with_env(&parsed, options.env.clone())?;

        // Extract metadata for reports & CLI summaries
        let (program_kind, plugin_info, control_count, property_count, ui_preview) =
            match &model.target {
                TargetKind::Plugin(p) => (
                    ProgramKind::Plugin,
                    Some(p.info.clone()),
                    p.controls.len(),
                    p.properties.len(),
                    build_ui_preview("Plugin", &p.info.name, &p.controls, &p.pages),
                ),
                TargetKind::TextController(c) => (
                    ProgramKind::TextController,
                    None,
                    c.controls.len(),
                    0,
                    build_ui_preview("TextController", &c.class_name, &c.controls, &[]),
                ),
                TargetKind::ProceduralScript(_) => (
                    ProgramKind::ProceduralScript,
                    None,
                    0,
                    0,
                    UiPreviewPayload {
                        program_kind: "ProceduralScript".to_string(),
                        ..UiPreviewPayload::default()
                    },
                ),
            };

        // Lowering (Semantic Model -> Typed IR)
        let ir = model.into_ir();

        // Code Generation (IR -> Q-SYS Lua 5.3)
        let backend = LuaBackend::new();
        let codegen_config = CodegenConfig {
            emit_source_maps: options.emit_source_maps,
            minify: options.minify,
            target_name: options.target_name.clone().unwrap_or_else(|| {
                plugin_info
                    .as_ref()
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "main".to_string())
            }),
        };

        let (lua, codegen_diags) = match backend.emit(&ir, &codegen_config) {
            Ok(res) => res,
            Err(err) => {
                all_diagnostics.report(
                    DiagnosticKind::syntax_error(format!("Codegen error: {err}")),
                    Span::empty(),
                );
                return Err(all_diagnostics);
            }
        };

        all_diagnostics.items.extend(codegen_diags.items);

        Ok(CompiledArtifact {
            file,
            lua,
            program_kind,
            diagnostics: all_diagnostics,
            plugin_info,
            control_count,
            property_count,
            ui_preview,
        })
    }

    /// Checks source code syntax and semantics without performing code generation.
    pub fn check(source: &str, filename: impl Into<String>) -> Result<CheckResult, Diagnostics> {
        let file = SourceFile::new(FileId(1), filename.into(), source);
        Self::check_file(file)
    }

    /// Checks an in-memory `SourceFile` against the default Q-SYS runtime environment.
    pub fn check_file(file: Arc<SourceFile>) -> Result<CheckResult, Diagnostics> {
        Self::check_file_with_env(file, RuntimeEnv::qsys())
    }

    /// Checks an in-memory `SourceFile` against a custom runtime environment.
    pub fn check_file_with_env(
        file: Arc<SourceFile>,
        env: Arc<RuntimeEnv>,
    ) -> Result<CheckResult, Diagnostics> {
        let parsed = parse_module(file.clone())?;
        let model = analyze_with_env(&parsed, env)?;

        let (program_kind, plugin_info, control_count, property_count) = match &model.target {
            TargetKind::Plugin(p) => (
                ProgramKind::Plugin,
                Some(p.info.clone()),
                p.controls.len(),
                p.properties.len(),
            ),
            TargetKind::TextController(c) => {
                (ProgramKind::TextController, None, c.controls.len(), 0)
            }
            TargetKind::ProceduralScript(_) => (ProgramKind::ProceduralScript, None, 0, 0),
        };

        Ok(CheckResult {
            file,
            program_kind,
            diagnostics: Diagnostics::new(),
            plugin_info,
            control_count,
            property_count,
        })
    }
}

fn build_ui_preview(
    program_kind: &str,
    title: &str,
    controls: &[DeclaredControl],
    explicit_pages: &[qsys_model::PageLayout],
) -> UiPreviewPayload {
    let ctl_meta: Vec<UiControlMetadata> = controls
        .iter()
        .map(|c| {
            let (unit, min, max, default_num, default_str, button_type) = match &c.def.kind {
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
                } => (
                    Some(unit.as_str().to_string()),
                    Some(*min),
                    Some(*max),
                    Some(*default),
                    None,
                    None,
                ),
                ControlKind::Button { button_type } => (
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(button_type.as_str().to_string()),
                ),
                ControlKind::Text { default, .. } => {
                    (None, None, None, None, Some(default.clone()), None)
                }
                ControlKind::Indicator { unit } => (
                    Some(unit.as_str().to_string()),
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            };
            UiControlMetadata {
                ident: c.ident.clone(),
                name: c.def.name.clone(),
                kind: c.def.kind.qsys_type_str().to_string(),
                unit,
                min,
                max,
                default_num,
                default_str,
                button_type,
                count: c.def.count,
            }
        })
        .collect();

    if !explicit_pages.is_empty() {
        return UiPreviewPayload {
            program_kind: program_kind.to_string(),
            plugin_name: title.to_string(),
            has_explicit_layout: true,
            pages: explicit_pages.to_vec(),
            controls: ctl_meta,
        };
    }

    if controls.is_empty() && program_kind != "Plugin" {
        return UiPreviewPayload {
            program_kind: program_kind.to_string(),
            plugin_name: title.to_string(),
            has_explicit_layout: false,
            pages: Vec::new(),
            controls: ctl_meta,
        };
    }

    // Build an automatic preview page when the plugin hasn't defined `layout = UI.Page(...)` yet
    let mut group_children = Vec::new();
    for ctl in controls {
        let style = ControlStyle::default_for_kind(&ctl.def.kind);
        let type_property = ctl
            .def
            .generic_spec
            .as_ref()
            .map(|g| g.property_name.clone());
        if ctl.def.count <= 1 {
            group_children.push(UiNode::Control(UiControlSpec {
                control_name: ctl.def.name.clone(),
                style,
                label: Some(ctl.def.name.clone()),
                label_placement: LabelPlacement::Left { width: 100.0 },
                pretty_name: Some(ctl.def.name.clone()),
                legend: None,
                width: None,
                height: None,
                color: None,
                margin: None,
                read_only: false,
                invisible: false,
                type_property: type_property.clone(),
                count_property: ctl.def.count_property.clone(),
                base_control_name: None,
                channel_index: None,
                dynamic_label_template: None,
                dynamic_pretty_template: None,
                dynamic_legend_template: None,
                label_lua_expr: None,
                pretty_lua_expr: None,
                is_dynamic_repeat_head: false,
                skip_lua_emit: false,
            }));
        } else {
            let mut row_items = Vec::new();
            for ch in 1..=ctl.def.count {
                row_items.push(UiNode::Control(UiControlSpec {
                    control_name: format!("{} {}", ctl.def.name, ch),
                    style: style.clone(),
                    label: Some(format!("{} {}", ctl.ident, ch)),
                    label_placement: LabelPlacement::Top,
                    pretty_name: Some(format!("{} {}", ctl.def.name, ch)),
                    legend: None,
                    width: None,
                    height: None,
                    color: None,
                    margin: None,
                    read_only: false,
                    invisible: false,
                    type_property: type_property.clone(),
                    count_property: ctl.def.count_property.clone(),
                    base_control_name: Some(ctl.def.name.clone()),
                    channel_index: Some(ch),
                    dynamic_label_template: None,
                    dynamic_pretty_template: None,
                    dynamic_legend_template: None,
                    label_lua_expr: None,
                    pretty_lua_expr: None,
                    is_dynamic_repeat_head: false,
                    skip_lua_emit: false,
                }));
            }
            group_children.push(UiNode::Row {
                children: row_items,
                gap: 8.0,
                padding: 0.0,
            });
        }
    }

    let auto_page = UiPageSpec {
        name: "Main".to_string(),
        padding: 16.0,
        gap: 12.0,
        width: None,
        children: vec![
            UiNode::Header {
                text: title.to_string(),
                width: None,
                height: 28.0,
                font_size: 14,
                color: None,
            },
            UiNode::GroupBox {
                title: "Controls".to_string(),
                children: group_children,
                direction: FlexDirection::Column,
                gap: 10.0,
                padding: 12.0,
                width: None,
                height: None,
                color: None,
                stroke: 1,
            },
        ],
    };

    UiPreviewPayload {
        program_kind: program_kind.to_string(),
        plugin_name: title.to_string(),
        has_explicit_layout: false,
        pages: vec![auto_page.solve()],
        controls: ctl_meta,
    }
}
