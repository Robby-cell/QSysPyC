import init, { compile_qsyspy } from './pkg/qsys_wasm.js';
import { registerLspFeatures } from './lsp.js';

let pyEditor, luaEditor;
let lastCompiledLua = "";
let lastProgramKind = "Plugin";

const EXAMPLES = {
  mixer: `from qsys import Control, Property, QSysPlugin, qsys

class MatrixMixer(QSysPlugin):
    version = "1.0.0"
    author = "Audio Engineering"
    description = "8x8 Matrix Gain Processor"

    # Component Controls
    Gain = Control.Float(name="MasterGain", min=-100.0, max=20.0, default=0.0)
    Mute = Control.Button(name="MuteAll", type="Toggle")
    Channels = Property.Integer(name="ChannelCount", default=8, min=1, max=64)

    @Gain.EventHandler
    def on_gain(self, ctl):
        """Fires whenever the MasterGain control changes."""
        if ctl.Value:
            # Control truthiness automatically guards against Lua's '0 is truthy'
            self.Mute.Boolean = False
`,
  wol: `from qsys import Control, QSysPlugin, qsys

class WakeOnLan(QSysPlugin):
    version = "1.0.0"
    author = "AV Engineering"
    description = "Sends Wake-on-LAN magic packets over UDP broadcast"

    Mac = Control.Text(name="MacAddress", default="00:11:22:33:44:55", pins="Both")
    Ip = Control.Text(name="BroadcastIP", default="255.255.255.255", pins="Both")
    Send = Control.Button(name="SendPacket", type="Trigger", pins="Both")
    Status = Control.Text(name="Status", default="Ready", pins="Output")

    @Send.EventHandler
    def on_send(self, ctl):
        self.Status.String = "Sending WoL Packet..."
        # We could use qsys.UdpSocket, but qsys.lua is to write Lua directly
        qsys.lua("""
            local sock = UdpSocket.New()
            sock:Open()
            sock:Send(Controls["BroadcastIP"].String, 9, string.rep(string.char(0xFF), 6))
            sock:Close()
        """)
`,
  script: `def calculate_gain(linear: float, offset: int) -> float:
    """Helper function to calculate dB offset."""
    return linear * 20.0 + offset

def run():
    channels = ["CH1", "CH2", "CH3"]
    msg = "Status: " + channels[0]
    print(msg)
`
};

async function main() {
  const statusText = document.getElementById('status-text');
  try {
    if (statusText) statusText.innerText = "Loading WASM...";
    await init();
    if (statusText) statusText.innerText = "WASM Loaded";
    setupMonaco();
  } catch (err) {
    console.error("Failed to load QSysPy WASM module:", err);
    if (statusText) statusText.innerText = "WASM Load Error";
    const luaContainer = document.getElementById('lua-editor');
    if (luaContainer) {
      luaContainer.innerHTML = `
        <div style="padding: 20px; color: #f14c4c; font-family: monospace; font-size: 13px;">
          <h3>Failed to load QSysPy WASM</h3>
          <p>${err.message || err}</p>
          <br>
          <p>Please recompile the WASM binary from the repository root:</p>
          <code>wasm-pack build crates/qsys_wasm --target web --out-dir web/pkg</code>
        </div>
      `;
    }
  }
}

function setupMonaco() {
  require.config({ paths: { vs: 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.45.0/min/vs' } });

  require(['vs/editor/editor.main'], function () {
    try {
      registerLspFeatures(monaco);
    } catch (e) {
      console.warn("LSP registration warning:", e);
    }

    // Python Input Editor
    pyEditor = monaco.editor.create(document.getElementById('python-editor'), {
      value: EXAMPLES.mixer,
      language: 'python',
      theme: 'vs-dark',
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 13,
      lineNumbers: 'on',
      scrollBeyondLastLine: false,
      tabSize: 4,
      suggestOnTriggerCharacters: true
    });

    // Lua Output Viewer
    luaEditor = monaco.editor.create(document.getElementById('lua-editor'), {
      value: '-- Compiling initial code...',
      language: 'lua',
      theme: 'vs-dark',
      readOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 13,
      lineNumbers: 'on',
      scrollBeyondLastLine: false
    });

    // Recompile on keystroke (debounced 150ms)
    let debounceTimer;
    pyEditor.onDidChangeModelContent(() => {
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(triggerCompile, 150);
    });

    // UI event listeners
    const exampleSelect = document.getElementById('example-select');
    if (exampleSelect) {
      exampleSelect.addEventListener('change', (e) => {
        if (EXAMPLES[e.target.value]) {
          pyEditor.setValue(EXAMPLES[e.target.value]);
        }
      });
    }

    const btnCopy = document.getElementById('btn-copy');
    if (btnCopy) {
      btnCopy.addEventListener('click', () => {
        navigator.clipboard.writeText(lastCompiledLua);
        alert('Lua code copied to clipboard!');
      });
    }

    const btnDownload = document.getElementById('btn-download');
    if (btnDownload) {
      btnDownload.addEventListener('click', () => {
        const ext = lastProgramKind === 'Plugin' ? 'qplug' : 'lua';
        const blob = new Blob([lastCompiledLua], { type: 'text/plain' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `output.${ext}`;
        a.click();
        URL.revokeObjectURL(url);
      });
    }

    // Run first compilation immediately
    triggerCompile();
  });
}

function triggerCompile() {
  if (!pyEditor || !luaEditor) return;

  const statusPill = document.getElementById('status-pill');
  const statusText = document.getElementById('status-text');
  const errorDrawer = document.getElementById('error-drawer');

  try {
    const code = pyEditor.getValue();
    if (!code.trim()) {
      luaEditor.setValue("-- Empty Python source file");
      if (statusText) statusText.innerText = "Empty";
      return;
    }

    const res = compile_qsyspy(code, "playground.py");
    const markers = res.markers ? JSON.parse(res.markers) : [];

    // Map compiler diagnostics to Monaco red squiggles
    if (monaco && pyEditor.getModel()) {
      const monacoMarkers = markers.map(m => ({
        code: m.code || "E0000",
        message: m.help ? `${m.message}\n💡 Help: ${m.help}` : m.message,
        severity: m.severity === 'error' ? monaco.MarkerSeverity.Error : monaco.MarkerSeverity.Warning,
        startLineNumber: m.startLineNumber || 1,
        startColumn: m.startColumn || 1,
        endLineNumber: m.endLineNumber || m.startLineNumber || 1,
        endColumn: m.endColumn || (m.startColumn + 1) || 2
      }));
      monaco.editor.setModelMarkers(pyEditor.getModel(), "qsyspy", monacoMarkers);
    }

    if (res.success) {
      const kind = res.target_kind || "Compiled";
      if (statusPill) statusPill.className = "status-badge ok";
      if (statusText) statusText.innerText = `Compiled (${kind})`;

      luaEditor.setValue(res.lua);
      lastCompiledLua = res.lua;
      lastProgramKind = kind;

      const outExt = document.getElementById('output-ext');
      const btnDl = document.getElementById('btn-download');
      if (outExt) outExt.innerText = kind === 'Plugin' ? '.qplug' : '.lua';
      if (btnDl) btnDl.innerText = kind === 'Plugin' ? 'Download .qplug' : 'Download .lua';
      if (errorDrawer) errorDrawer.style.display = "none";
    } else {
      if (statusPill) statusPill.className = "status-badge error";
      if (statusText) statusText.innerText = `${markers.length} Error(s)`;

      // SHOW COMPILATION ERRORS DIRECTLY IN THE LUA PANE INSTEAD OF STALLING
      const errorHeader = [
        "-- ============================================================",
        `-- COMPILATION FAILED: ${markers.length} Error(s)`,
        "-- ============================================================",
        ...markers.map(m => {
          const loc = `Line ${m.startLineNumber || '?'}:${m.startColumn || '?'}`;
          const help = m.help ? `\n--   Help: ${m.help}` : "";
          return `-- [${m.code || 'ERROR'}] ${loc} - ${m.message}${help}`;
        }),
        "-- ============================================================"
      ].join("\n");

      luaEditor.setValue(errorHeader);

      if (errorDrawer) {
        errorDrawer.innerHTML = markers.map(m => `
          <div class="error-item">
            <span class="code">${m.code || 'ERROR'}</span>
            <span>Line ${m.startLineNumber || '?'}:${m.startColumn || '?'} - ${m.message}</span>
          </div>
        `).join('');
        errorDrawer.style.display = "block";
      }
    }
  } catch (err) {
    console.error("Compilation threw an exception:", err);
    if (statusPill) statusPill.className = "status-badge error";
    if (statusText) statusText.innerText = "Execution Crash";
    luaEditor.setValue(`-- Compilation threw an unhandled JavaScript/WASM error:\n-- ${err.message || err}`);
  }
}

main();
