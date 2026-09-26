use crate::Severity;

use serde::{Deserialize, Serialize};

/// JSON-serializable marker for web editors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorMarker {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    #[serde(rename = "startLineNumber")]
    pub start_line_number: usize,
    #[serde(rename = "startColumn")]
    pub start_column: usize,
    #[serde(rename = "endLineNumber")]
    pub end_line_number: usize,
    #[serde(rename = "endColumn")]
    pub end_column: usize,
    pub help: Option<String>,
}
