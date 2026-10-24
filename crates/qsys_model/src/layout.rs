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

/// A single `count_property * step` linear term in a dynamic layout coordinate or size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynLinearTerm {
    pub count_property: String,
    pub step: i64,
}

/// A linear expression `base + sum(count_property_i * step_i)` in points.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DynLinearSum {
    pub base: i64,
    pub terms: Vec<DynLinearTerm>,
}

impl DynLinearSum {
    pub fn static_val(base: i64) -> Self {
        Self {
            base,
            terms: Vec::new(),
        }
    }

    pub fn add_term(&mut self, count_property: String, step: i64) {
        if step == 0 {
            return;
        }
        if let Some(existing) = self
            .terms
            .iter_mut()
            .find(|t| t.count_property == count_property)
        {
            existing.step += step;
        } else {
            self.terms.push(DynLinearTerm {
                count_property,
                step,
            });
        }
    }

    pub fn add_sum(&self, other: &Self) -> Self {
        let mut out = self.clone();
        out.base += other.base;
        for t in &other.terms {
            out.add_term(t.count_property.clone(), t.step);
        }
        out
    }

    pub fn min_value_at_one(&self) -> i64 {
        let mut total = self.base;
        for term in &self.terms {
            let mult = parse_base_prop_and_multiplier(&term.count_property)
                .map(|(_, m)| m)
                .unwrap_or(1);
            total += term.step * mult;
        }
        total
    }

    pub fn normalized_rates(&self) -> Option<Vec<(String, i64)>> {
        let mut rates: Vec<(String, i64)> = Vec::new();
        for term in &self.terms {
            let (base_prop, mult) = parse_base_prop_and_multiplier(&term.count_property)?;
            let eff_rate = term.step * mult;
            if eff_rate <= 0 {
                return None;
            }
            if let Some(existing) = rates.iter_mut().find(|(p, _)| p == base_prop) {
                existing.1 += eff_rate;
            } else {
                rates.push((base_prop.to_string(), eff_rate));
            }
        }
        Some(rates)
    }

    /// Returns true if `self >= other` for all positive integer property values (`>= 1`).
    pub fn dominates(&self, other: &Self) -> bool {
        if self == other {
            return true;
        }
        if self.min_value_at_one() < other.min_value_at_one() {
            return false;
        }
        let Some(self_rates) = self.normalized_rates() else {
            return false;
        };
        let Some(other_rates) = other.normalized_rates() else {
            return false;
        };
        for (prop, other_rate) in &other_rates {
            let self_rate = self_rates
                .iter()
                .find(|(p, _)| p == prop)
                .map(|(_, r)| *r)
                .unwrap_or(0);
            if self_rate < *other_rate {
                return false;
            }
        }
        true
    }
}

fn parse_base_prop_and_multiplier(spec: &str) -> Option<(&str, i64)> {
    if !spec.contains("{PROP:") {
        return Some((spec, 1));
    }
    if let Some(inner) = spec
        .strip_prefix("{PROP:")
        .and_then(|s| s.strip_suffix('}'))
        && !inner.contains("{PROP:")
    {
        return Some((inner, 1));
    }
    if let Some((left, right)) = spec.split_once(" * ") {
        if let Some(prop) = left
            .strip_prefix("{PROP:")
            .and_then(|s| s.strip_suffix('}'))
            && !prop.contains("{PROP:")
            && let Ok(k) = right.trim().parse::<i64>()
            && k > 0
        {
            return Some((prop, k));
        }
        if let Some(prop) = right
            .strip_prefix("{PROP:")
            .and_then(|s| s.strip_suffix('}'))
            && !prop.contains("{PROP:")
            && let Ok(k) = left.trim().parse::<i64>()
            && k > 0
        {
            return Some((prop, k));
        }
    }
    None
}

/// A dynamic coordinate or dimension which is the maximum of an optional static minimum
/// and one or more `DynLinearSum` branches.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DynAxisExpr {
    pub min_static: Option<i64>,
    pub branches: Vec<DynLinearSum>,
}

impl DynAxisExpr {
    pub fn static_val(v: f64) -> Self {
        Self {
            min_static: Some(v.round() as i64),
            branches: Vec::new(),
        }
    }

    pub fn from_sum(sum: DynLinearSum) -> Self {
        if sum.terms.is_empty() {
            Self {
                min_static: Some(sum.base),
                branches: Vec::new(),
            }
        } else {
            Self {
                min_static: None,
                branches: vec![sum],
            }
        }
    }

    pub fn is_dynamic(&self) -> bool {
        !self.branches.is_empty()
    }

    pub fn into_opt(self) -> Option<Self> {
        if self.is_dynamic() { Some(self) } else { None }
    }

    pub fn add_const(&self, delta: i64) -> Self {
        if delta == 0 {
            return self.clone();
        }
        if self.branches.is_empty() {
            Self {
                min_static: Some(self.min_static.unwrap_or(0) + delta),
                branches: Vec::new(),
            }
        } else {
            let mut out = Self {
                min_static: self.min_static.map(|m| m + delta),
                branches: self
                    .branches
                    .iter()
                    .map(|b| {
                        let mut c = b.clone();
                        c.base += delta;
                        c
                    })
                    .collect(),
            };
            out.normalize();
            out
        }
    }

    fn to_sums(&self) -> Vec<DynLinearSum> {
        if self.branches.is_empty() {
            vec![DynLinearSum::static_val(self.min_static.unwrap_or(0))]
        } else {
            let mut sums = self.branches.clone();
            if let Some(ms) = self.min_static {
                sums.push(DynLinearSum::static_val(ms));
            }
            sums
        }
    }

    pub fn seq_add(&self, other: &Self) -> Self {
        if !self.is_dynamic() && !other.is_dynamic() {
            return Self {
                min_static: Some(self.min_static.unwrap_or(0) + other.min_static.unwrap_or(0)),
                branches: Vec::new(),
            };
        }
        let left_sums = self.to_sums();
        let right_sums = other.to_sums();
        let mut min_static: Option<i64> = None;
        let mut branches = Vec::new();
        for l in &left_sums {
            for r in &right_sums {
                let combined = l.add_sum(r);
                if combined.terms.is_empty() {
                    min_static = Some(
                        min_static
                            .map(|m| m.max(combined.base))
                            .unwrap_or(combined.base),
                    );
                } else {
                    branches.push(combined);
                }
            }
        }
        let mut out = Self {
            min_static,
            branches,
        };
        out.normalize();
        out
    }

    pub fn max_with(&self, other: &Self) -> Self {
        let min_static = match (self.min_static, other.min_static) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        let mut branches = self.branches.clone();
        branches.extend(other.branches.clone());
        let mut out = Self {
            min_static,
            branches,
        };
        out.normalize();
        out
    }

    pub fn normalize(&mut self) {
        let mut pruned: Vec<DynLinearSum> = Vec::new();
        for cand in &self.branches {
            if pruned.iter().any(|existing| existing.dominates(cand)) {
                continue;
            }
            pruned.retain(|existing| !cand.dominates(existing));
            pruned.push(cand.clone());
        }
        self.branches = pruned;

        if let Some(ms) = self.min_static
            && self
                .branches
                .iter()
                .any(|b| b.min_value_at_one() >= ms && b.normalized_rates().is_some())
        {
            self.min_static = None;
        }
    }
}

/// Specification for emitting a dynamic `for i = 1, props["..."].Value do` loop
/// in `GetControlLayout(props)` when a control's `count` is driven by a `Property`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicRepeatSpec {
    pub count_property: String,
    pub base_control_name: String,
    pub label_template: Option<String>,
    pub pretty_name_template: String,
    pub legend_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_lua_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pretty_name_lua_expr: Option<String>,
    pub step_x: f64,
    pub step_y: f64,
    pub label_offset_x: f64,
    pub label_offset_y: f64,
    pub label_width: f64,
    pub label_height: f64,
    pub label_font_size: u32,
}

/// Per-control graphical layout parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlLayoutDef {
    pub name: String,
    pub pretty_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pretty_name_lua_expr: Option<String>,
    pub style: Option<ControlStyle>,
    pub position: Position,
    pub size: Size,
    pub color: Option<Color>,
    pub margin: Option<f64>,
    pub legend: Option<String>,
    pub is_disabled: bool,
    pub is_invisible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_property: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_property: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_control_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_repeat: Option<DynamicRepeatSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_x: Option<DynAxisExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_y: Option<DynAxisExpr>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_lua_emit: bool,
}

impl ControlLayoutDef {
    pub fn new(name: impl Into<String>, position: Position, size: Size) -> Self {
        Self {
            name: name.into(),
            pretty_name: None,
            pretty_name_lua_expr: None,
            style: None,
            position,
            size,
            color: None,
            margin: None,
            legend: None,
            is_disabled: false,
            is_invisible: false,
            count_property: None,
            type_property: None,
            base_control_name: None,
            channel_index: None,
            dynamic_repeat: None,
            dyn_x: None,
            dyn_y: None,
            skip_lua_emit: false,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_lua_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_x: Option<DynAxisExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_y: Option<DynAxisExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_width: Option<DynAxisExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyn_height: Option<DynAxisExpr>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_lua_emit: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_property: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_property: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_control_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_label_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_pretty_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_legend_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_lua_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pretty_lua_expr: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_dynamic_repeat_head: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skip_lua_emit: bool,
}

/// Declarative HTML/Flexbox-like UI node authored in Python via `UI.*`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
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
        let (dyn_measured_w, _dyn_measured_h) =
            measure_dyn_node(&root_node, FlexDirection::Column, self.gap);
        let total_width = self.width.unwrap_or_else(|| measured.width.max(260.0));
        let total_height = measured.height.max(80.0);
        let dyn_total_width = match self.width {
            Some(w) => DynAxisExpr::static_val(w),
            None if dyn_measured_w.is_dynamic() => dyn_measured_w,
            None => DynAxisExpr::static_val(total_width),
        };

        let mut controls = Vec::new();
        let mut graphics = Vec::new();

        arrange_node(
            &root_node,
            0.0,
            0.0,
            total_width,
            &DynAxisExpr::static_val(0.0),
            &DynAxisExpr::static_val(0.0),
            &dyn_total_width,
            FlexDirection::Column,
            self.gap,
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

fn is_preview_tail_node(node: &UiNode) -> bool {
    matches!(
        node,
        UiNode::Control(spec) if spec.skip_lua_emit && !spec.is_dynamic_repeat_head
    )
}

fn measure_dyn_node(
    node: &UiNode,
    parent_dir: FlexDirection,
    parent_gap: f64,
) -> (DynAxisExpr, DynAxisExpr) {
    match node {
        UiNode::Spacer { width, height } => (
            DynAxisExpr::static_val(*width),
            DynAxisExpr::static_val(*height),
        ),

        UiNode::Header { width, height, .. } => {
            let dw = match (width, parent_dir) {
                (Some(w), _) => DynAxisExpr::static_val(*w),
                (None, FlexDirection::Column) => DynAxisExpr::default(),
                (None, FlexDirection::Row) => DynAxisExpr::static_val(220.0),
            };
            (dw, DynAxisExpr::static_val(*height))
        }

        UiNode::Label {
            text,
            width,
            height,
            ..
        } => {
            let auto_w = ((text.chars().count() as f64) * 7.2 + 12.0).max(56.0);
            (
                DynAxisExpr::static_val(width.unwrap_or(auto_w)),
                DynAxisExpr::static_val(*height),
            )
        }

        UiNode::Control(spec) => {
            let sz = measure_node(node);
            if spec.is_dynamic_repeat_head
                && let Some(prop_name) = &spec.count_property
            {
                match parent_dir {
                    FlexDirection::Row => {
                        let step_x = (sz.width + parent_gap).round() as i64;
                        let mut sum = DynLinearSum::static_val(-(parent_gap.round() as i64));
                        sum.add_term(prop_name.clone(), step_x);
                        (
                            DynAxisExpr::from_sum(sum),
                            DynAxisExpr::static_val(sz.height),
                        )
                    }
                    FlexDirection::Column => {
                        let step_y = (sz.height + parent_gap).round() as i64;
                        let mut sum = DynLinearSum::static_val(-(parent_gap.round() as i64));
                        sum.add_term(prop_name.clone(), step_y);
                        (
                            DynAxisExpr::static_val(sz.width),
                            DynAxisExpr::from_sum(sum),
                        )
                    }
                }
            } else {
                (
                    DynAxisExpr::static_val(sz.width),
                    DynAxisExpr::static_val(sz.height),
                )
            }
        }

        UiNode::Row {
            children,
            gap,
            padding,
        } => {
            let mut total_w = DynAxisExpr::static_val(0.0);
            let mut max_h = DynAxisExpr::default();
            let mut first = true;
            for child in children.iter().filter(|c| !is_preview_tail_node(c)) {
                let (cw, ch) = measure_dyn_node(child, FlexDirection::Row, *gap);
                if !first {
                    total_w = total_w.add_const(gap.round() as i64);
                }
                first = false;
                total_w = total_w.seq_add(&cw);
                max_h = max_h.max_with(&ch);
            }
            let pad2 = (2.0 * padding).round() as i64;
            if max_h.min_static.is_none() && max_h.branches.is_empty() {
                max_h = DynAxisExpr::static_val(0.0);
            }
            (total_w.add_const(pad2), max_h.add_const(pad2))
        }

        UiNode::Col {
            children,
            gap,
            padding,
        } => {
            let mut max_w = DynAxisExpr::default();
            let mut total_h = DynAxisExpr::static_val(0.0);
            let mut first = true;
            for child in children.iter().filter(|c| !is_preview_tail_node(c)) {
                let (cw, ch) = measure_dyn_node(child, FlexDirection::Column, *gap);
                if !first {
                    total_h = total_h.add_const(gap.round() as i64);
                }
                first = false;
                max_w = max_w.max_with(&cw);
                total_h = total_h.seq_add(&ch);
            }
            let pad2 = (2.0 * padding).round() as i64;
            if max_w.min_static.is_none() && max_w.branches.is_empty() {
                max_w = DynAxisExpr::static_val(0.0);
            }
            (max_w.add_const(pad2), total_h.add_const(pad2))
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
            let (inner_w, inner_h) = match direction {
                FlexDirection::Row => measure_dyn_node(
                    &UiNode::Row {
                        children: children.clone(),
                        gap: *gap,
                        padding: 0.0,
                    },
                    FlexDirection::Row,
                    *gap,
                ),
                FlexDirection::Column => measure_dyn_node(
                    &UiNode::Col {
                        children: children.clone(),
                        gap: *gap,
                        padding: 0.0,
                    },
                    FlexDirection::Column,
                    *gap,
                ),
            };
            let pad2 = (2.0 * padding).round() as i64;
            let h_extra = (2.0 * padding + header_offset).round() as i64;
            let dw = match width {
                Some(w) => DynAxisExpr::static_val(*w),
                None => inner_w.add_const(pad2),
            };
            let dh = match height {
                Some(h) => DynAxisExpr::static_val(*h),
                None => inner_h.add_const(h_extra),
            };
            (dw, dh)
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

#[allow(clippy::too_many_arguments)]
fn arrange_node(
    node: &UiNode,
    x: f64,
    y: f64,
    available_w: f64,
    dyn_x: &DynAxisExpr,
    dyn_y: &DynAxisExpr,
    dyn_available_w: &DynAxisExpr,
    parent_dir: FlexDirection,
    parent_gap: f64,
    controls: &mut Vec<ControlLayoutDef>,
    graphics: &mut Vec<GraphicItem>,
) -> (Size, DynAxisExpr, DynAxisExpr) {
    match node {
        UiNode::Spacer { width, height } => (
            Size::new(*width, *height),
            DynAxisExpr::static_val(*width),
            DynAxisExpr::static_val(*height),
        ),

        UiNode::Header {
            text,
            width,
            height,
            font_size,
            color,
        } => {
            let w = width.unwrap_or(available_w);
            let dyn_w = match width {
                Some(explicit_w) => DynAxisExpr::static_val(*explicit_w),
                None => dyn_available_w.clone(),
            };
            let dyn_h = DynAxisExpr::static_val(*height);
            graphics.push(GraphicItem {
                kind: GraphicItemKind::Header {
                    text: text.clone(),
                    font_size: *font_size,
                },
                position: Position::new(x, y),
                size: Size::new(w, *height),
                text_lua_expr: None,
                dyn_x: dyn_x.clone().into_opt(),
                dyn_y: dyn_y.clone().into_opt(),
                dyn_width: dyn_w.clone().into_opt(),
                dyn_height: None,
                color: color.clone(),
                skip_lua_emit: false,
            });
            (Size::new(w, *height), dyn_w, dyn_h)
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
                text_lua_expr: None,
                dyn_x: dyn_x.clone().into_opt(),
                dyn_y: dyn_y.clone().into_opt(),
                dyn_width: None,
                dyn_height: None,
                color: color.clone(),
                skip_lua_emit: false,
            });
            (
                Size::new(w, *height),
                DynAxisExpr::static_val(w),
                DynAxisExpr::static_val(*height),
            )
        }

        UiNode::Control(spec) => {
            let def_size = spec.style.default_size();
            let ctl_w = spec.width.unwrap_or(def_size.width);
            let ctl_h = spec.height.unwrap_or(def_size.height);
            let skip_lbl_lua = spec.skip_lua_emit || spec.is_dynamic_repeat_head;

            let (ctl_x, ctl_y, ctl_dyn_x, ctl_dyn_y, total_w, total_h, lbl_info) =
                if let Some(ref lbl) = spec.label {
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
                                text_lua_expr: spec.label_lua_expr.clone(),
                                dyn_x: dyn_x.clone().into_opt(),
                                dyn_y: dyn_y.clone().into_opt(),
                                dyn_width: None,
                                dyn_height: None,
                                color: None,
                                skip_lua_emit: skip_lbl_lua,
                            });
                            (
                                x,
                                y + 20.0,
                                dyn_x.clone(),
                                dyn_y.add_const(20),
                                box_w,
                                ctl_h + 20.0,
                                Some((x, y, box_w, 18.0, 11u32)),
                            )
                        }
                        LabelPlacement::Left { width: lbl_w } => {
                            let row_h = ctl_h.max(22.0);
                            let lbl_y_off = ((row_h - 18.0) / 2.0).max(0.0);
                            let lbl_y = y + lbl_y_off;
                            graphics.push(GraphicItem {
                                kind: GraphicItemKind::Label {
                                    text: lbl.clone(),
                                    font_size: 12,
                                    h_align: "Left".to_string(),
                                },
                                position: Position::new(x, lbl_y),
                                size: Size::new(lbl_w, 18.0),
                                text_lua_expr: spec.label_lua_expr.clone(),
                                dyn_x: dyn_x.clone().into_opt(),
                                dyn_y: dyn_y.add_const(lbl_y_off.round() as i64).into_opt(),
                                dyn_width: None,
                                dyn_height: None,
                                color: None,
                                skip_lua_emit: skip_lbl_lua,
                            });
                            (
                                x + lbl_w + 8.0,
                                y,
                                dyn_x.add_const((lbl_w + 8.0).round() as i64),
                                dyn_y.clone(),
                                lbl_w + 8.0 + ctl_w,
                                row_h,
                                Some((x, lbl_y, lbl_w, 18.0, 12u32)),
                            )
                        }
                    }
                } else {
                    (x, y, dyn_x.clone(), dyn_y.clone(), ctl_w, ctl_h, None)
                };

            let dynamic_repeat = if spec.is_dynamic_repeat_head
                && let (Some(prop_name), Some(base_name)) =
                    (&spec.count_property, &spec.base_control_name)
            {
                let step_x = if parent_dir == FlexDirection::Row {
                    total_w + parent_gap
                } else {
                    0.0
                };
                let step_y = if parent_dir == FlexDirection::Column {
                    total_h + parent_gap
                } else {
                    0.0
                };
                let (lbl_ox, lbl_oy, lbl_w, lbl_h, lbl_fs) =
                    if let Some((lx, ly, lw, lh, lfs)) = lbl_info {
                        (lx - ctl_x, ly - ctl_y, lw, lh, lfs)
                    } else {
                        (0.0, 0.0, 0.0, 0.0, 11)
                    };
                let pretty_tmpl = spec
                    .dynamic_pretty_template
                    .clone()
                    .or_else(|| spec.pretty_name.clone())
                    .or_else(|| spec.dynamic_label_template.clone())
                    .unwrap_or_else(|| base_name.clone());
                Some(DynamicRepeatSpec {
                    count_property: prop_name.clone(),
                    base_control_name: base_name.clone(),
                    label_template: spec.dynamic_label_template.clone(),
                    pretty_name_template: pretty_tmpl,
                    legend_template: spec.dynamic_legend_template.clone(),
                    label_lua_expr: spec.label_lua_expr.clone(),
                    pretty_name_lua_expr: spec.pretty_lua_expr.clone(),
                    step_x,
                    step_y,
                    label_offset_x: lbl_ox,
                    label_offset_y: lbl_oy,
                    label_width: lbl_w,
                    label_height: lbl_h,
                    label_font_size: lbl_fs,
                })
            } else {
                None
            };

            controls.push(ControlLayoutDef {
                name: spec.control_name.clone(),
                pretty_name: spec.pretty_name.clone().or_else(|| spec.label.clone()),
                pretty_name_lua_expr: spec
                    .pretty_lua_expr
                    .clone()
                    .or_else(|| spec.label_lua_expr.clone()),
                style: Some(spec.style.clone()),
                position: Position::new(ctl_x, ctl_y),
                size: Size::new(ctl_w, ctl_h),
                dyn_x: ctl_dyn_x.into_opt(),
                dyn_y: ctl_dyn_y.into_opt(),
                color: spec.color.clone(),
                margin: spec.margin,
                legend: spec.legend.clone(),
                is_disabled: spec.read_only,
                is_invisible: spec.invisible,
                count_property: spec.count_property.clone(),
                type_property: spec.type_property.clone(),
                base_control_name: spec.base_control_name.clone(),
                channel_index: spec.channel_index,
                dynamic_repeat,
                skip_lua_emit: spec.skip_lua_emit,
            });

            let (dyn_w, dyn_h) = measure_dyn_node(node, parent_dir, parent_gap);
            (Size::new(total_w, total_h), dyn_w, dyn_h)
        }

        UiNode::Row {
            children,
            gap,
            padding,
        } => {
            let measured = measure_node(node);
            let (dyn_measured_w, dyn_measured_h) = measure_dyn_node(node, parent_dir, parent_gap);
            let inner_h = (measured.height - 2.0 * padding).max(0.0);
            let pad_i = padding.round() as i64;
            let child_dyn_y = dyn_y.add_const(pad_i);
            let mut cur_x = x + padding;
            let mut cur_dyn_x = dyn_x.add_const(pad_i);
            for child in children {
                let is_tail = is_preview_tail_node(child);
                let child_sz = measure_node(child);
                let (child_dw_meas, _) = measure_dyn_node(child, FlexDirection::Row, *gap);
                let (arranged, child_dw, _child_dh) = arrange_node(
                    child,
                    cur_x,
                    y + padding,
                    child_sz.width,
                    &cur_dyn_x,
                    &child_dyn_y,
                    &child_dw_meas,
                    FlexDirection::Row,
                    *gap,
                    controls,
                    graphics,
                );
                cur_x += arranged.width + gap;
                if !is_tail {
                    cur_dyn_x = cur_dyn_x.seq_add(&child_dw).add_const(gap.round() as i64);
                }
            }
            (
                Size::new(
                    (cur_x - gap + padding - x).max(measured.width),
                    inner_h + 2.0 * padding,
                ),
                dyn_measured_w,
                dyn_measured_h,
            )
        }

        UiNode::Col {
            children,
            gap,
            padding,
        } => {
            let measured = measure_node(node);
            let (dyn_measured_w, _dyn_measured_h) = measure_dyn_node(node, parent_dir, parent_gap);
            let box_w = available_w.max(measured.width);
            let dyn_box_w = dyn_available_w.max_with(&dyn_measured_w);
            let inner_w = (box_w - 2.0 * padding).max(0.0);
            let pad_i = padding.round() as i64;
            let dyn_inner_w = dyn_box_w.add_const(-2 * pad_i);
            let child_dyn_x = dyn_x.add_const(pad_i);
            let mut cur_y = y + padding;
            let mut cur_dyn_y = dyn_y.add_const(pad_i);
            let mut total_dyn_h = DynAxisExpr::static_val(0.0);
            let mut first_dyn = true;
            for child in children {
                let is_tail = is_preview_tail_node(child);
                let (arranged, _child_dw, child_dh) = arrange_node(
                    child,
                    x + padding,
                    cur_y,
                    inner_w,
                    &child_dyn_x,
                    &cur_dyn_y,
                    &dyn_inner_w,
                    FlexDirection::Column,
                    *gap,
                    controls,
                    graphics,
                );
                cur_y += arranged.height + gap;
                if !is_tail {
                    let gap_i = gap.round() as i64;
                    cur_dyn_y = cur_dyn_y.seq_add(&child_dh).add_const(gap_i);
                    if !first_dyn {
                        total_dyn_h = total_dyn_h.add_const(gap_i);
                    }
                    first_dyn = false;
                    total_dyn_h = total_dyn_h.seq_add(&child_dh);
                }
            }
            let total_h = if children.is_empty() {
                2.0 * padding
            } else {
                cur_y - gap + padding - y
            };
            let dyn_total_h = total_dyn_h.add_const(2 * pad_i);
            (Size::new(box_w, total_h), dyn_box_w, dyn_total_h)
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
            let (dyn_measured_w, dyn_measured_h) = measure_dyn_node(node, parent_dir, parent_gap);
            let box_w = width.unwrap_or_else(|| available_w.max(measured.width));
            let box_h = height.unwrap_or(measured.height);
            let dyn_box_w = match width {
                Some(w) => DynAxisExpr::static_val(*w),
                None => dyn_available_w.max_with(&dyn_measured_w),
            };
            let dyn_box_h = match height {
                Some(h) => DynAxisExpr::static_val(*h),
                None => dyn_measured_h,
            };
            let header_offset = if title.is_empty() { 0.0 } else { 24.0 };

            // Insert GroupBox graphic first so it renders behind its child controls
            graphics.push(GraphicItem {
                kind: GraphicItemKind::GroupBox {
                    text: title.clone(),
                    stroke: *stroke,
                },
                position: Position::new(x, y),
                size: Size::new(box_w, box_h),
                text_lua_expr: None,
                dyn_x: dyn_x.clone().into_opt(),
                dyn_y: dyn_y.clone().into_opt(),
                dyn_width: dyn_box_w.clone().into_opt(),
                dyn_height: dyn_box_h.clone().into_opt(),
                color: color.clone(),
                skip_lua_emit: false,
            });

            let inner_w = (box_w - 2.0 * padding).max(0.0);
            let inner_x = x + padding;
            let inner_y = y + padding + header_offset;
            let pad_i = padding.round() as i64;
            let header_i = header_offset.round() as i64;
            let dyn_inner_w = dyn_box_w.add_const(-2 * pad_i);
            let dyn_inner_x = dyn_x.add_const(pad_i);
            let dyn_inner_y = dyn_y.add_const(pad_i + header_i);

            match direction {
                FlexDirection::Row => {
                    let row = UiNode::Row {
                        children: children.clone(),
                        gap: *gap,
                        padding: 0.0,
                    };
                    arrange_node(
                        &row,
                        inner_x,
                        inner_y,
                        inner_w,
                        &dyn_inner_x,
                        &dyn_inner_y,
                        &dyn_inner_w,
                        FlexDirection::Row,
                        *gap,
                        controls,
                        graphics,
                    );
                }
                FlexDirection::Column => {
                    let col = UiNode::Col {
                        children: children.clone(),
                        gap: *gap,
                        padding: 0.0,
                    };
                    arrange_node(
                        &col,
                        inner_x,
                        inner_y,
                        inner_w,
                        &dyn_inner_x,
                        &dyn_inner_y,
                        &dyn_inner_w,
                        FlexDirection::Column,
                        *gap,
                        controls,
                        graphics,
                    );
                }
            }

            (Size::new(box_w, box_h), dyn_box_w, dyn_box_h)
        }
    }
}
