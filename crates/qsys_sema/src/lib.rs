pub mod analyzer;
pub mod binding;
pub mod classifier;
pub mod declarations;
pub mod lower;
pub mod symbols;
pub mod types;
pub mod ui_layout;

use std::sync::Arc;

use rustc_hash::FxHashMap;

use qsys_diagnostics::{Diagnostics, SourceFile, Span};
use qsys_ir::{FunctionId, SymbolId};
use qsys_model::{PageLayout, PluginInfo, is_design_time_method};
use qsys_parser::{ParsedModule, ast};
use qsys_runtime::{ExecutionContext, RuntimeEnv};

use crate::analyzer::ExprAnalyzer;
pub use crate::binding::{ResolvedBinding, resolve_event_decorator};
pub use crate::classifier::{TargetClassification, classify_module};
pub use crate::declarations::{
    ConstValue, DeclaredControl, DeclaredProperty, ExtractedDeclarations, eval_constant,
    extract_class_declarations,
};
pub use crate::lower::lower_to_ir;
pub use crate::symbols::{ScopeKind, ScopeStack, Symbol, SymbolKind};
pub use crate::types::{ResolvedTarget, TruthinessStrategy, Type, TypeMap};

impl SemanticModel {
    pub fn into_ir(&self) -> qsys_ir::IRProgram {
        lower_to_ir(self)
    }
}

#[derive(Debug, Clone)]
pub enum TargetKind {
    Plugin(PluginSemantic),
    TextController(ControllerSemantic),
    ProceduralScript(ScriptSemantic),
}

#[derive(Debug, Clone)]
pub struct SemanticModel {
    pub file: Arc<SourceFile>,
    pub target: TargetKind,
    pub type_map: TypeMap,
    /// The runtime environment the program was analyzed against.
    pub env: Arc<RuntimeEnv>,
}

#[derive(Debug, Clone)]
pub struct PluginSemantic {
    pub info: PluginInfo,
    pub class_name: String,
    pub properties: Vec<DeclaredProperty>,
    pub controls: Vec<DeclaredControl>,
    pub pages: Vec<PageLayout>,
    pub helper_classes: Vec<ast::ClassDef>,
    pub event_bindings: Vec<ResolvedBinding>,
    pub methods: Vec<ResolvedMethod>,
    pub init_statements: Vec<ast::Stmt>,
    pub module_statements: Vec<ast::Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ControllerSemantic {
    pub class_name: String,
    pub controls: Vec<DeclaredControl>,
    pub helper_classes: Vec<ast::ClassDef>,
    pub event_bindings: Vec<ResolvedBinding>,
    pub methods: Vec<ResolvedMethod>,
    pub init_statements: Vec<ast::Stmt>,
    pub module_statements: Vec<ast::Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct ScriptSemantic {
    pub bound_pins: Vec<String>,
    pub event_bindings: Vec<ResolvedBinding>,
    pub classes: Vec<ast::ClassDef>,
    pub methods: Vec<ResolvedMethod>,
    pub statements: Vec<ast::Stmt>,
}

#[derive(Debug, Clone)]
pub struct ResolvedMethod {
    pub id: FunctionId,
    pub name: String,
    pub params: Vec<(SymbolId, String)>,
    pub body: Vec<ast::Stmt>,
    pub span: Span,
}

pub(crate) fn type_from_annotation(ann: Option<&str>, env: &RuntimeEnv) -> Type {
    match ann {
        Some("str") | Some("String") => Type::String,
        Some("int") | Some("Integer") => Type::Int,
        Some("float") | Some("Double") => Type::Float,
        Some("bool") | Some("Boolean") => Type::Bool,
        Some("tuple") | Some("Tuple") => Type::Tuple(Vec::new()),
        Some("None") => Type::None,
        Some("ControlHandle") => Type::ControlHandle,
        Some("PropertyHandle") => Type::PropertyHandle,
        Some(name) if env.object_type(name).is_some() => Type::Native {
            name: name.to_string(),
        },
        _ => Type::Any,
    }
}

/// Fault-tolerant semantic analysis that always returns a `SemanticModel` alongside any diagnostics.
/// Used by IDE / LSP features (`Analyzer`) so completions, hover, and inlay hints continue to work
/// even when the user is mid-edit and diagnostics are present.
///
/// Analyzes against the default Q-SYS runtime environment ([`RuntimeEnv::qsys`]).
pub fn analyze_lossy(parsed: &ParsedModule) -> (SemanticModel, Diagnostics) {
    analyze_lossy_with_env(parsed, RuntimeEnv::qsys())
}

/// Fault-tolerant semantic analysis against a custom runtime environment.
pub fn analyze_lossy_with_env(
    parsed: &ParsedModule,
    env: Arc<RuntimeEnv>,
) -> (SemanticModel, Diagnostics) {
    let mut diagnostics = Diagnostics::new();

    // 1. Collect helper classes
    let mut custom_classes: Vec<ast::ClassDef> = Vec::new();
    // 2. Collect top-level statements (constants, helper classes, utility functions, bundled modules, imports, etc.) in source order
    let mut module_statements: Vec<ast::Stmt> = Vec::new();

    for stmt in &parsed.module.statements {
        match &stmt.kind {
            ast::StmtKind::ClassDef(cls) => {
                if !cls
                    .bases
                    .iter()
                    .any(|b| b == "QSysPlugin" || b == "TextController")
                {
                    custom_classes.push(cls.clone());
                    module_statements.push(stmt.clone());
                }
            }
            ast::StmtKind::ModuleDef { body, .. } => {
                for m_stmt in body {
                    if let ast::StmtKind::ClassDef(cls) = &m_stmt.kind {
                        custom_classes.push(cls.clone());
                    }
                }
                module_statements.push(stmt.clone());
            }
            _ => {
                module_statements.push(stmt.clone());
            }
        }
    }

    let mut scope_stack = ScopeStack::new();
    let mut type_map = TypeMap::new();

    let classification = match classify_module(&parsed.module, &mut diagnostics) {
        Some(c) => c,
        None => {
            return (
                SemanticModel {
                    file: parsed.file.clone(),
                    target: TargetKind::ProceduralScript(ScriptSemantic::default()),
                    type_map,
                    env,
                },
                diagnostics,
            );
        }
    };

    // Pre-populate module scope with top-level constants, classes, functions, and imports for class-based targets
    if matches!(
        classification,
        TargetClassification::Plugin { .. } | TargetClassification::TextController { .. }
    ) {
        // A plugin's module-level statements are emitted before `PluginInfo`, so they execute
        // in both the design-time and runtime phases. Text controllers only run at runtime.
        let module_context = if matches!(classification, TargetClassification::Plugin { .. }) {
            ExecutionContext::Both
        } else {
            ExecutionContext::Runtime
        };
        let mut module_analyzer =
            ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics, &env)
                .with_context(module_context);
        module_analyzer.analyze_statements(&module_statements);
    }

    let target = match classification {
        TargetClassification::Plugin { class } => {
            let decls = extract_class_declarations(class, &mut scope_stack, &mut diagnostics);

            for (span, ctl_id) in &decls.ui_control_resolutions {
                type_map.insert_resolution(*span, ResolvedTarget::Control(*ctl_id));
                type_map.insert_type(*span, Type::ControlHandle);
            }
            for (span, prop_id) in &decls.control_property_resolutions {
                type_map.insert_resolution(*span, ResolvedTarget::Property(*prop_id));
                type_map.insert_type(*span, Type::Int);
            }

            scope_stack.push_scope(ScopeKind::Class {
                name: class.name.clone(),
            });

            // 1. Register controls in class scope
            for ctl in &decls.controls {
                let sym_id = scope_stack.alloc_symbol_id();
                scope_stack.define(Symbol::new(
                    sym_id,
                    ctl.ident.clone(),
                    SymbolKind::Control {
                        id: ctl.id,
                        def: ctl.def.clone(),
                    },
                    class.span,
                ));
            }

            // 2. Register properties in class scope
            for prop in &decls.properties {
                let sym_id = scope_stack.alloc_symbol_id();
                scope_stack.define(Symbol::new(
                    sym_id,
                    prop.ident.clone(),
                    SymbolKind::Property {
                        id: prop.id,
                        def: prop.def.clone(),
                    },
                    class.span,
                ));
            }

            // 3. PASS 1: Register all methods in class scope with a FunctionId
            let mut method_ids: FxHashMap<String, FunctionId> = FxHashMap::default();
            for stmt in &decls.remaining_statements {
                if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                    let method_id = scope_stack.alloc_function_id();
                    let sym_id = scope_stack.alloc_symbol_id();
                    scope_stack.define(Symbol::new(
                        sym_id,
                        f.name.clone(),
                        SymbolKind::Method {
                            id: method_id,
                            param_count: f.params.len(),
                            has_self: f.params.first().is_some_and(|p| p.name == "self"),
                        },
                        f.span,
                    ));
                    method_ids.insert(f.name.clone(), method_id);
                }
            }

            let mut event_bindings = Vec::new();
            let mut methods = Vec::new();
            let mut init_statements = Vec::new();

            // 4. PASS 2: Analyze method bodies and event decorators
            for stmt in &decls.remaining_statements {
                match &stmt.kind {
                    ast::StmtKind::FunctionDef(f) => {
                        let method_id = *method_ids.get(&f.name).unwrap();

                        for dec in &f.decorators {
                            if let Some(binding) = resolve_event_decorator(
                                dec,
                                method_id,
                                f,
                                &scope_stack,
                                &mut diagnostics,
                            ) {
                                event_bindings.push(binding);
                            }
                        }

                        scope_stack.push_scope(ScopeKind::Function {
                            name: f.name.clone(),
                        });

                        let self_id = scope_stack.alloc_symbol_id();
                        type_map.insert_symbol_name(self_id, "self");
                        scope_stack.define(Symbol::new(
                            self_id,
                            "self",
                            SymbolKind::Variable {
                                id: self_id,
                                ty: Type::ClassInstance {
                                    name: class.name.clone(),
                                },
                            },
                            f.span,
                        ));

                        let is_event_handler =
                            event_bindings.iter().any(|b| b.handler_id == method_id);
                        let mut resolved_params = Vec::new();
                        let skip_count =
                            usize::from(f.params.first().is_some_and(|p| p.name == "self"));

                        for (idx, param) in f.params.iter().enumerate().skip(skip_count) {
                            let param_ty = if is_event_handler && idx == 1 {
                                Type::ControlHandle
                            } else if is_event_handler
                                && idx == 2
                                && param.type_annotation.is_none()
                            {
                                Type::Int
                            } else {
                                type_from_annotation(param.type_annotation.as_deref(), &env)
                            };

                            let param_sym = scope_stack.alloc_symbol_id();
                            type_map.insert_symbol_name(param_sym, param.name.clone());
                            scope_stack.define(Symbol::new(
                                param_sym,
                                param.name.clone(),
                                SymbolKind::Parameter {
                                    id: param_sym,
                                    ty: param_ty.clone(),
                                    index: idx,
                                },
                                param.span,
                            ));
                            type_map
                                .insert_resolution(param.span, ResolvedTarget::Local(param_sym));
                            type_map.insert_type(param.span, param_ty);
                            resolved_params.push((param_sym, param.name.clone()));
                        }

                        let method_context = if is_design_time_method(&f.name) {
                            ExecutionContext::DesignTime
                        } else {
                            ExecutionContext::Runtime
                        };
                        let mut analyzer = ExprAnalyzer::new(
                            &mut scope_stack,
                            &mut type_map,
                            &mut diagnostics,
                            &env,
                        )
                        .with_context(method_context);
                        analyzer.analyze_statements(&f.body);

                        scope_stack.pop_scope();

                        let mut ret_ty = type_from_annotation(f.return_type.as_deref(), &env);
                        if ret_ty == Type::Any
                            && let Some(ret_expr) = analyzer::find_first_return_expr(&f.body)
                            && let Some(inferred) = type_map.get_type(ret_expr.span)
                            && *inferred != Type::Any
                        {
                            ret_ty = inferred.clone();
                        }
                        type_map.insert_class_member_type(
                            class.name.clone(),
                            f.name.clone(),
                            ret_ty,
                        );

                        methods.push(ResolvedMethod {
                            id: method_id,
                            name: f.name.clone(),
                            params: resolved_params,
                            body: f.body.clone(),
                            span: f.span,
                        });
                    }
                    ast::StmtKind::Pass => {}
                    ast::StmtKind::Expr(e) if matches!(e.kind, ast::ExprKind::String(_)) => {}
                    _ => {
                        init_statements.push(stmt.clone());
                    }
                }
            }

            if !init_statements.is_empty() {
                let self_id = scope_stack.alloc_symbol_id();
                type_map.insert_symbol_name(self_id, "self");
                scope_stack.define(Symbol::new(
                    self_id,
                    "self",
                    SymbolKind::Variable {
                        id: self_id,
                        ty: Type::ClassInstance {
                            name: class.name.clone(),
                        },
                    },
                    class.span,
                ));
                let mut analyzer =
                    ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics, &env)
                        .with_context(ExecutionContext::Runtime);
                analyzer.analyze_statements(&init_statements);
            }

            scope_stack.pop_scope();

            TargetKind::Plugin(PluginSemantic {
                info: decls.plugin_info,
                class_name: class.name.clone(),
                properties: decls.properties,
                controls: decls.controls,
                pages: decls.pages,
                helper_classes: custom_classes,
                event_bindings,
                methods,
                init_statements,
                module_statements,
                span: class.span,
            })
        }

        TargetClassification::TextController { class } => {
            let decls = extract_class_declarations(class, &mut scope_stack, &mut diagnostics);

            scope_stack.push_scope(ScopeKind::Class {
                name: class.name.clone(),
            });

            for ctl in &decls.controls {
                let sym_id = scope_stack.alloc_symbol_id();
                scope_stack.define(Symbol::new(
                    sym_id,
                    ctl.ident.clone(),
                    SymbolKind::Control {
                        id: ctl.id,
                        def: ctl.def.clone(),
                    },
                    class.span,
                ));
            }

            let mut method_ids: FxHashMap<String, FunctionId> = FxHashMap::default();
            for stmt in &decls.remaining_statements {
                if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                    let method_id = scope_stack.alloc_function_id();
                    let sym_id = scope_stack.alloc_symbol_id();
                    scope_stack.define(Symbol::new(
                        sym_id,
                        f.name.clone(),
                        SymbolKind::Method {
                            id: method_id,
                            param_count: f.params.len(),
                            has_self: f.params.first().is_some_and(|p| p.name == "self"),
                        },
                        f.span,
                    ));
                    method_ids.insert(f.name.clone(), method_id);
                }
            }

            let mut event_bindings = Vec::new();
            let mut methods = Vec::new();
            let mut init_statements = Vec::new();

            for stmt in &decls.remaining_statements {
                match &stmt.kind {
                    ast::StmtKind::FunctionDef(f) => {
                        let method_id = *method_ids.get(&f.name).unwrap();

                        for dec in &f.decorators {
                            if let Some(binding) = resolve_event_decorator(
                                dec,
                                method_id,
                                f,
                                &scope_stack,
                                &mut diagnostics,
                            ) {
                                event_bindings.push(binding);
                            }
                        }

                        scope_stack.push_scope(ScopeKind::Function {
                            name: f.name.clone(),
                        });

                        let self_id = scope_stack.alloc_symbol_id();
                        type_map.insert_symbol_name(self_id, "self");
                        scope_stack.define(Symbol::new(
                            self_id,
                            "self",
                            SymbolKind::Variable {
                                id: self_id,
                                ty: Type::ClassInstance {
                                    name: class.name.clone(),
                                },
                            },
                            f.span,
                        ));

                        let is_event_handler =
                            event_bindings.iter().any(|b| b.handler_id == method_id);
                        let mut resolved_params = Vec::new();
                        let skip_count =
                            usize::from(f.params.first().is_some_and(|p| p.name == "self"));

                        for (idx, param) in f.params.iter().enumerate().skip(skip_count) {
                            let param_ty = if is_event_handler && idx == 1 {
                                Type::ControlHandle
                            } else if is_event_handler
                                && idx == 2
                                && param.type_annotation.is_none()
                            {
                                Type::Int
                            } else {
                                type_from_annotation(param.type_annotation.as_deref(), &env)
                            };

                            let param_sym = scope_stack.alloc_symbol_id();
                            type_map.insert_symbol_name(param_sym, param.name.clone());
                            scope_stack.define(Symbol::new(
                                param_sym,
                                param.name.clone(),
                                SymbolKind::Parameter {
                                    id: param_sym,
                                    ty: param_ty.clone(),
                                    index: idx,
                                },
                                param.span,
                            ));
                            type_map
                                .insert_resolution(param.span, ResolvedTarget::Local(param_sym));
                            type_map.insert_type(param.span, param_ty);
                            resolved_params.push((param_sym, param.name.clone()));
                        }

                        let mut analyzer = ExprAnalyzer::new(
                            &mut scope_stack,
                            &mut type_map,
                            &mut diagnostics,
                            &env,
                        )
                        .with_context(ExecutionContext::Runtime);
                        analyzer.analyze_statements(&f.body);

                        scope_stack.pop_scope();

                        let mut ret_ty = type_from_annotation(f.return_type.as_deref(), &env);
                        if ret_ty == Type::Any
                            && let Some(ret_expr) = analyzer::find_first_return_expr(&f.body)
                            && let Some(inferred) = type_map.get_type(ret_expr.span)
                            && *inferred != Type::Any
                        {
                            ret_ty = inferred.clone();
                        }
                        type_map.insert_class_member_type(
                            class.name.clone(),
                            f.name.clone(),
                            ret_ty,
                        );

                        methods.push(ResolvedMethod {
                            id: method_id,
                            name: f.name.clone(),
                            params: resolved_params,
                            body: f.body.clone(),
                            span: f.span,
                        });
                    }
                    ast::StmtKind::Pass => {}
                    ast::StmtKind::Expr(e) if matches!(e.kind, ast::ExprKind::String(_)) => {}
                    _ => {
                        init_statements.push(stmt.clone());
                    }
                }
            }

            if !init_statements.is_empty() {
                let self_id = scope_stack.alloc_symbol_id();
                type_map.insert_symbol_name(self_id, "self");
                scope_stack.define(Symbol::new(
                    self_id,
                    "self",
                    SymbolKind::Variable {
                        id: self_id,
                        ty: Type::ClassInstance {
                            name: class.name.clone(),
                        },
                    },
                    class.span,
                ));
                let mut analyzer =
                    ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics, &env)
                        .with_context(ExecutionContext::Runtime);
                analyzer.analyze_statements(&init_statements);
            }

            scope_stack.pop_scope();

            TargetKind::TextController(ControllerSemantic {
                class_name: class.name.clone(),
                controls: decls.controls,
                helper_classes: custom_classes,
                event_bindings,
                methods,
                init_statements,
                module_statements,
                span: class.span,
            })
        }

        TargetClassification::ProceduralScript => {
            let mut analyzer =
                ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics, &env)
                    .with_context(ExecutionContext::Runtime);
            analyzer.analyze_statements(&parsed.module.statements);

            let mut methods = Vec::new();
            let mut top_level_statements = Vec::new();

            for stmt in &parsed.module.statements {
                if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                    let func_id = type_map
                        .get_resolution(f.span)
                        .and_then(|r| match r {
                            ResolvedTarget::Function(id) => Some(*id),
                            _ => None,
                        })
                        .unwrap_or(FunctionId(0));
                    let resolved_params = f
                        .params
                        .iter()
                        .map(|p| {
                            let sym_id = type_map
                                .get_resolution(p.span)
                                .and_then(|r| match r {
                                    ResolvedTarget::Local(id) => Some(*id),
                                    _ => None,
                                })
                                .unwrap_or(SymbolId(0));
                            (sym_id, p.name.clone())
                        })
                        .collect();
                    methods.push(ResolvedMethod {
                        id: func_id,
                        name: f.name.clone(),
                        params: resolved_params,
                        body: f.body.clone(),
                        span: f.span,
                    });
                } else {
                    top_level_statements.push(stmt.clone());
                }
            }

            TargetKind::ProceduralScript(ScriptSemantic {
                bound_pins: Vec::new(),
                event_bindings: Vec::new(),
                classes: custom_classes,
                methods,
                statements: top_level_statements,
            })
        }
    };

    (
        SemanticModel {
            file: parsed.file.clone(),
            target,
            type_map,
            env,
        },
        diagnostics,
    )
}

/// Strict semantic analysis against the default Q-SYS runtime environment.
pub fn analyze(parsed: &ParsedModule) -> Result<SemanticModel, Diagnostics> {
    analyze_with_env(parsed, RuntimeEnv::qsys())
}

/// Strict semantic analysis against a custom runtime environment.
pub fn analyze_with_env(
    parsed: &ParsedModule,
    env: Arc<RuntimeEnv>,
) -> Result<SemanticModel, Diagnostics> {
    let (model, diagnostics) = analyze_lossy_with_env(parsed, env);
    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(model)
    }
}

#[cfg(test)]
mod tests {
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_model::ControlMember;
    use qsys_parser::parse_module;

    use super::*;

    #[test]
    fn test_end_to_end_plugin_analysis() {
        let src = r#"
class Mixer(QSysPlugin):
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)

    @Gain.EventHandler
    def on_gain(self, ctl):
        if ctl.Value:
            print("Gain is active")
"#;
        let file = SourceFile::new(FileId(1), "test.py", src);
        let parsed = parse_module(file).expect("Should parse");
        let model = analyze(&parsed).expect("Semantic analysis should succeed");

        match &model.target {
            TargetKind::Plugin(p) => {
                assert_eq!(p.class_name, "Mixer");
                assert_eq!(p.controls.len(), 1);
                assert_eq!(p.controls[0].ident, "Gain");
                assert_eq!(p.controls[0].def.name, "MasterGain");
                assert_eq!(p.event_bindings.len(), 1);

                let binding = &p.event_bindings[0];
                assert_eq!(binding.control_name, "MasterGain");
                assert_eq!(binding.member, ControlMember::EventHandler);
                assert_eq!(binding.handler_name, "on_gain");
            }
            _ => panic!("Expected Plugin target"),
        }
    }

    #[test]
    fn test_escape_hatch_string_validation() {
        let valid_src = r#"
def run():
    qsys.lua("print('hello from lua')")
"#;
        let file = SourceFile::new(FileId(1), "test.py", valid_src);
        let parsed = parse_module(file).expect("Should parse");
        assert!(analyze(&parsed).is_ok());

        let invalid_src = r#"
def run():
    qsys.lua(12345)
"#;
        let file_bad = SourceFile::new(FileId(2), "bad.py", invalid_src);
        let parsed_bad = parse_module(file_bad).expect("Should parse");
        let result = analyze(&parsed_bad);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.items[0].message().contains("raw string literal"));
    }
}
