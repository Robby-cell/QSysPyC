import init, { compile_qsyspy } from './pkg/qsys_wasm.js';
import { registerLspFeatures } from './lsp.js';

let pyEditor, luaEditor;
let lastCompiledLua = '';
let lastProgramKind = 'Plugin';

const EXAMPLES = {
  mixer: `from qsys import Control, ControlHandle, Property, QSysPlugin

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
        if ctl.Value:
            # Control truthiness automatically guards against Lua's '0 is truthy'
            self.Mute.Boolean = False
            self.Status.String = f"Active ({self.Channels} ch)"
`,
  wol: `from qsys import Control, ControlHandle, QSysPlugin, UdpSocket

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
  controller: `from qsys import Control, ControlHandle, TextController, Timer

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
  script: `def calculate_gain(linear: float, offset: int) -> float:
    """Helper function to calculate dB offset."""
    return linear * 20.0 + offset

def run():
    channels = ["CH1", "CH2", "CH3"]
    for i in range(0, 3):
        msg = "Channel: " + channels[i]
        print(msg)
`
};

async function main() {
  const statusText = document.getElementById('status-text');
  try {
    if (statusText) statusText.innerText = 'Loading WASM...';
    await init();
    if (statusText) statusText.innerText = 'WASM Loaded';
    setupMonaco();
  } catch (err) {
    console.error('Failed to load QSysPy WASM module:', err);
    if (statusText) statusText.innerText = 'WASM Load Error';
    const luaContainer = document.getElementById('lua-editor');
    if (luaContainer) {
      luaContainer.innerHTML = `
        <div style="padding: 20px; color: #f14c4c; font-family: monospace; font-size: 13px;">
          <h3>Failed to load QSysPy WASM</h3>
          <p>${err.message || err}</p>
          <br>
          <p>Please recompile the WASM binary from the repository root:</p>
          <code>wasm-pack build crates/qsys_wasm --target web --out-dir ../../web/pkg</code>
        </div>
      `;
    }
  }
}

function setupMonaco() {
  require.config({
    paths: { vs: 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.45.0/min/vs' }
  });

  require(['vs/editor/editor.main'], function () {
    try {
      registerLspFeatures(monaco);
    } catch (e) {
      console.warn('LSP registration warning:', e);
    }

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
      suggestOnTriggerCharacters: true,
      quickSuggestions: { other: true, comments: false, strings: false },
      parameterHints: { enabled: true },
      inlayHints: { enabled: 'on' },
      lightbulb: { enabled: true }
    });

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

    let debounceTimer;
    pyEditor.onDidChangeModelContent(() => {
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(triggerCompile, 120);
    });

    const exampleSelect = document.getElementById('example-select');
    if (exampleSelect) {
      exampleSelect.addEventListener('change', e => {
        if (EXAMPLES[e.target.value]) {
          pyEditor.setValue(EXAMPLES[e.target.value]);
        }
      });
    }

    const btnCopy = document.getElementById('btn-copy');
    if (btnCopy) {
      btnCopy.addEventListener('click', async () => {
        await navigator.clipboard.writeText(lastCompiledLua);
        const orig = btnCopy.innerText;
        btnCopy.innerText = 'Copied!';
        setTimeout(() => {
          btnCopy.innerText = orig;
        }, 1400);
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

    triggerCompile();
  });
}

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function applyQuickFix(marker, replacement) {
  if (!pyEditor) return;
  const model = pyEditor.getModel();
  if (!model) return;

  const markerRange = new monaco.Range(
    marker.startLineNumber || 1,
    marker.startColumn || 1,
    marker.endLineNumber || marker.startLineNumber || 1,
    marker.endColumn || (marker.startColumn || 1) + 1
  );
  const existingText = model.getValueInRange(markerRange);
  const dotIdx = existingText.lastIndexOf('.');
  const editRange =
    dotIdx !== -1
      ? new monaco.Range(
          markerRange.startLineNumber,
          markerRange.startColumn + dotIdx + 1,
          markerRange.endLineNumber,
          markerRange.endColumn
        )
      : markerRange;

  pyEditor.executeEdits('qsyspy-quickfix', [{ range: editRange, text: replacement }]);
  pyEditor.focus();
}

function triggerCompile() {
  if (!pyEditor || !luaEditor) return;

  const statusPill = document.getElementById('status-pill');
  const statusText = document.getElementById('status-text');
  const errorDrawer = document.getElementById('error-drawer');

  let res = null;
  try {
    const code = pyEditor.getValue();
    if (!code.trim()) {
      luaEditor.setValue('-- Empty Python source file');
      if (statusText) statusText.innerText = 'Empty';
      if (errorDrawer) errorDrawer.style.display = 'none';
      return;
    }

    res = compile_qsyspy(code, 'playground.py');
    const markers = res.markers ? JSON.parse(res.markers) : [];

    if (monaco && pyEditor.getModel()) {
      const monacoMarkers = markers.map(m => ({
        code: m.code || 'E0000',
        message: m.help ? `${m.message}\nHelp: ${m.help}` : m.message,
        severity:
          m.severity === 'error' ? monaco.MarkerSeverity.Error : monaco.MarkerSeverity.Warning,
        startLineNumber: m.startLineNumber || 1,
        startColumn: m.startColumn || 1,
        endLineNumber: m.endLineNumber || m.startLineNumber || 1,
        endColumn: m.endColumn || (m.startColumn + 1) || 2
      }));
      monaco.editor.setModelMarkers(pyEditor.getModel(), 'qsyspy', monacoMarkers);
    }

    if (res.success) {
      const kind = res.target_kind || 'Compiled';
      if (statusPill) statusPill.className = 'status-badge ok';
      if (statusText) statusText.innerText = `Compiled (${kind})`;

      luaEditor.setValue(res.lua);
      lastCompiledLua = res.lua;
      lastProgramKind = kind;

      const outExt = document.getElementById('output-ext');
      const btnDl = document.getElementById('btn-download');
      if (outExt) outExt.innerText = kind === 'Plugin' ? '.qplug' : '.lua';
      if (btnDl) btnDl.innerText = kind === 'Plugin' ? 'Download .qplug' : 'Download .lua';
      if (errorDrawer) errorDrawer.style.display = 'none';
    } else {
      if (statusPill) statusPill.className = 'status-badge error';
      if (statusText) statusText.innerText = `${markers.length} Diagnostic(s)`;

      const errorHeader = [
        '-- ============================================================',
        `-- DIAGNOSTICS: ${markers.length} issue(s) detected (showing last valid Lua below)`,
        '-- ============================================================',
        ...markers.map(m => {
          const loc = `Line ${m.startLineNumber || '?'}:${m.startColumn || '?'}`;
          const help = m.help ? `\n--   Help: ${m.help}` : '';
          return `-- [${m.code || 'ERROR'}] ${loc} - ${m.message}${help}`;
        }),
        '-- ============================================================'
      ].join('\n');

      luaEditor.setValue(
        lastCompiledLua ? `${errorHeader}\n\n${lastCompiledLua}` : errorHeader
      );

      if (errorDrawer) {
        errorDrawer.innerHTML = '';
        markers.forEach(m => {
          const row = document.createElement('div');
          row.className = 'error-item';
          row.title = 'Click to jump to line in editor';

          const line = m.startLineNumber || 1;
          const col = m.startColumn || 1;

          const codeSpan = document.createElement('span');
          codeSpan.className = 'code';
          codeSpan.textContent = m.code || 'ERROR';

          const msgSpan = document.createElement('span');
          msgSpan.className = 'error-msg';
          msgSpan.innerHTML = `<strong>Line ${line}:${col}</strong> &mdash; ${escapeHtml(m.message)}${
            m.help ? ` <span class="error-help">(${escapeHtml(m.help)})</span>` : ''
          }`;

          row.appendChild(codeSpan);
          row.appendChild(msgSpan);

          const didYouMean = m.message && m.message.match(/did you mean '([^']+)'\?/);
          if (didYouMean) {
            const fixBtn = document.createElement('button');
            fixBtn.className = 'quickfix-btn';
            fixBtn.textContent = `Fix: '${didYouMean[1]}'`;
            fixBtn.addEventListener('click', ev => {
              ev.stopPropagation();
              applyQuickFix(m, didYouMean[1]);
            });
            row.appendChild(fixBtn);
          }

          row.addEventListener('click', () => {
            pyEditor.revealLineInCenter(line);
            pyEditor.setPosition({ lineNumber: line, column: col });
            pyEditor.focus();
          });

          errorDrawer.appendChild(row);
        });
        errorDrawer.style.display = 'block';
      }
    }
  } catch (err) {
    console.error('Compilation threw an exception:', err);
    if (statusPill) statusPill.className = 'status-badge error';
    if (statusText) statusText.innerText = 'Execution Crash';
    luaEditor.setValue(
      `-- Compilation threw an unhandled JavaScript/WASM error:\n-- ${err.message || err}`
    );
  } finally {
    if (res && typeof res.free === 'function') {
      res.free();
    }
  }
}

main();

