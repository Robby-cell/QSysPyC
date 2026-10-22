use serde::{Deserialize, Serialize};

use crate::controls::ControlKind;

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

    pub fn parse_str(s: &str) -> Self {
        let trimmed = s.trim();
        if trimmed.starts_with('#') {
            Self::Hex(trimmed.to_string())
        } else {
            Self::Named(trimmed.to_string())
        }
    }

    /// Converts any `Color` representation into an `(r, g, b)` byte triple for Q-SYS Lua `{ r, g, b }`.
    pub fn to_rgb_tuple(&self) -> (u8, u8, u8) {
        match self {
            Self::Rgb(r, g, b) => (*r, *g, *b),
            Self::Hex(hex) => parse_hex_rgb(hex).unwrap_or((14, 99, 156)),
            Self::Named(name) => match name.to_ascii_lowercase().as_str() {
                "red" | "danger" | "error" => (217, 83, 79),
                "green" | "ok" | "success" => (78, 201, 176),
                "blue" | "primary" | "accent" => (14, 99, 156),
                "yellow" | "warning" | "amber" => (204, 167, 0),
                "orange" => (224, 122, 46),
                "purple" | "violet" => (137, 87, 229),
                "cyan" | "teal" => (42, 161, 152),
                "white" => (255, 255, 255),
                "black" => (20, 20, 20),
                "gray" | "grey" | "muted" => (120, 120, 124),
                "dark" | "panel" => (45, 45, 48),
                _ => (14, 99, 156),
            },
        }
    }
}

fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let clean = hex.trim_start_matches('#');
    if clean.len() == 6 {
        let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
        let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
        let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
        Some((r, g, b))
    } else if clean.len() == 3 {
        let r = u8::from_str_radix(&clean[0..1], 16).ok()? * 17;
        let g = u8::from_str_radix(&clean[1..2], 16).ok()? * 17;
        let b = u8::from_str_radix(&clean[2..3], 16).ok()? * 17;
        Some((r, g, b))
    } else {
        None
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

    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "fader" | "slider" => Self::Fader,
            "knob" | "dial" | "rotary" => Self::Knob,
            "button" | "btn" | "toggle" | "trigger" => Self::Button,
            "text" | "textbox" | "input" => Self::Text,
            "meter" | "level" => Self::Meter,
            "led" | "indicator" => Self::Led,
            _ => Self::Custom(name.to_string()),
        }
    }

    pub fn default_for_kind(kind: &ControlKind) -> Self {
        match kind {
            ControlKind::Fader { .. } => Self::Fader,
            ControlKind::Knob { .. } => Self::Knob,
            ControlKind::Button { .. } => Self::Button,
            ControlKind::Text { .. } => Self::Text,
            ControlKind::Indicator { .. } => Self::Led,
        }
    }

    pub fn default_size(&self) -> Size {
        match self {
            Self::Fader => Size::new(160.0, 36.0),
            Self::Knob => Size::new(56.0, 56.0),
            Self::Button => Size::new(84.0, 32.0),
            Self::Text => Size::new(140.0, 28.0),
            Self::Meter => Size::new(36.0, 88.0),
            Self::Led => Size::new(24.0, 24.0),
            Self::Custom(_) => Size::new(100.0, 32.0),
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

/// A solved Q-SYS page containing positioned control layouts and static graphic elements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageLayout {
    pub name: String,
    pub size: Size,
    pub controls: Vec<ControlLayoutDef>,
    pub graphics: Vec<GraphicItem>,
}

/// Layout direction for container nodes (`Row` vs `Col` / `GroupBox`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlexDirection {
    Row,
    Column,
}

/// Placement of an inline label relative to a `UI.Control` or `UI.Field`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LabelPlacement {
    Top,
    Left { width: f64 },
}

/// Declarative specification of a control widget inside a `UI` tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiControlSpec {
    pub control_name: String,
    pub style: ControlStyle,
    pub label: Option<String>,
    pub label_placement: LabelPlacement,
    pub pretty_name: Option<String>,
    pub legend: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub color: Option<Color>,
    pub margin: Option<f64>,
    pub read_only: bool,
    pub invisible: bool,
}

/// Declarative HTML/Flexbox-like UI node authored in Python via `UI.*`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UiNode {
    Row {
        children: Vec<UiNode>,
        gap: f64,
        padding: f64,
    },
    Col {
        children: Vec<UiNode>,
        gap: f64,
        padding: f64,
    },
    GroupBox {
        title: String,
        children: Vec<UiNode>,
        direction: FlexDirection,
        gap: f64,
        padding: f64,
        width: Option<f64>,
        height: Option<f64>,
        color: Option<Color>,
        stroke: u32,
    },
    Header {
        text: String,
        width: Option<f64>,
        height: f64,
        font_size: u32,
        color: Option<Color>,
    },
    Label {
        text: String,
        width: Option<f64>,
        height: f64,
        font_size: u32,
        h_align: String,
        color: Option<Color>,
    },
    Control(UiControlSpec),
    Spacer {
        width: f64,
        height: f64,
    },
}

/// A declarative Q-SYS page (`UI.Page("Main", ...)`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPageSpec {
    pub name: String,
    pub children: Vec<UiNode>,
    pub padding: f64,
    pub gap: f64,
    pub width: Option<f64>,
}

impl UiPageSpec {
    /// Solves the 2-pass Flexbox/Box layout into concrete `(x, y)` positions and `(w, h)` sizes
    /// ready for `GetControlLayout(props)` emission and the live web preview.
    pub fn solve(&self) -> PageLayout {
        let root_node = UiNode::Col {
            children: self.children.clone(),
            gap: self.gap,
            padding: self.padding,
        };

        let measured = measure_node(&root_node);
        let total_width = self.width.unwrap_or_else(|| measured.width.max(260.0));
        let total_height = measured.height.max(80.0);

        let mut controls = Vec::new();
        let mut graphics = Vec::new();

        arrange_node(
            &root_node,
            0.0,
            0.0,
            total_width,
            &mut controls,
            &mut graphics,
        );

        PageLayout {
            name: self.name.clone(),
            size: Size::new(total_width, total_height),
            controls,
            graphics,
        }
    }
}

fn measure_node(node: &UiNode) -> Size {
    match node {
        UiNode::Spacer { width, height } => Size::new(*width, *height),

        UiNode::Header { width, height, .. } => Size::new(width.unwrap_or(220.0), *height),

        UiNode::Label {
            text,
            width,
            height,
            ..
        } => {
            let auto_w = ((text.chars().count() as f64) * 7.2 + 12.0).max(56.0);
            Size::new(width.unwrap_or(auto_w), *height)
        }

        UiNode::Control(spec) => {
            let def_size = spec.style.default_size();
            let ctl_w = spec.width.unwrap_or(def_size.width);
            let ctl_h = spec.height.unwrap_or(def_size.height);

            if spec.label.is_some() {
                match spec.label_placement {
                    LabelPlacement::Top => Size::new(ctl_w.max(64.0), ctl_h + 20.0),
                    LabelPlacement::Left { width: lbl_w } => {
                        Size::new(lbl_w + 8.0 + ctl_w, ctl_h.max(22.0))
                    }
                }
            } else {
                Size::new(ctl_w, ctl_h)
            }
        }

        UiNode::Row {
            children,
            gap,
            padding,
        } => {
            let mut total_w = 0.0;
            let mut max_h: f64 = 0.0;
            for (idx, child) in children.iter().enumerate() {
                let sz = measure_node(child);
                if idx > 0 {
                    total_w += *gap;
                }
                total_w += sz.width;
                max_h = max_h.max(sz.height);
            }
            Size::new(total_w + 2.0 * padding, max_h + 2.0 * padding)
        }

        UiNode::Col {
            children,
            gap,
            padding,
        } => {
            let mut max_w: f64 = 0.0;
            let mut total_h = 0.0;
            for (idx, child) in children.iter().enumerate() {
                let sz = measure_node(child);
                if idx > 0 {
                    total_h += *gap;
                }
                max_w = max_w.max(sz.width);
                total_h += sz.height;
            }
            Size::new(max_w + 2.0 * padding, total_h + 2.0 * padding)
        }

        UiNode::GroupBox {
            title,
            children,
            direction,
            gap,
            padding,
            width,
            height,
            ..
        } => {
            let header_offset = if title.is_empty() { 0.0 } else { 24.0 };
            let inner = match direction {
                FlexDirection::Row => measure_node(&UiNode::Row {
                    children: children.clone(),
                    gap: *gap,
                    padding: 0.0,
                }),
                FlexDirection::Column => measure_node(&UiNode::Col {
                    children: children.clone(),
                    gap: *gap,
                    padding: 0.0,
                }),
            };
            let w = width.unwrap_or(inner.width + 2.0 * padding);
            let h = height.unwrap_or(inner.height + 2.0 * padding + header_offset);
            Size::new(w, h)
        }
    }
}

fn arrange_node(
    node: &UiNode,
    x: f64,
    y: f64,
    available_w: f64,
    controls: &mut Vec<ControlLayoutDef>,
    graphics: &mut Vec<GraphicItem>,
) -> Size {
    match node {
        UiNode::Spacer { width, height } => Size::new(*width, *height),

        UiNode::Header {
            text,
            width,
            height,
            font_size,
            color,
        } => {
            let w = width.unwrap_or(available_w);
            graphics.push(GraphicItem {
                kind: GraphicItemKind::Header {
                    text: text.clone(),
                    font_size: *font_size,
                },
                position: Position::new(x, y),
                size: Size::new(w, *height),
                color: color.clone(),
            });
            Size::new(w, *height)
        }

        UiNode::Label {
            text,
            width,
            height,
            font_size,
            h_align,
            color,
        } => {
            let measured = measure_node(node);
            let w = width.unwrap_or(measured.width);
            graphics.push(GraphicItem {
                kind: GraphicItemKind::Label {
                    text: text.clone(),
                    font_size: *font_size,
                    h_align: h_align.clone(),
                },
                position: Position::new(x, y),
                size: Size::new(w, *height),
                color: color.clone(),
            });
            Size::new(w, *height)
        }

        UiNode::Control(spec) => {
            let def_size = spec.style.default_size();
            let ctl_w = spec.width.unwrap_or(def_size.width);
            let ctl_h = spec.height.unwrap_or(def_size.height);

            let (ctl_x, ctl_y, total_w, total_h) = if let Some(ref lbl) = spec.label {
                match spec.label_placement {
                    LabelPlacement::Top => {
                        let box_w = ctl_w.max(64.0);
                        graphics.push(GraphicItem {
                            kind: GraphicItemKind::Label {
                                text: lbl.clone(),
                                font_size: 11,
                                h_align: "Left".to_string(),
                            },
                            position: Position::new(x, y),
                            size: Size::new(box_w, 18.0),
                            color: None,
                        });
                        (x, y + 20.0, box_w, ctl_h + 20.0)
                    }
                    LabelPlacement::Left { width: lbl_w } => {
                        let row_h = ctl_h.max(22.0);
                        let lbl_y = y + ((row_h - 18.0) / 2.0).max(0.0);
                        graphics.push(GraphicItem {
                            kind: GraphicItemKind::Label {
                                text: lbl.clone(),
                                font_size: 12,
                                h_align: "Left".to_string(),
                            },
                            position: Position::new(x, lbl_y),
                            size: Size::new(lbl_w, 18.0),
                            color: None,
                        });
                        (x + lbl_w + 8.0, y, lbl_w + 8.0 + ctl_w, row_h)
                    }
                }
            } else {
                (x, y, ctl_w, ctl_h)
            };

            controls.push(ControlLayoutDef {
                name: spec.control_name.clone(),
                pretty_name: spec.pretty_name.clone().or_else(|| spec.label.clone()),
                style: Some(spec.style.clone()),
                position: Position::new(ctl_x, ctl_y),
                size: Size::new(ctl_w, ctl_h),
                color: spec.color.clone(),
                margin: spec.margin,
                legend: spec.legend.clone(),
                is_disabled: spec.read_only,
                is_invisible: spec.invisible,
            });

            Size::new(total_w, total_h)
        }

        UiNode::Row {
            children,
            gap,
            padding,
        } => {
            let measured = measure_node(node);
            let inner_h = (measured.height - 2.0 * padding).max(0.0);
            let mut cur_x = x + padding;
            for child in children {
                let child_sz = measure_node(child);
                // Vertically align non-label/groupbox children if desired, or top-align cleanly
                let arranged = arrange_node(
                    child,
                    cur_x,
                    y + padding,
                    child_sz.width,
                    controls,
                    graphics,
                );
                cur_x += arranged.width + gap;
            }
            Size::new(
                (cur_x - gap + padding - x).max(measured.width),
                inner_h + 2.0 * padding,
            )
        }

        UiNode::Col {
            children,
            gap,
            padding,
        } => {
            let measured = measure_node(node);
            let box_w = available_w.max(measured.width);
            let inner_w = (box_w - 2.0 * padding).max(0.0);
            let mut cur_y = y + padding;
            for child in children {
                let arranged = arrange_node(child, x + padding, cur_y, inner_w, controls, graphics);
                cur_y += arranged.height + gap;
            }
            let total_h = if children.is_empty() {
                2.0 * padding
            } else {
                cur_y - gap + padding - y
            };
            Size::new(box_w, total_h)
        }

        UiNode::GroupBox {
            title,
            children,
            direction,
            gap,
            padding,
            width,
            height,
            color,
            stroke,
        } => {
            let measured = measure_node(node);
            let box_w = width.unwrap_or_else(|| available_w.max(measured.width));
            let box_h = height.unwrap_or(measured.height);
            let header_offset = if title.is_empty() { 0.0 } else { 24.0 };

            // Insert GroupBox graphic first so it renders behind its child controls
            graphics.push(GraphicItem {
                kind: GraphicItemKind::GroupBox {
                    text: title.clone(),
                    stroke: *stroke,
                },
                position: Position::new(x, y),
                size: Size::new(box_w, box_h),
                color: color.clone(),
            });

            let inner_w = (box_w - 2.0 * padding).max(0.0);
            let inner_x = x + padding;
            let inner_y = y + padding + header_offset;

            match direction {
                FlexDirection::Row => {
                    let row = UiNode::Row {
                        children: children.clone(),
                        gap: *gap,
                        padding: 0.0,
                    };
                    arrange_node(&row, inner_x, inner_y, inner_w, controls, graphics);
                }
                FlexDirection::Column => {
                    let col = UiNode::Col {
                        children: children.clone(),
                        gap: *gap,
                        padding: 0.0,
                    };
                    arrange_node(&col, inner_x, inner_y, inner_w, controls, graphics);
                }
            }

            Size::new(box_w, box_h)
        }
    }
}
