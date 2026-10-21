// Starter templates and default project structures for QSysPy Studio.

export const PYRIGHT_CONFIG = JSON.stringify(
  {
    include: ["src", "."],
    stubPath: "typings",
    pythonVersion: "3.11",
    typeCheckingMode: "basic",
  },
  null,
  2,
);

export const STARTER_FILES = {
  "src/matrix_mixer.py": `from qsys import Control, ControlHandle, Property, QSysPlugin
from dsp_math import GainScaler

class MatrixMixer(QSysPlugin):
    version = "1.0.0"
    author = "Audio Engineering"
    description = "8x8 Matrix Gain Processor"

    # Component Controls & Inspector Properties
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0, unit="dB")
    Mute = Control.Button(name="MuteAll", type="Toggle")
    Status = Control.Text(name="Status", default="Ready", type="ReadOnly", pins="Output")
    Channels = Property.Integer(name="ChannelCount", default=8, min=1, max=64)

    @Gain.EventHandler
    def on_gain(self, ctl: ControlHandle):
        """Fires whenever the MasterGain control changes."""
        scaler = GainScaler(-100.0, 20.0)
        if ctl.Value:
            # Control truthiness automatically guards against Lua's '0 is truthy'
            self.Mute.Boolean = False
            clamped = scaler.clamp(ctl.Value)
            self.Status.String = f"Active: {clamped} dB ({self.Channels} ch)"
`,

  "src/dsp_math.py": `# Shared DSP helper module imported by src/matrix_mixer.py
# QSysPy automatically bundles local project imports into the emitted .qplug / .lua output!

class GainScaler:
    def __init__(self, min_db: float, max_db: float) -> None:
        self.min_db = min_db
        self.max_db = max_db

    def clamp(self, value: float) -> float:
        if value < self.min_db:
            return self.min_db
        if value > self.max_db:
            return self.max_db
        return value
`,

  "src/wake_on_lan.py": `from qsys import Control, ControlHandle, QSysPlugin, UdpSocket

class WakeOnLan(QSysPlugin):
    version = "1.0.0"
    author = "AV Engineering"
    description = "Sends Wake-on-LAN magic packets over UDP broadcast"

    IPAddress = Control.IPAddress(default="255.255.255.255")
    MACAddress = Control.MACAddress(default="00:11:22:33:44:55")
    SendButton = Control.Button(name="Send", type="Trigger", pins="None")
    TriggerPin = Control.Button(name="Trigger", type="Trigger", pins="Input")
    Status = Control.Text(name="Status", default="Ready", pins="Output")

    def send_packet(self):
        sock = UdpSocket.New()
        sock.Open()

        # Binary magic packet payload (6x 0xFF + 16x MAC bytes)
        packet = "\\xff" * 6 + "\\x00\\x11\\x22\\x33\\x44\\x55" * 16

        sock.Send(self.IPAddress.String, 9, packet)
        sock.Close()

        self.Status.String = "Packet sent"

    @SendButton.EventHandler
    def on_ui_press(self, ctl: ControlHandle):
        """Fires when clicked from the UI panel."""
        self.send_packet()

    @TriggerPin.EventHandler
    def on_pin_pulse(self, ctl: ControlHandle):
        """Fires when pulsed via external schematic wire."""
        self.send_packet()
`,

  "src/room_controller.py": `from qsys import Control, ControlHandle, TextController, Timer

class RoomController(TextController):
    Power = Control.Button(name="SystemPower", type="Toggle")
    Status = Control.Text(name="RoomStatus", default="Standby")

    @Power.EventHandler
    def on_power(self, ctl: ControlHandle):
        if ctl.Boolean:
            self.Status.String = "Warming up..."
        else:
            self.Status.String = "Standby"
`,

  "src/procedural_dsp.py": `def calculate_gain(linear: float, offset: int) -> float:
    """Helper function to calculate dB offset."""
    return linear * 20.0 + offset

def run():
    channels = ["CH1", "CH2", "CH3"]
    for i in range(0, 3):
        msg = "Channel: " + channels[i]
        print(msg)
`,

  "pyrightconfig.json": PYRIGHT_CONFIG,

  "README.md": `# Q-SYS Starter Examples

Welcome to **QSysPy Studio**! This workspace provides a persistent virtual filesystem and supports bidirectional synchronization with local folders on your machine.

## Project Layout

- \`src/matrix_mixer.py\` — Example \`QSysPlugin\` (\`.qplug\`) importing \`GainScaler\` from \`src/dsp_math.py\`
- \`src/dsp_math.py\` — Reusable helper class bundled automatically when imported
- \`src/wake_on_lan.py\` — UDP network \`QSysPlugin\` (\`.qplug\`)
- \`src/room_controller.py\` — \`TextController\` script component (\`.lua\`)
- \`src/procedural_dsp.py\` — Standalone procedural Lua script (\`.lua\`)
- \`typings/qsys.pyi\` — PEP 561 Python type stubs for VS Code / Pyright / PyCharm
- \`pyrightconfig.json\` — Configures local Python language servers to use \`typings/qsys.pyi\`

## Working with Local Projects

1. **Open Local Folder**: Click **Local Folder** in the header to mount a directory from your computer using the File System Access API.
2. **Sync to Disk (\`Ctrl+S\`)**: Save source changes and compiled \`.qplug\` / \`.lua\` artifacts directly back to your local folder.
3. **Export Project**: Export your project as a \`.zip\` archive (including compiled outputs and \`.pyi\` stubs) or as a portable \`.qsysproj.json\` snapshot.
`,
};

export function createTemplateFiles(templateKind, projectName, stubsContent) {
  const safeClass =
    (projectName || "MyPlugin").replace(/[^a-zA-Z0-9_]/g, "").replace(/^[0-9]+/, "") || "MyPlugin";

  const baseFiles = {
    "pyrightconfig.json": PYRIGHT_CONFIG,
  };
  if (stubsContent) {
    baseFiles["typings/qsys.pyi"] = stubsContent;
  }

  if (templateKind === "starter") {
    return {
      files: {
        ...STARTER_FILES,
        ...baseFiles,
      },
      folders: ["src", "typings"],
      openTabs: ["src/matrix_mixer.py", "src/dsp_math.py", "src/wake_on_lan.py"],
      activeFile: "src/matrix_mixer.py",
    };
  }

  if (templateKind === "controller") {
    return {
      files: {
        ...baseFiles,
        "src/controller.py": `from qsys import Control, ControlHandle, TextController, Timer

class ${safeClass}(TextController):
    Trigger = Control.Button(name="Trigger", type="Trigger")
    Status = Control.Text(name="Status", default="Ready")

    @Trigger.EventHandler
    def on_trigger(self, ctl: ControlHandle):
        self.Status.String = "Triggered"
`,
        "README.md": `# ${projectName}\n\nQ-SYS TextController project created with QSysPy Studio.\n`,
      },
      folders: ["src", "typings"],
      openTabs: ["src/controller.py"],
      activeFile: "src/controller.py",
    };
  }

  if (templateKind === "empty") {
    return {
      files: {
        ...baseFiles,
        "main.py": `from qsys import Control, ControlHandle, QSysPlugin\n\n`,
      },
      folders: ["typings"],
      openTabs: ["main.py"],
      activeFile: "main.py",
    };
  }

  // Default: 'plugin'
  return {
    files: {
      ...baseFiles,
      "src/main.py": `from qsys import Control, ControlHandle, Property, QSysPlugin
from helpers import format_status

class ${safeClass}(QSysPlugin):
    version = "1.0.0"
    author = "Q-SYS Developer"
    description = "${projectName} Plugin"

    Enable = Control.Button(name="Enable", type="Toggle")
    Status = Control.Text(name="Status", default="Standby", type="ReadOnly", pins="Output")

    @Enable.EventHandler
    def on_enable(self, ctl: ControlHandle):
        if ctl.Boolean:
            self.Status.String = format_status("Online")
        else:
            self.Status.String = format_status("Standby")
`,
      "src/helpers.py": `def format_status(state: str) -> str:
    return f"System: {state}"
`,
      "README.md": `# ${projectName}\n\nQ-SYS Plugin (\`.qplug\`) project created with QSysPy Studio.\n`,
    },
    folders: ["src", "typings"],
    openTabs: ["src/main.py", "src/helpers.py"],
    activeFile: "src/main.py",
  };
}
