use serde::{Deserialize, Serialize};

/// Pin styling on the Q-SYS schematic component block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PinStyle {
    #[default]
    None,
    Input,
    Output,
    Both,
}

impl PinStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Input => "Input",
            Self::Output => "Output",
            Self::Both => "Both",
        }
    }
}

/// Engineering units supported by Q-SYS continuous controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControlUnit {
    #[default]
    Float,
    Integer,
    Decibels,
    Hertz,
    Percent,
    Seconds,
    Pan,
}

impl ControlUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Float => "Float",
            Self::Integer => "Integer",
            Self::Decibels => "dB",
            Self::Hertz => "Hz",
            Self::Percent => "Percent",
            Self::Seconds => "Seconds",
            Self::Pan => "Pan",
        }
    }
}

/// Type of Button control in Q-SYS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ButtonType {
    #[default]
    Momentary,
    Toggle,
    Trigger,
}

impl ButtonType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Momentary => "Momentary",
            Self::Toggle => "Toggle",
            Self::Trigger => "Trigger",
        }
    }
}

/// Type of Text control in Q-SYS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextType {
    #[default]
    Text,
    ReadOnly,
}

impl TextType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::ReadOnly => "ReadOnly",
        }
    }
}

/// The specific configuration for a Q-SYS control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlKind {
    Button {
        button_type: ButtonType,
    },
    Knob {
        unit: ControlUnit,
        min: f64,
        max: f64,
        default: f64,
    },
    Fader {
        unit: ControlUnit,
        min: f64,
        max: f64,
        default: f64,
    },
    Text {
        text_type: TextType,
        default: String,
    },
    Indicator {
        unit: ControlUnit,
    },
}

impl ControlKind {
    pub fn qsys_type_str(&self) -> &'static str {
        match self {
            Self::Button { .. } => "Button",
            Self::Knob { .. } => "Knob",
            Self::Fader { .. } => "Fader",
            Self::Text { .. } => "Text",
            Self::Indicator { .. } => "Indicator",
        }
    }
}

/// Configuration for a property-typed generic control (`Control[Type](...)` / `Control.Generic(Type, ...)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenericControlSpec {
    pub property_name: String,
    pub choices: Vec<String>,
    pub default_choice: String,
    pub unit: ControlUnit,
    pub min: f64,
    pub max: f64,
    pub default_num: f64,
    pub default_str: String,
    pub button_type: ButtonType,
    pub text_type: TextType,
}

impl GenericControlSpec {
    /// Resolves a choice string (e.g. `"Integer"`, `"String"`, `"Boolean"`, `"Float"`) into a [`ControlKind`].
    pub fn resolve_choice_kind(&self, choice: &str) -> ControlKind {
        match choice.trim().to_ascii_lowercase().as_str() {
            "string" | "text" | "str" => ControlKind::Text {
                text_type: self.text_type,
                default: self.default_str.clone(),
            },
            "readonly" => ControlKind::Text {
                text_type: TextType::ReadOnly,
                default: self.default_str.clone(),
            },
            "boolean" | "bool" | "toggle" => ControlKind::Button {
                button_type: ButtonType::Toggle,
            },
            "button" | "momentary" => ControlKind::Button {
                button_type: ButtonType::Momentary,
            },
            "trigger" => ControlKind::Button {
                button_type: ButtonType::Trigger,
            },
            "fader" | "slider" => ControlKind::Fader {
                unit: self.unit,
                min: self.min,
                max: self.max,
                default: self.default_num,
            },
            "indicator" | "led" | "meter" => ControlKind::Indicator { unit: self.unit },
            "integer" | "int" => ControlKind::Knob {
                unit: ControlUnit::Integer,
                min: self.min,
                max: self.max,
                default: self.default_num,
            },
            _ => ControlKind::Knob {
                unit: self.unit,
                min: self.min,
                max: self.max,
                default: self.default_num,
            },
        }
    }
}

/// Authoritative compile-time definition of a control.
/// Matches the table format emitted in `GetControls(props)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlDef {
    pub name: String,
    pub kind: ControlKind,
    pub count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_property: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generic_spec: Option<GenericControlSpec>,
    pub pin_style: PinStyle,
    pub user_pin: bool,
}

impl ControlDef {
    pub fn new(name: impl Into<String>, kind: ControlKind) -> Self {
        Self {
            name: name.into(),
            kind,
            count: 1,
            count_property: None,
            generic_spec: None,
            pin_style: PinStyle::Both,
            user_pin: true,
        }
    }

    pub fn button(name: impl Into<String>, button_type: ButtonType) -> Self {
        Self::new(name, ControlKind::Button { button_type })
    }

    pub fn knob(
        name: impl Into<String>,
        unit: ControlUnit,
        min: f64,
        max: f64,
        default: f64,
    ) -> Self {
        Self::new(
            name,
            ControlKind::Knob {
                unit,
                min,
                max,
                default,
            },
        )
    }

    pub fn fader(
        name: impl Into<String>,
        unit: ControlUnit,
        min: f64,
        max: f64,
        default: f64,
    ) -> Self {
        Self::new(
            name,
            ControlKind::Fader {
                unit,
                min,
                max,
                default,
            },
        )
    }

    pub fn text(name: impl Into<String>, text_type: TextType, default: impl Into<String>) -> Self {
        Self::new(
            name,
            ControlKind::Text {
                text_type,
                default: default.into(),
            },
        )
    }

    pub fn with_count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }

    pub fn with_count_property(mut self, count_property: Option<String>) -> Self {
        self.count_property = count_property;
        self
    }

    pub fn with_generic_spec(mut self, generic_spec: Option<GenericControlSpec>) -> Self {
        self.generic_spec = generic_spec;
        self
    }

    pub fn with_pin_style(mut self, pin_style: PinStyle) -> Self {
        self.pin_style = pin_style;
        self
    }

    pub fn with_user_pin(mut self, user_pin: bool) -> Self {
        self.user_pin = user_pin;
        self
    }
}

/// Valid runtime members on any Q-SYS Control handle.
/// Used by `qsys_sema` for autocomplete and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlMember {
    Value,
    String,
    Boolean,
    Data,
    Generic,
    Any,
    Values,
    Strings,
    Choices,
    EventHandler,
    IsDisabled,
    IsInvisible,
    Color,
    Legend,
    RampTime,
}

impl ControlMember {
    pub fn from_ident(ident: &str) -> Option<Self> {
        match ident {
            "Value" => Some(Self::Value),
            "String" => Some(Self::String),
            "Boolean" => Some(Self::Boolean),
            "Data" => Some(Self::Data),
            "Generic" => Some(Self::Generic),
            "Any" => Some(Self::Any),
            "Values" => Some(Self::Values),
            "Strings" => Some(Self::Strings),
            "Choices" => Some(Self::Choices),
            "EventHandler" => Some(Self::EventHandler),
            "IsDisabled" => Some(Self::IsDisabled),
            "IsInvisible" => Some(Self::IsInvisible),
            "Color" => Some(Self::Color),
            "Legend" => Some(Self::Legend),
            "RampTime" => Some(Self::RampTime),
            _ => None,
        }
    }

    pub fn is_generic_accessor(&self) -> bool {
        matches!(self, Self::Data | Self::Generic | Self::Any)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Value => "Value",
            Self::String => "String",
            Self::Boolean => "Boolean",
            Self::Data => "Data",
            Self::Generic => "Generic",
            Self::Any => "Any",
            Self::Values => "Values",
            Self::Strings => "Strings",
            Self::Choices => "Choices",
            Self::EventHandler => "EventHandler",
            Self::IsDisabled => "IsDisabled",
            Self::IsInvisible => "IsInvisible",
            Self::Color => "Color",
            Self::Legend => "Legend",
            Self::RampTime => "RampTime",
        }
    }

    pub fn all_member_names() -> &'static [&'static str] {
        &[
            "Value",
            "String",
            "Boolean",
            "Data",
            "Generic",
            "Any",
            "Values",
            "Strings",
            "Choices",
            "EventHandler",
            "IsDisabled",
            "IsInvisible",
            "Color",
            "Legend",
            "RampTime",
        ]
    }
}
