use wasm_bindgen::prelude::*;

use qsys_compiler::{Analyzer, Compiler, CompilerOptions, generate_default_pyi};

#[wasm_bindgen]
pub struct CompilationResult {
    success: bool,
    lua: String,
    markers_json: String,
    target_kind: String,
}

#[wasm_bindgen]
impl CompilationResult {
    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn lua(&self) -> String {
        self.lua.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn markers(&self) -> String {
        self.markers_json.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn target_kind(&self) -> String {
        self.target_kind.clone()
    }
}

#[wasm_bindgen]
pub fn compile_qsyspy(source: &str, filename: Option<String>) -> CompilationResult {
    let name = filename.unwrap_or_else(|| "playground.py".to_string());
    let options = CompilerOptions {
        filename: name.clone(),
        ..Default::default()
    };

    match Compiler::compile(source, &options) {
        Ok(artifact) => {
            let markers_json = artifact.to_editor_markers_json();
            CompilationResult {
                success: true,
                lua: artifact.lua,
                markers_json,
                target_kind: format!("{:?}", artifact.program_kind),
            }
        }
        Err(diags) => {
            let file = qsys_diagnostics::SourceFile::new(qsys_diagnostics::FileId(1), name, source);
            let markers = diags.to_editor_markers(&file);
            CompilationResult {
                success: false,
                lua: String::new(),
                markers_json: serde_json::to_string(&markers).unwrap_or_else(|_| "[]".to_string()),
                target_kind: "Unknown".to_string(),
            }
        }
    }
}

/// Query the Rust compiler for hover documentation at (line, column).
#[wasm_bindgen]
pub fn get_hover(source: &str, line: usize, col: usize) -> Option<String> {
    Analyzer::hover(source, line, col).map(|h| serde_json::to_string(&h).unwrap())
}

/// Query the Rust compiler for context-aware completions at (line, column).
#[wasm_bindgen]
pub fn get_completions(source: &str, line: usize, col: usize) -> String {
    let items = Analyzer::completions(source, line, col);
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
}

/// Query the Rust compiler for inline type hints.
#[wasm_bindgen]
pub fn get_inlay_hints(source: &str) -> String {
    let hints = Analyzer::inlay_hints(source);
    serde_json::to_string(&hints).unwrap_or_else(|_| "[]".to_string())
}

/// Query the Rust compiler for function/method signature help at (line, column).
#[wasm_bindgen]
pub fn get_signature_help(source: &str, line: usize, col: usize) -> Option<String> {
    Analyzer::signature_help(source, line, col).and_then(|s| serde_json::to_string(&s).ok())
}

/// Generate the PEP 561 `qsys.pyi` Python type stub file for IDE integration.
#[wasm_bindgen]
pub fn get_qsys_stubs() -> String {
    generate_default_pyi()
}
