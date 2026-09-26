use crate::span::{FileId, Span};

use std::sync::Arc;

/// In-memory representation of a source file with pre-calculated line offsets.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub id: FileId,
    pub name: String,
    pub src: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(id: FileId, name: impl Into<String>, src: impl Into<String>) -> Arc<Self> {
        let name = name.into();
        let src = src.into();

        // Precompute line starts for fast O(log N) line/col lookup
        let mut line_starts = vec![0];
        for (idx, byte) in src.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(idx + 1);
            }
        }

        Arc::new(Self {
            id,
            name,
            src,
            line_starts,
        })
    }

    /// Converts a byte offset to 1-indexed (line, column).
    pub fn location(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.src.len());
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };

        let line_start = self.line_starts[line_idx];
        let col = offset.saturating_sub(line_start) + 1;
        (line_idx + 1, col)
    }

    /// Slices the text corresponding to a Span.
    pub fn slice(&self, span: Span) -> &str {
        let start = span.start.min(self.src.len());
        let end = span.end.min(self.src.len());
        if start <= end {
            &self.src[start..end]
        } else {
            ""
        }
    }

    /// Converts 1-indexed (line, column) to a 0-indexed byte offset.
    pub fn offset(&self, line: usize, col: usize) -> usize {
        if line == 0 || self.line_starts.is_empty() {
            return 0;
        }
        let line_idx = (line - 1).min(self.line_starts.len() - 1);
        let line_start = self.line_starts[line_idx];
        (line_start + col.saturating_sub(1)).min(self.src.len())
    }
}
