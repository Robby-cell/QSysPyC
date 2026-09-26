use serde::{Deserialize, Serialize};

/// Supported property types in Q-SYS Designer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropertyKind {
    Integer {
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
    },
    Double {
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
    String {
        default: String,
    },
    Enum {
        choices: Vec<String>,
        default: String,
    },
    Boolean {
        default: bool,
    },
}

impl PropertyKind {
    pub fn qsys_type_name(&self) -> &'static str {
        match self {
            Self::Integer { .. } => "integer",
            Self::Double { .. } => "double",
            Self::String { .. } => "string",
            Self::Enum { .. } => "enum",
            Self::Boolean { .. } => "boolean",
        }
    }
}

/// A property declaration exposed in the Q-SYS Properties inspector panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertyDef {
    pub name: String,
    pub kind: PropertyKind,
    pub description: Option<String>,
}

impl PropertyDef {
    pub fn integer(
        name: impl Into<String>,
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Integer { default, min, max },
            description: None,
        }
    }

    pub fn double(
        name: impl Into<String>,
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Double { default, min, max },
            description: None,
        }
    }

    pub fn string(name: impl Into<String>, default: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::String {
                default: default.into(),
            },
            description: None,
        }
    }

    pub fn enumeration(
        name: impl Into<String>,
        choices: impl IntoIterator<Item = impl Into<String>>,
        default: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Enum {
                choices: choices.into_iter().map(Into::into).collect(),
                default: default.into(),
            },
            description: None,
        }
    }

    pub fn boolean(name: impl Into<String>, default: bool) -> Self {
        Self {
            name: name.into(),
            kind: PropertyKind::Boolean { default },
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}
