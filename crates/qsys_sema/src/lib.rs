pub mod analyzer;
pub mod binding;
pub mod classifier;
pub mod declarations;
pub mod lower;
pub mod symbols;
pub mod types;

pub use binding::{ResolvedBinding, resolve_event_decorator};
pub use classifier::{TargetClassification, classify_module};
pub use declarations::{
    ConstValue, DeclaredControl, DeclaredProperty, ExtractedDeclarations, eval_constant,
    extract_class_declarations,
};
pub use lower::lower_to_ir;
pub use symbols::{ScopeKind, ScopeStack, Symbol, SymbolKind};
pub use types::{ResolvedTarget, TruthinessStrategy, Type, TypeMap};

use std::sync::Arc;

use qsys_diagnostics::{Diagnostics, SourceFile, Span};
use qsys_ir::{FunctionId, SymbolId};
use qsys_model::PluginInfo;
use qsys_parser::ParsedModule;
use qsys_parser::ast;

use analyzer::ExprAnalyzer;

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
}

#[derive(Debug, Clone)]
pub struct PluginSemantic {
    pub info: PluginInfo,
    pub class_name: String,
    pub properties: Vec<DeclaredProperty>,
    pub controls: Vec<DeclaredControl>,
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
    pub span: Span,
}

#[derive(Debug, Clone)]
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

fn type_from_annotation(ann: Option<&str>) -> Type {
    match ann {
        Some("str") | Some("String") => Type::String,
        Some("int") | Some("Integer") => Type::Int,
        Some("float") | Some("Double") => Type::Float,
        Some("bool") | Some("Boolean") => Type::Bool,
        _ => Type::Any,
    }
}

pub fn analyze(parsed: &ParsedModule) -> Result<SemanticModel, Diagnostics> {
    let mut diagnostics = Diagnostics::new();

    // 1. Collect helper classes
    let mut custom_classes: Vec<ast::ClassDef> = Vec::new();
    // 2. Collect top-level statements (constants, utility functions, etc.)
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
                }
            }
            ast::StmtKind::Import { .. } | ast::StmtKind::ImportFrom { .. } => {}
            _ => {
                module_statements.push(stmt.clone());
            }
        }
    }

    // Classify module target (Plugin vs TextController vs Script)
    let classification = match classify_module(&parsed.module, &mut diagnostics) {
        Some(c) => c,
        None => return Err(diagnostics),
    };

    let mut scope_stack = ScopeStack::new();
    let mut type_map = TypeMap::new();

    // Pre-populate module scope with top-level constants and functions
    {
        let mut module_analyzer =
            ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics);
        for stmt in &parsed.module.statements {
            if let ast::StmtKind::ClassDef(cls) = &stmt.kind
                && cls
                    .bases
                    .iter()
                    .any(|b| b == "QSysPlugin" || b == "TextController")
            {
                continue;
            }
            module_analyzer.analyze_stmt(stmt);
        }
    }

    let target = match classification {
        TargetClassification::Plugin { class } => {
            let decls = extract_class_declarations(class, &mut scope_stack, &mut diagnostics);

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
            // This ensures `self.send_packet()` can resolve to FunctionId(0)
            let mut method_ids: rustc_hash::FxHashMap<String, FunctionId> =
                rustc_hash::FxHashMap::default();
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
                            has_self: f.params.first().map(|p| p.name == "self").unwrap_or(false),
                        },
                        f.span,
                    ));
                    method_ids.insert(f.name.clone(), method_id);
                }
            }

            let mut event_bindings = Vec::new();
            let mut methods = Vec::new();

            // 4. PASS 2: Analyze method bodies and event decorators
            for stmt in &decls.remaining_statements {
                if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
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

                    let is_event_handler = event_bindings.iter().any(|b| b.handler_id == method_id);
                    let mut resolved_params = Vec::new();

                    for (idx, param) in f.params.iter().enumerate().skip(1) {
                        let param_ty = if is_event_handler && idx == 1 {
                            Type::ControlHandle
                        } else {
                            type_from_annotation(param.type_annotation.as_deref())
                        };

                        let param_sym = scope_stack.alloc_symbol_id();
                        scope_stack.define(Symbol::new(
                            param_sym,
                            param.name.clone(),
                            SymbolKind::Parameter {
                                id: param_sym,
                                ty: param_ty,
                                index: idx,
                            },
                            param.span,
                        ));
                        type_map.insert_resolution(param.span, ResolvedTarget::Local(param_sym));
                        resolved_params.push((param_sym, param.name.clone()));
                    }

                    let mut analyzer =
                        ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics);
                    analyzer.analyze_statements(&f.body);

                    scope_stack.pop_scope();

                    methods.push(ResolvedMethod {
                        id: method_id,
                        name: f.name.clone(),
                        params: resolved_params,
                        body: f.body.clone(),
                        span: f.span,
                    });
                }
            }

            scope_stack.pop_scope();

            TargetKind::Plugin(PluginSemantic {
                info: decls.plugin_info,
                class_name: class.name.clone(),
                properties: decls.properties,
                controls: decls.controls,
                helper_classes: custom_classes.clone(),
                event_bindings,
                methods,
                init_statements: decls.remaining_statements,
                module_statements, // <--- PASS IT HERE
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

            let mut event_bindings = Vec::new();
            let mut methods = Vec::new();

            for stmt in &decls.remaining_statements {
                if let ast::StmtKind::FunctionDef(f) = &stmt.kind {
                    let method_id = scope_stack.alloc_function_id();

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

                    let is_event_handler = event_bindings.iter().any(|b| b.handler_id == method_id);
                    let mut resolved_params = Vec::new();

                    for (idx, param) in f.params.iter().enumerate().skip(1) {
                        let param_ty = if is_event_handler && idx == 1 {
                            Type::ControlHandle
                        } else {
                            type_from_annotation(param.type_annotation.as_deref())
                        };

                        let param_sym = scope_stack.alloc_symbol_id();
                        scope_stack.define(Symbol::new(
                            param_sym,
                            param.name.clone(),
                            SymbolKind::Parameter {
                                id: param_sym,
                                ty: param_ty,
                                index: idx,
                            },
                            param.span,
                        ));
                        type_map.insert_resolution(param.span, ResolvedTarget::Local(param_sym));
                        resolved_params.push((param_sym, param.name.clone()));
                    }

                    let mut analyzer =
                        ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics);
                    analyzer.analyze_statements(&f.body);

                    scope_stack.pop_scope();

                    methods.push(ResolvedMethod {
                        id: method_id,
                        name: f.name.clone(),
                        params: resolved_params,
                        body: f.body.clone(),
                        span: f.span,
                    });
                }
            }

            scope_stack.pop_scope();

            TargetKind::TextController(ControllerSemantic {
                class_name: class.name.clone(),
                controls: decls.controls,
                helper_classes: custom_classes.clone(),
                event_bindings,
                methods,
                init_statements: decls.remaining_statements,
                span: class.span,
            })
        }

        TargetClassification::ProceduralScript => {
            let mut methods = Vec::new();
            let mut top_level_statements = Vec::new();

            for stmt in &parsed.module.statements {
                match &stmt.kind {
                    ast::StmtKind::FunctionDef(f) => {
                        let func_id = scope_stack.alloc_function_id();
                        let func_sym = scope_stack.alloc_symbol_id();
                        scope_stack.define(Symbol::new(
                            func_sym,
                            f.name.clone(),
                            SymbolKind::Method {
                                id: func_id,
                                param_count: f.params.len(),
                                has_self: false,
                            },
                            f.span,
                        ));
                        type_map.insert_resolution(f.span, ResolvedTarget::Function(func_id));

                        scope_stack.push_scope(ScopeKind::Function {
                            name: f.name.clone(),
                        });

                        let mut resolved_params = Vec::new();
                        for (idx, param) in f.params.iter().enumerate() {
                            let param_ty = type_from_annotation(param.type_annotation.as_deref());
                            let param_sym = scope_stack.alloc_symbol_id();
                            scope_stack.define(Symbol::new(
                                param_sym,
                                param.name.clone(),
                                SymbolKind::Parameter {
                                    id: param_sym,
                                    ty: param_ty,
                                    index: idx,
                                },
                                param.span,
                            ));
                            type_map
                                .insert_resolution(param.span, ResolvedTarget::Local(param_sym));
                            resolved_params.push((param_sym, param.name.clone()));
                        }

                        let mut analyzer =
                            ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics);
                        analyzer.analyze_statements(&f.body);

                        scope_stack.pop_scope();

                        methods.push(ResolvedMethod {
                            id: func_id,
                            name: f.name.clone(),
                            params: resolved_params,
                            body: f.body.clone(),
                            span: f.span,
                        });
                    }
                    _ => {
                        let mut analyzer =
                            ExprAnalyzer::new(&mut scope_stack, &mut type_map, &mut diagnostics);
                        analyzer.analyze_stmt(stmt);
                        top_level_statements.push(stmt.clone());
                    }
                }
            }

            TargetKind::ProceduralScript(ScriptSemantic {
                bound_pins: Vec::new(),
                event_bindings: Vec::new(),
                classes: custom_classes.clone(),
                methods,
                statements: top_level_statements,
            })
        }
    };

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(SemanticModel {
            file: parsed.file.clone(),
            target,
            type_map,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsys_diagnostics::{FileId, SourceFile};
    use qsys_model::ControlMember;
    use qsys_parser::parse_module;

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
