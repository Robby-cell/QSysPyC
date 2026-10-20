use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Authoritative catalog of all compiler diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    // Syntax & Parser Errors (E0001 - E0099)
    SyntaxError {
        message: String,
    },
    ExpectedModuleBody,

    // Unsupported Python Features (E0100 - E0199)
    UnsupportedAsync {
        construct: String,
    },
    UnsupportedGenerator,
    UnsupportedYieldFrom,
    UnsupportedMatchStatement,
    UnsupportedStatement {
        name: String,
    },
    UnsupportedExpression {
        name: String,
    },
    UnsupportedOperator {
        op: String,
    },
    UnsupportedChainedComparison,
    UnsupportedChainedAssignment,
    UnsupportedForTarget,
    UnsupportedComplexLiteral,

    // Semantic & Q-SYS Binding Errors (E0200 - E0299)
    UnknownControlMember {
        control_name: String,
        member: String,
        candidates: Vec<String>,
    },
    MissingSelfArgument {
        method_name: String,
    },
    DuplicateControlDeclaration {
        name: String,
    },
    InvalidDeclarationArgument {
        name: String,
        message: String,
    },
    InvalidEscapeHatchCall {
        message: String,
    },

    UndefinedVariable {
        name: String,
        candidates: Vec<String>,
    },
    UnknownClassMember {
        class_name: String,
        member: String,
        candidates: Vec<String>,
    },
    UnavailableInContext {
        name: String,
        available: String,
        context: String,
    },
    RemovedBuiltin {
        name: String,
        reason: String,
    },
}

impl DiagnosticKind {
    pub fn syntax_error(message: impl Into<String>) -> Self {
        Self::SyntaxError {
            message: message.into(),
        }
    }

    pub fn unsupported_async(construct: impl Into<String>) -> Self {
        Self::UnsupportedAsync {
            construct: construct.into(),
        }
    }

    pub fn unsupported_statement(name: impl Into<String>) -> Self {
        Self::UnsupportedStatement { name: name.into() }
    }

    pub fn unsupported_expression(name: impl Into<String>) -> Self {
        Self::UnsupportedExpression { name: name.into() }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::SyntaxError { .. } => "E0001",
            Self::ExpectedModuleBody => "E0002",
            Self::UnsupportedAsync { .. } => "E0101",
            Self::UnsupportedGenerator => "E0102",
            Self::UnsupportedYieldFrom => "E0103",
            Self::UnsupportedMatchStatement => "E0104",
            Self::UnsupportedStatement { .. } => "E0105",
            Self::UnsupportedExpression { .. } => "E0106",
            Self::UnsupportedOperator { .. } => "E0107",
            Self::UnsupportedChainedComparison => "E0108",
            Self::UnsupportedChainedAssignment => "E0109",
            Self::UnsupportedForTarget => "E0110",
            Self::UnsupportedComplexLiteral => "E0111",
            Self::UnknownControlMember { .. } => "E0201",
            Self::MissingSelfArgument { .. } => "E0202",
            Self::DuplicateControlDeclaration { .. } => "E0203",
            Self::InvalidDeclarationArgument { .. } => "E0204",
            Self::InvalidEscapeHatchCall { .. } => "E0205",
            Self::UndefinedVariable { .. } => "E0206",
            Self::UnknownClassMember { .. } => "E0207",
            Self::UnavailableInContext { .. } => "E0208",
            Self::RemovedBuiltin { .. } => "E0209",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::SyntaxError { message } => format!("Syntax error: {message}"),
            Self::ExpectedModuleBody => "Expected a valid Python module body".to_string(),
            Self::UnsupportedAsync { construct } => {
                format!("'{construct}' is not supported in QSysPy")
            }
            Self::UnsupportedGenerator => {
                "Generators ('yield') are not supported in QSysPy".to_string()
            }
            Self::UnsupportedYieldFrom => "'yield from' is not supported in QSysPy".to_string(),
            Self::UnsupportedMatchStatement => {
                "'match/case' statements are not supported in QSysPy".to_string()
            }
            Self::UnsupportedStatement { name } => {
                format!("Python statement '{name}' is not supported in QSysPy")
            }
            Self::UnsupportedExpression { name } => {
                format!("Python expression '{name}' is not supported in QSysPy")
            }
            Self::UnsupportedOperator { op } => {
                format!("Operator '{op}' is not supported in QSysPy")
            }
            Self::UnsupportedChainedComparison => {
                "Chained comparisons ('a < b < c') are not supported in QSysPy".to_string()
            }
            Self::UnsupportedChainedAssignment => {
                "Chained assignments ('a = b = ...') are not supported in QSysPy".to_string()
            }
            Self::UnsupportedForTarget => {
                "Pattern unpacking in for loops is not supported in QSysPy".to_string()
            }
            Self::UnsupportedComplexLiteral => {
                "Complex numbers are not supported in QSysPy".to_string()
            }
            Self::UnknownControlMember {
                control_name,
                member,
                candidates,
            } => {
                let hint = if candidates.is_empty() {
                    String::new()
                } else {
                    format!("; did you mean '{}'?", candidates.join(", "))
                };
                format!("Control '{control_name}' has no member '{member}'{hint}")
            }
            Self::MissingSelfArgument { method_name } => {
                format!("Method '{method_name}' is missing 'self' as its first parameter")
            }
            Self::DuplicateControlDeclaration { name } => {
                format!("Control '{name}' is declared multiple times")
            }
            Self::InvalidDeclarationArgument { name, message } => {
                format!("Invalid declaration for '{name}': {message}")
            }
            Self::InvalidEscapeHatchCall { message } => {
                format!("Invalid 'qsys.lua()' invocation: {message}")
            }
            Self::UndefinedVariable { name, candidates } => {
                let hint = if candidates.is_empty() {
                    String::new()
                } else {
                    format!("; did you mean '{}'?", candidates.join(", "))
                };
                format!("Undefined variable '{name}'{hint}")
            }
            Self::UnknownClassMember {
                class_name,
                member,
                candidates,
            } => {
                let hint = if candidates.is_empty() {
                    String::new()
                } else {
                    format!("; did you mean '{}'?", candidates.join(", "))
                };
                format!("Class '{class_name}' has no control or property '{member}'{hint}")
            }
            Self::UnavailableInContext {
                name,
                available,
                context,
            } => {
                format!("'{name}' is only available {available}, but is used {context}")
            }
            Self::RemovedBuiltin { name, reason } => {
                format!("'{name}' is not available on this target: {reason}")
            }
        }
    }

    pub fn help(&self) -> Option<String> {
        match self {
            Self::UnsupportedAsync { .. } => Some(
                "Q-SYS scripts execute in a single-threaded cooperative event loop. Use Timers for delayed or periodic operations.".to_string(),
            ),
            Self::UnsupportedGenerator | Self::UnsupportedYieldFrom => Some(
                "Collect items into a list and return the list directly.".to_string(),
            ),
            Self::UnsupportedChainedComparison => Some(
                "Split the condition using 'and': 'a < b and b < c'.".to_string(),
            ),
            Self::UnsupportedChainedAssignment => Some(
                "Assign each variable on its own line.".to_string(),
            ),
            Self::UnsupportedForTarget => Some(
                "Iterate over a single variable or iterate by index: 'for item in items:'.".to_string(),
            ),
            Self::MissingSelfArgument { .. } => Some(
                "Add 'self' as the first parameter: def method(self, ...)".to_string(),
            ),
            Self::InvalidEscapeHatchCall { .. } => Some(
                "qsys.lua() requires exactly one string literal: qsys.lua(\"print('raw')\")".to_string(),
            ), Self::UndefinedVariable { .. } => Some(
                "Ensure the variable is defined in the current scope or passed as a parameter.".to_string(),
            ),
            Self::UnknownClassMember { .. } => Some(
                "Controls and properties must be declared at the class level: 'name = Control.<Type>()'.".to_string(),
            ),
            Self::UnavailableInContext { available, .. } => Some(format!(
                "Move this usage into code that runs {available}, such as a runtime method or event handler."
            )),
            _ => None,
        }
    }

    pub fn severity(&self) -> Severity {
        Severity::Error
    }
}
