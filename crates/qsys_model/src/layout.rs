use serde::{Deserialize, Serialize};

/// Represents a 2D position or coordinate (X, Y) in points.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Represents a 2D bounding dimension (Width, Height) in points.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// Color representation in Q-SYS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Color {
    Rgb(u8, u8, u8),
    Hex(String),   // e.g. "#FF0000"
    Named(String), // e.g. "Red", "Transparent"
}

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb(r, g, b)
    }

    pub fn hex(hex: impl Into<String>) -> Self {
        Self::Hex(hex.into())
    }
}

/// Visual control widget style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlStyle {
    Fader,
    Knob,
    Button,
    Text,
    Meter,
    Led,
    Custom(String),
}

impl ControlStyle {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fader => "Fader",
            Self::Knob => "Knob",
            Self::Button => "Button",
            Self::Text => "Text",
            Self::Meter => "Meter",
            Self::Led => "Led",
            Self::Custom(s) => s.as_str(),
        }
    }
}

/// Per-control graphical layout parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlLayoutDef {
    pub name: String,
    pub pretty_name: Option<String>,
    pub style: Option<ControlStyle>,
    pub position: Position,
    pub size: Size,
    pub color: Option<Color>,
    pub margin: Option<f64>,
    pub legend: Option<String>,
    pub is_disabled: bool,
    pub is_invisible: bool,
}

impl ControlLayoutDef {
    pub fn new(name: impl Into<String>, position: Position, size: Size) -> Self {
        Self {
            name: name.into(),
            pretty_name: None,
            style: None,
            position,
            size,
            color: None,
            margin: None,
            legend: None,
            is_disabled: false,
            is_invisible: false,
        }
    }
}

/// Static graphic decoration elements (GroupBox, Header, Text, Image).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GraphicItemKind {
    Header {
        text: String,
        font_size: u32,
    },
    GroupBox {
        text: String,
        stroke: u32,
    },
    Label {
        text: String,
        font_size: u32,
        h_align: String,
    },
    Separator,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphicItem {
    pub kind: GraphicItemKind,
    pub position: Position,
    pub size: Size,
    pub color: Option<Color>,
}
