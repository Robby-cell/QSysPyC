use std::fs;
use std::path::Path;

use qsys_runtime::{QSysRuntime, RuntimeEnv, pyi};

/// Hand-written stubs for the QSysPy declaration DSL (`QSysPlugin`, `Control.*`, `Property.*`).
/// Runtime globals and native object types are generated from the `RuntimeEnv` (see [`generate_pyi`]).
const DSL_PYI: &str = r#"# Python type stubs for QSysPy (Q-SYS Core Compiler)
# Compatible with Pyright, Pylance, mypy, and PyCharm

from typing import Any, Callable, Literal, TypeVar, overload

F = TypeVar("F", bound=Callable[..., Any])

PinStyle = Literal["None", "Input", "Output", "Both"]
ButtonType = Literal["Momentary", "Toggle", "Trigger"]
TextType = Literal["Text", "ReadOnly"]
ControlUnit = Literal["Float", "Integer", "dB", "Hz", "Percent", "Seconds", "Pan"]

class ControlHandle:
    """Runtime handle for a Q-SYS schematic or hardware control."""

    Value: float
    """Current continuous numeric value of the control."""

    String: str
    """Current text string representation of the control value."""

    Boolean: bool
    """Current boolean/binary state of the control (True/False)."""

    Values: list[float]
    """Array of numeric values for multi-channel controls."""

    Strings: list[str]
    """Array of text strings for multi-channel controls."""

    IsDisabled: bool
    """Whether the control is grayed out and disabled in the UI."""

    IsInvisible: bool
    """Whether the control is hidden from the UI."""

    Color: str
    """Hex color or named color of the control widget in the UI."""

    Legend: str
    """Text label displayed on the control button or knob."""

    RampTime: float
    """Time in seconds over which the control ramps to new values."""

    @classmethod
    def EventHandler(cls, func: F) -> F:
        """
        Decorator that binds a method as an event handler for this control.

        Example:
            @Gain.EventHandler
            def on_gain(self, ctl: ControlHandle):
                print(ctl.Value)
        """
        ...

class Control:
    """Factory declarations for Q-SYS controls."""

    @staticmethod
    def Float(
        name: str | None = None,
        min: float = 0.0,
        max: float = 1.0,
        default: float = 0.0,
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Continuous float control (Knob or continuous slider)."""
        ...

    @staticmethod
    def Knob(
        name: str | None = None,
        min: float = 0.0,
        max: float = 1.0,
        default: float = 0.0,
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Rotary continuous knob control."""
        ...

    @staticmethod
    def Fader(
        name: str | None = None,
        min: float = 0.0,
        max: float = 1.0,
        default: float = 0.0,
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Linear fader slider control."""
        ...

    @staticmethod
    def Button(
        name: str | None = None,
        type: ButtonType = "Momentary",
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Push button control with configurable behavior (Momentary, Toggle, Trigger)."""
        ...

    @staticmethod
    def Boolean(
        name: str | None = None,
        type: ButtonType = "Toggle",
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Binary toggle button control."""
        ...

    @staticmethod
    def Text(
        name: str | None = None,
        default: str = "",
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Text display or input box control."""
        ...

    @staticmethod
    def Indicator(
        name: str | None = None,
        pins: PinStyle = "Both",
        count: int = 1,
    ) -> ControlHandle:
        """Read-only LED or meter display indicator."""
        ...

class PropertyHandle:
    """Handle to a plugin property value configured in Q-SYS Designer properties panel."""

    Value: Any

class Property:
    """Factory declarations for Q-SYS Plugin Properties."""

    @staticmethod
    def Integer(
        name: str | None = None,
        default: int = 0,
        min: int | None = None,
        max: int | None = None,
        description: str | None = None,
    ) -> PropertyHandle:
        """Integer property exposed in the Q-SYS Properties inspector panel."""
        ...

    @staticmethod
    def Double(
        name: str | None = None,
        default: float = 0.0,
        min: float | None = None,
        max: float | None = None,
        description: str | None = None,
    ) -> PropertyHandle:
        """Double-precision float property exposed in the Q-SYS Properties inspector panel."""
        ...

    @staticmethod
    def Float(
        name: str | None = None,
        default: float = 0.0,
        min: float | None = None,
        max: float | None = None,
        description: str | None = None,
    ) -> PropertyHandle:
        """Float property exposed in the Q-SYS Properties inspector panel."""
        ...

    @staticmethod
    def String(
        name: str | None = None,
        default: str = "",
        description: str | None = None,
    ) -> PropertyHandle:
        """Text string property exposed in the Q-SYS Properties inspector panel."""
        ...

    @staticmethod
    def Enum(
        choices: list[str],
        default: str | None = None,
        name: str | None = None,
        description: str | None = None,
    ) -> PropertyHandle:
        """Dropdown enumeration property exposed in the Q-SYS Properties inspector panel."""
        ...

    @staticmethod
    def Boolean(
        default: bool = False,
        name: str | None = None,
        description: str | None = None,
    ) -> PropertyHandle:
        """Boolean checkbox property exposed in the Q-SYS Properties inspector panel."""
        ...

class QSysPlugin:
    """Base class for all Q-SYS Plugins (.qplug)."""

    name: str
    version: str
    id: str
    author: str
    description: str | None
    manufacturer: str | None
    show_debug: bool

    def __getattr__(self, name: str) -> ControlHandle:
        """Access declared controls on self (e.g. self.Gain.Value)."""
        ...

class TextController:
    """Base class for standalone Text Controller / Script Component blocks."""

    def __getattr__(self, name: str) -> ControlHandle:
        """Access bound pin controls on self."""
        ...

"#;

/// Builds the complete `qsys.pyi` source: the DSL stubs followed by every type and global
/// registered by the [`QSysRuntime`] plugin in `env`.
pub fn generate_pyi(env: &RuntimeEnv) -> String {
    let runtime = pyi::render_pyi(env, |origin| origin == QSysRuntime::NAME);
    format!(
        "{DSL_PYI}# --- Q-SYS runtime (generated from the `{}` runtime plugin) ---\n\n{runtime}",
        QSysRuntime::NAME
    )
}

/// Builds the complete `qsys.pyi` source for the default Q-SYS runtime environment.
pub fn generate_default_pyi() -> String {
    generate_pyi(&RuntimeEnv::qsys())
}

/// Generates standard PEP 561 stub files (`qsys/__init__.pyi`, `qsys/py.typed`, `qsys.pyi`).
pub fn write_stubs_to_dir(out_dir: &Path) -> std::io::Result<()> {
    let pyi_source = generate_default_pyi();

    // 1. Write standalone qsys.pyi (for single-file imports)
    fs::create_dir_all(out_dir)?;
    let single_pyi = out_dir.join("qsys.pyi");
    fs::write(single_pyi, &pyi_source)?;

    // 2. Write package layout: qsys/__init__.pyi and qsys/py.typed (PEP 561)
    let pkg_dir = out_dir.join("qsys");
    fs::create_dir_all(&pkg_dir)?;
    fs::write(pkg_dir.join("__init__.pyi"), &pyi_source)?;
    fs::write(pkg_dir.join("py.typed"), "")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_stubs_cover_dsl_and_runtime() {
        let pyi = generate_pyi(&RuntimeEnv::qsys());
        assert!(pyi.starts_with("# Python type stubs for QSysPy"));
        assert!(pyi.contains("class QSysPlugin:"));
        assert!(pyi.contains("class UdpSocketHandle:"));
        assert!(pyi.contains("    def Send(self, ip: str, port: int, data: str) -> None:"));
        assert!(pyi.contains("class UdpSocket:\n"));
        assert!(pyi.contains("Properties: dict[str, PropertyHandle]"));
    }
}
