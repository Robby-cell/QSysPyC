# QSysPy (`qsyspyc`)

[![Rust](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Target](https://img.shields.io/badge/Target-Q--SYS%20Lua%205.3-green.svg)](https://www.qsys.com/)

A strict, deterministic Python-to-Lua compiler and toolchain designed specifically for the **Q-SYS Core audio/DSP platform**.

`qsyspyc` allows AV systems programmers to write modern, type-annotated Python to produce clean, idiomatic, and valid Q-SYS Designer `.qplug` plugins and `.lua` control scripts.

---

## The Core Philosophy

> **"Unsupported Python must produce an actionable diagnostic, never an approximation."**

Q-SYS Core processors operate mission-critical audio, video, and control engines. Naive transpilation (substituting operators, flattening control flow, or ignoring semantics) can cause silent runtime crashes or corrupt DSP states. 

`qsyspyc` enforces total soundness:
* **Truthiness Protection**: In Lua, `0` and `""` are truthy. In Python, they are falsy. QSysPy’s semantic engine infers operand types and lowers `if ctl.Value:` to `if (ctl.Value ~= 0) then` and `if name:` to `if (#name > 0) then`.
* **Zero Silent Drops**: Chained assignments (`a = b = 1`), unsupported operators, and tuple unpacking in for-loops produce clear diagnostics (`E0101`–`E0205`) with line and column numbers.
* **Stable IDs & Fuzzy Member Checks**: References like `self.Master.Value` resolve to internal IDs (`ControlId`, `SymbolId`), and typos like `ctl.Valu` emit actionable suggestions (`did you mean 'Value'?`).

---

## Architecture & Workspace

The project is structured as a modular Cargo workspace:

```
crates/
├── qsys_model/        # Authoritative Q-SYS domain model (Controls, Properties, Pin styles, Layout)
├── qsys_diagnostics/  # Diagnostic catalog (E0001–E0205), span tracking, Monaco/LSP markers
├── qsys_parser/       # AST lowering from ruff_python_parser; strict syntax validation
├── qsys_sema/         # Scope management, symbol resolution, stable IDs, type inference
├── qsys_ir/           # Target-agnostic Intermediate Representation (Typed IR)
├── qsys_codegen_lua/  # Q-SYS Lua 5.3 code generator (PluginInfo, GetControls, runtime guard)
├── qsys_compiler/     # Unified orchestration driver (Compiler::compile, Compiler::check)
├── qsyspyc/           # Native CLI binary (build, check, gen-stubs)
└── qsys_wasm/         # WebAssembly bindings for in-browser compilation
web/                   # Monaco Editor-based static playground studio
```

---

## Quickstart

### Prerequisites

* [Rust](https://rustup.rs/) (2024 edition / Rust 1.85+)
* [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) *(optional, for WASM/Web Playground)*

### Building the CLI

```bash
# Clone the repository
git clone https://github.com/Robby-cell/qsyspy.git
cd qsyspy

# Build the compiler CLI
cargo build --release -p qsyspyc

# The binary will be available at:
./target/release/qsyspyc --help
```

### Running Tests

```bash
cargo test
```

---

## CLI Usage

### 1. Compile a Python Plugin to `.qplug`

```bash
# Automatically derives output name (e.g. mixer.qplug)
qsyspyc build mixer.py

# Or specify a custom output path:
qsyspyc build mixer.py -o dist/MatrixMixer.qplug
```

### 2. Validate Code Without Emitting Output

```bash
qsyspyc check mixer.py
```

If an error is found, `qsyspyc` prints formatted diagnostics with line numbers, code snippets, and suggestions:
```text
error[E0201]: Control 'ctl' has no member 'Valu'; did you mean 'Value'? at mixer.py:8:17
  = help: Declare controls using Control.<Type>() or check member casing.
```

### 3. Generate Type Stubs (`.pyi`) for VS Code / PyCharm

```bash
qsyspyc gen-stubs
```

This creates standard PEP 561 stub files (`typings/qsys/__init__.pyi`, `typings/qsys/py.typed`, `typings/qsys.pyi`). 

Add this to `.vscode/settings.json` for full IDE autocompletion:
```json
{
  "python.analysis.extraPaths": ["./typings"]
}
```

---

## Writing a Q-SYS Plugin in Python

```python
from qsys import QSysPlugin, Control, Property, ControlHandle, qsys


class MatrixMixer(QSysPlugin):
    version = "1.0.0"
    author = "Audio Systems Engineering"
    description = "8-Channel Matrix Gain Controller"

    # Declare component controls with wiring pin configuration
    Gain = Control.Float(
        name="MasterGain", min=-100.0, max=20.0, default=0.0, pins="Both"
    )
    Mute = Control.Button(name="MuteAll", type="Toggle", pins="Both")

    # Declare properties shown in the Q-SYS Designer properties panel
    Channels = Property.Integer(name="ChannelCount", default=8, min=1, max=64)

    @Gain.EventHandler
    def on_gain(self, ctl: ControlHandle):
        """Fires whenever MasterGain is modified in the UI or via an external pin."""
        if ctl.Value:
            # Type-inferred: ctl.Value != 0 guard is automatically emitted in Lua
            self.Mute.Boolean = False

    @Mute.EventHandler
    def on_mute(self, ctl: ControlHandle):
        # Escape hatch: Execute native Q-SYS Lua socket/library code directly
        qsys.lua("print('Mute toggled: ' .. tostring(Controls['MuteAll'].Boolean))")
```

When compiled with `qsyspyc build`, this produces:
* Complete `PluginInfo` metadata table.
* `GetProperties()` and `GetControls(props)` definitions matching Q-SYS plugin architecture.
* A default `GetControlLayout(props)` layout envelope.
* Runtime lifecycle code guarded by `if Controls then ... end`.
* Event handler bindings mapped to `Controls["MasterGain"].EventHandler`.

---

## Web Playground & Studio

The project includes an in-browser playground powered by Monaco Editor and WebAssembly.

### Running the Web Studio Locally

Compile the WASM crate
```bash
wasm-pack build crates/qsys_wasm --target web --out-dir ../../web/pkg
```

Serve the web folder
```bash
cd web
python3 -m http.server 3000
```

Or to serve with `npm`:
```bash
npx serve ./web
```

Open `http://localhost:3000` in any browser.

---

## Compiler Diagnostic Codes

| Code Range | Category | Examples |
|---|---|---|
| **E0001–E0099** | Syntax & Structure | Python syntax errors, unclosed parentheses, missing module body |
| **E0101–E0199** | Language Subset Guardrails | Unsupported `async/await`, generators (`yield`), match/case, chained assignment |
| **E0201–E0299** | Semantic & Q-SYS Binding | Unknown control members (`ctl.Valu`), missing `self`, invalid pin style |

---

## Contributing

We welcome issues, feedback, and pull requests!

### Development Workflow

1. Fork and clone the repository.
2. Verify that existing tests pass:
   ```bash
   cargo test
   ```
3. Format and lint your changes:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   ```

### Contribution Rules

1. **Strict Lowering**: Never add silent fallbacks in the parser or lowering phases. If a Python language feature cannot be represented in Q-SYS Lua with 100% semantic fidelity, emit an explicit diagnostic code in `crates/qsys_diagnostics/src/kind.rs`.
2. **ID Preservation**: When extending the semantic analyzer or IR, avoid passing raw identifier strings. Use or extend `ControlId`, `PropertyId`, `FunctionId`, or `SymbolId`.
3. **Tests Required**: Every bug fix or new feature must include integration tests under `crates/<crate_name>/tests/`.

---

## License

Dual-licensed under either:
* **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))
* **MIT License** ([LICENSE-MIT](LICENSE-MIT))

at your option.
