import init, { compile_qsyspy, get_qsys_stubs } from "./pkg/qsys_wasm.js";
import {
  getBaseName,
  getExtension,
  getParentFolder,
  normalizePath,
  WorkspaceManager,
} from "./fs.js";
import { registerLspFeatures } from "./lsp.js";

let pyEditor = null;
let luaEditor = null;
let workspace = null;
let lastCompiledLua = "";
let lastProgramKind = "Plugin";
let lastArtifactFilename = "output.qplug";

// Monaco per-file models and view states (cursor, scroll, undo history)
const fileModels = new Map(); // key: `${projectId}::${path}` -> ITextModel
const viewStates = new Map(); // key: `${projectId}::${path}` -> ICodeEditorViewState
const collapsedFolders = new Set();

let currentFilterQuery = "";
let pendingPathPromptAction = null;
let toastTimer = null;

function showToast(message, durationMs = 2600) {
  const toast = document.getElementById("toast");
  if (!toast) return;
  toast.textContent = message;
  toast.classList.remove("hidden");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.classList.add("hidden");
  }, durationMs);
}

/**
 * Mandatory fallback for `<dialog closedby="any">` in browsers without native `closedBy` support
 * (such as Safari), per modern-web-guidance `light-dismiss-a-dialog`.
 */
function setupDialogLightDismissFallbacks() {
  if ("closedBy" in HTMLDialogElement.prototype) return;

  document.querySelectorAll('dialog[closedby="any"]').forEach((dialog) => {
    dialog.addEventListener("click", (event) => {
      if (event.target !== dialog) return;
      const rect = dialog.getBoundingClientRect();
      const isDialogContent =
        rect.top <= event.clientY &&
        event.clientY <= rect.top + rect.height &&
        rect.left <= event.clientX &&
        event.clientX <= rect.left + rect.width;
      if (isDialogContent) return;
      dialog.close();
    });
  });
}

async function main() {
  setupDialogLightDismissFallbacks();

  const statusText = document.getElementById("status-text");
  try {
    if (statusText) statusText.textContent = "Loading WASM...";
    await init();
    if (statusText) statusText.textContent = "WASM Ready";

    const stubs = typeof get_qsys_stubs === "function" ? get_qsys_stubs() : "";
    workspace = new WorkspaceManager(stubs);
    await workspace.init();

    setupUIEventListeners();
    setupMonaco();
  } catch (err) {
    console.error("Failed to load QSysPy WASM module:", err);
    if (statusText) statusText.textContent = "WASM Load Error";
    const luaContainer = document.getElementById("lua-editor");
    if (luaContainer) {
      luaContainer.innerHTML = `
        <div class="wasm-error-box">
          <h3>Failed to load QSysPy WASM</h3>
          <p>${escapeHtml(err.message || err)}</p>
          <br>
          <p>Please recompile the WASM binary from the repository root:</p>
          <code>wasm-pack build crates/qsys_wasm --target web --out-dir ../../web/pkg</code>
        </div>
      `;
    }
  }
}

function getLanguageForPath(path) {
  const ext = getExtension(path);
  switch (ext) {
    case "py":
    case "pyi":
      return "python";
    case "lua":
    case "qplug":
      return "lua";
    case "json":
      return "json";
    case "md":
      return "markdown";
    default:
      return "plaintext";
  }
}

function getOrCreateModel(proj, filePath) {
  const key = `${proj.id}::${filePath}`;
  const fileObj = proj.files[filePath];
  const content = fileObj ? fileObj.content : "";
  const lang = getLanguageForPath(filePath);

  if (fileModels.has(key)) {
    const existing = fileModels.get(key);
    if (!existing.isDisposed()) {
      if (existing.getValue() !== content) {
        existing.setValue(content);
      }
      return existing;
    }
  }

  const uri = monaco.Uri.parse(`inmemory://qsyspy/${proj.id}/${encodeURI(filePath)}`);
  let model = monaco.editor.getModel(uri);
  if (model && !model.isDisposed()) {
    model.setValue(content);
  } else {
    model = monaco.editor.createModel(content, lang, uri);
  }
  fileModels.set(key, model);
  return model;
}

function setupMonaco() {
  require.config({
    paths: { vs: "https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.45.0/min/vs" },
  });

  require(["vs/editor/editor.main"], function () {
    try {
      registerLspFeatures(monaco, (model) => {
        const proj = workspace?.getActiveProject();
        if (!proj || !proj.activeFile || !proj.activeFile.endsWith(".py")) {
          return { source: model.getValue(), lineOffset: 0 };
        }
        const bundled = workspace.bundlePythonFile(proj.activeFile, model.getValue());
        return {
          source: bundled.bundledSource,
          lineOffset: bundled.lineOffset,
        };
      });
    } catch (e) {
      console.warn("LSP registration warning:", e);
    }

    pyEditor = monaco.editor.create(document.getElementById("python-editor"), {
      value: "",
      language: "python",
      theme: "vs-dark",
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 13,
      lineNumbers: "on",
      scrollBeyondLastLine: false,
      tabSize: 4,
      suggestOnTriggerCharacters: true,
      quickSuggestions: { other: true, comments: false, strings: false },
      parameterHints: { enabled: true },
      inlayHints: { enabled: "on" },
      lightbulb: { enabled: true },
    });

    luaEditor = monaco.editor.create(document.getElementById("lua-editor"), {
      value: "-- Compiling initial code...",
      language: "lua",
      theme: "vs-dark",
      readOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 13,
      lineNumbers: "on",
      scrollBeyondLastLine: false,
    });

    let debounceTimer;
    pyEditor.onDidChangeModelContent(() => {
      const proj = workspace.getActiveProject();
      if (proj && proj.activeFile) {
        const changed = workspace.updateFileContent(proj.activeFile, pyEditor.getValue());
        if (changed) {
          updateStorageBadge();
          renderTabs();
          renderFileTree();
        }
      }
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(triggerCompile, 110);
    });

    workspace.onChange((event) => {
      if (event === "persisted") {
        updateStorageBadge();
        return;
      }
      renderWorkspaceUI();
    });

    renderWorkspaceUI();
  });
}

function renderWorkspaceUI() {
  renderProjectSelector();
  renderFileTree();
  renderTabs();
  syncActiveEditorModel();
  updateStorageBadge();
}

function renderProjectSelector() {
  const select = document.getElementById("project-select");
  const sidebarTitle = document.getElementById("sidebar-project-name");
  const localBadge = document.getElementById("local-link-badge");
  const btnSyncDisk = document.getElementById("btn-sync-disk");
  const btnReloadDisk = document.getElementById("btn-reload-disk");

  const projects = workspace.getProjectsList();
  const active = workspace.getActiveProject();

  if (select) {
    select.innerHTML = "";
    for (const proj of projects) {
      const opt = document.createElement("option");
      opt.value = proj.id;
      opt.textContent = proj.hasLocalHandle ? `${proj.name} (Local)` : proj.name;
      if (active && proj.id === active.id) {
        opt.selected = true;
      }
      select.appendChild(opt);
    }
  }

  if (sidebarTitle && active) {
    sidebarTitle.textContent = active.name;
    sidebarTitle.title = active.name;
  }

  const isLocal = Boolean(active?.hasLocalHandle);
  if (localBadge) localBadge.classList.toggle("hidden", !isLocal);
  if (btnSyncDisk) btnSyncDisk.classList.toggle("hidden", !isLocal);
  if (btnReloadDisk) btnReloadDisk.classList.toggle("hidden", !isLocal);
}

function updateStorageBadge() {
  const pill = document.getElementById("storage-pill");
  const text = document.getElementById("storage-text");
  const proj = workspace?.getActiveProject();
  if (!pill || !text || !proj) return;

  const dirtyCount = Object.values(proj.files).filter((f) => f.dirty).length;
  if (proj.hasLocalHandle) {
    if (dirtyCount > 0) {
      pill.classList.add("dirty");
      text.textContent = `${dirtyCount} Unsynced to Disk`;
    } else {
      pill.classList.remove("dirty");
      text.textContent = "Synced to Local Disk";
    }
  } else {
    pill.classList.remove("dirty");
    text.textContent = "Saved in Browser";
  }
}

function renderFileTree() {
  const treeEl = document.getElementById("file-tree");
  if (!treeEl || !workspace) return;
  const proj = workspace.getActiveProject();
  if (!proj) return;

  const nodes = workspace.buildTree(currentFilterQuery);
  treeEl.innerHTML = "";

  const buildDomNodes = (items, container, depth) => {
    for (const item of items) {
      const li = document.createElement("li");
      li.setAttribute("role", "treeitem");

      const row = document.createElement("div");
      row.className = "tree-row";
      row.style.setProperty("--depth", String(depth));

      const left = document.createElement("div");
      left.className = "tree-row-left";

      const actions = document.createElement("div");
      actions.className = "tree-row-actions";

      if (item.kind === "folder") {
        const isCollapsed = collapsedFolders.has(item.path) && !currentFilterQuery;
        const chevron = document.createElement("span");
        chevron.className = "tree-chevron";
        chevron.textContent = isCollapsed ? "▸" : "▾";
        left.appendChild(chevron);

        const badge = document.createElement("span");
        badge.className = "file-badge";
        badge.textContent = "DIR";
        left.appendChild(badge);

        const label = document.createElement("span");
        label.className = "tree-label";
        label.textContent = item.name;
        left.appendChild(label);

        const addFileBtn = document.createElement("button");
        addFileBtn.type = "button";
        addFileBtn.className = "tree-action-btn";
        addFileBtn.title = `New file in ${item.path}`;
        addFileBtn.ariaLabel = `New file in ${item.path}`;
        addFileBtn.textContent = "+";
        addFileBtn.addEventListener("click", (ev) => {
          ev.stopPropagation();
          openPathPrompt("new-file", `${item.path}/`);
        });

        const renameBtn = document.createElement("button");
        renameBtn.type = "button";
        renameBtn.className = "tree-action-btn";
        renameBtn.title = `Rename ${item.path}`;
        renameBtn.ariaLabel = `Rename ${item.path}`;
        renameBtn.textContent = "✎";
        renameBtn.addEventListener("click", (ev) => {
          ev.stopPropagation();
          openPathPrompt("rename", item.path);
        });

        const delBtn = document.createElement("button");
        delBtn.type = "button";
        delBtn.className = "tree-action-btn";
        delBtn.title = `Delete folder ${item.path}`;
        delBtn.ariaLabel = `Delete folder ${item.path}`;
        delBtn.textContent = "✕";
        delBtn.addEventListener("click", (ev) => {
          ev.stopPropagation();
          workspace.deletePath(item.path);
          showToast(`Deleted folder '${item.path}'`);
        });

        actions.append(addFileBtn, renameBtn, delBtn);
        row.append(left, actions);

        row.addEventListener("click", () => {
          if (collapsedFolders.has(item.path)) {
            collapsedFolders.delete(item.path);
          } else {
            collapsedFolders.add(item.path);
          }
          renderFileTree();
        });

        li.appendChild(row);

        if (!isCollapsed && item.children && item.children.length > 0) {
          const subUl = document.createElement("ul");
          subUl.setAttribute("role", "group");
          buildDomNodes(item.children, subUl, depth + 1);
          li.appendChild(subUl);
        }
      } else {
        if (proj.activeFile === item.path) {
          row.classList.add("active");
        }

        const spacer = document.createElement("span");
        spacer.className = "tree-chevron";
        spacer.textContent = "";
        left.appendChild(spacer);

        const badge = document.createElement("span");
        const ext = item.ext || "txt";
        badge.className = `file-badge ext-${ext}`;
        badge.textContent = ext.slice(0, 5).toUpperCase();
        left.appendChild(badge);

        const label = document.createElement("span");
        label.className = "tree-label";
        label.textContent = item.name;
        label.title = item.path;
        left.appendChild(label);

        if (item.dirty) {
          const dot = document.createElement("span");
          dot.className = "dirty-dot";
          dot.title = "Modified since last local disk sync";
          left.appendChild(dot);
        }

        const dupBtn = document.createElement("button");
        dupBtn.type = "button";
        dupBtn.className = "tree-action-btn";
        dupBtn.title = `Duplicate ${item.path}`;
        dupBtn.ariaLabel = `Duplicate ${item.path}`;
        dupBtn.textContent = "⧉";
        dupBtn.addEventListener("click", (ev) => {
          ev.stopPropagation();
          const created = workspace.duplicateFile(item.path);
          if (created) showToast(`Duplicated as '${created.path}'`);
        });

        const renameBtn = document.createElement("button");
        renameBtn.type = "button";
        renameBtn.className = "tree-action-btn";
        renameBtn.title = `Rename ${item.path}`;
        renameBtn.ariaLabel = `Rename ${item.path}`;
        renameBtn.textContent = "✎";
        renameBtn.addEventListener("click", (ev) => {
          ev.stopPropagation();
          openPathPrompt("rename", item.path);
        });

        const delBtn = document.createElement("button");
        delBtn.type = "button";
        delBtn.className = "tree-action-btn";
        delBtn.title = `Delete ${item.path}`;
        delBtn.ariaLabel = `Delete ${item.path}`;
        delBtn.textContent = "✕";
        delBtn.addEventListener("click", (ev) => {
          ev.stopPropagation();
          workspace.deletePath(item.path);
          showToast(`Deleted '${item.path}'`);
        });

        actions.append(dupBtn, renameBtn, delBtn);
        row.append(left, actions);

        row.addEventListener("click", () => {
          workspace.openFile(item.path);
        });

        li.appendChild(row);
      }

      container.appendChild(li);
    }
  };

  buildDomNodes(nodes, treeEl, 0);
}

function renderTabs() {
  const tabsContainer = document.getElementById("editor-tabs");
  const activeFileLabel = document.getElementById("active-file-path");
  if (!tabsContainer || !workspace) return;

  const proj = workspace.getActiveProject();
  if (!proj) return;

  tabsContainer.innerHTML = "";

  for (const tabPath of proj.openTabs) {
    const fileObj = proj.files[tabPath];
    if (!fileObj) continue;

    const tab = document.createElement("div");
    tab.className = `editor-tab${proj.activeFile === tabPath ? " active" : ""}`;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", proj.activeFile === tabPath ? "true" : "false");
    tab.title = tabPath;

    const nameSpan = document.createElement("span");
    nameSpan.textContent = getBaseName(tabPath);
    tab.appendChild(nameSpan);

    if (fileObj.dirty) {
      const dot = document.createElement("span");
      dot.className = "dirty-dot";
      tab.appendChild(dot);
    }

    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.className = "tab-close-btn";
    closeBtn.ariaLabel = `Close ${tabPath}`;
    closeBtn.textContent = "✕";
    closeBtn.addEventListener("click", (ev) => {
      ev.stopPropagation();
      workspace.closeTab(tabPath);
    });

    tab.appendChild(closeBtn);
    tab.addEventListener("click", () => {
      workspace.openFile(tabPath);
    });

    tabsContainer.appendChild(tab);
  }

  if (activeFileLabel) {
    activeFileLabel.textContent = proj.activeFile || "No file open";
  }
}

let activeModelKey = "";

function syncActiveEditorModel() {
  if (!pyEditor || !workspace) return;
  const proj = workspace.getActiveProject();
  if (!proj || !proj.activeFile || !proj.files[proj.activeFile]) {
    return;
  }

  const nextKey = `${proj.id}::${proj.activeFile}`;
  if (activeModelKey && activeModelKey !== nextKey && pyEditor.getModel()) {
    viewStates.set(activeModelKey, pyEditor.saveViewState());
  }

  const model = getOrCreateModel(proj, proj.activeFile);
  if (pyEditor.getModel() !== model) {
    pyEditor.setModel(model);
    const savedState = viewStates.get(nextKey);
    if (savedState) {
      pyEditor.restoreViewState(savedState);
    }
  }
  activeModelKey = nextKey;

  triggerCompile();
}

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function applyQuickFix(marker, replacement) {
  if (!pyEditor) return;
  const model = pyEditor.getModel();
  if (!model) return;

  const markerRange = new monaco.Range(
    marker.startLineNumber || 1,
    marker.startColumn || 1,
    marker.endLineNumber || marker.startLineNumber || 1,
    marker.endColumn || (marker.startColumn || 1) + 1,
  );
  const existingText = model.getValueInRange(markerRange);
  const dotIdx = existingText.lastIndexOf(".");
  const editRange =
    dotIdx !== -1
      ? new monaco.Range(
          markerRange.startLineNumber,
          markerRange.startColumn + dotIdx + 1,
          markerRange.endLineNumber,
          markerRange.endColumn,
        )
      : markerRange;

  pyEditor.executeEdits("qsyspy-quickfix", [{ range: editRange, text: replacement }]);
  pyEditor.focus();
}

function triggerCompile() {
  if (!pyEditor || !luaEditor || !workspace) return;

  const statusPill = document.getElementById("status-pill");
  const statusText = document.getElementById("status-text");
  const errorDrawer = document.getElementById("error-drawer");
  const bundledBadge = document.getElementById("bundled-badge");
  const artifactFilenameEl = document.getElementById("artifact-filename");
  const artifactImportsEl = document.getElementById("artifact-imports");

  const proj = workspace.getActiveProject();
  const activePath = proj?.activeFile || "playground.py";
  const ext = getExtension(activePath);

  // If viewing a non-Python file (e.g. README.md, pyrightconfig.json, or a .qplug/.lua file)
  if (ext !== "py") {
    if (errorDrawer) errorDrawer.classList.remove("open");
    if (bundledBadge) bundledBadge.classList.add("hidden");
    if (ext === "qplug" || ext === "lua") {
      luaEditor.setValue(pyEditor.getValue());
      if (statusPill) statusPill.className = "status-badge ok";
      if (statusText) statusText.textContent = `Viewing .${ext}`;
    } else {
      if (statusPill) statusPill.className = "status-badge ok";
      if (statusText) statusText.textContent = `Viewing ${getBaseName(activePath)}`;
    }
    return;
  }

  let res = null;
  try {
    const code = pyEditor.getValue();
    if (!code.trim()) {
      luaEditor.setValue("-- Empty Python source file");
      if (statusText) statusText.textContent = "Empty";
      if (errorDrawer) errorDrawer.classList.remove("open");
      return;
    }

    const { bundledSource, lineOffset, importedFiles } = workspace.bundlePythonFile(
      activePath,
      code,
    );

    if (bundledBadge) {
      if (importedFiles.length > 0) {
        bundledBadge.textContent = `+${importedFiles.length} bundled module(s)`;
        bundledBadge.title = `Bundled project modules: ${importedFiles.join(", ")}`;
        bundledBadge.classList.remove("hidden");
      } else {
        bundledBadge.classList.add("hidden");
      }
    }

    res = compile_qsyspy(bundledSource, activePath);
    const rawMarkers = res.markers ? JSON.parse(res.markers) : [];

    // Adjust marker line numbers by subtracting the bundled preamble offset
    const markers = rawMarkers.map((m) => {
      const adjStart = (m.startLineNumber || 1) - lineOffset;
      const adjEnd = (m.endLineNumber || m.startLineNumber || 1) - lineOffset;
      const inPreamble = adjStart < 1;
      return {
        ...m,
        inPreamble,
        startLineNumber: Math.max(1, adjStart),
        endLineNumber: Math.max(1, adjEnd),
      };
    });

    if (monaco && pyEditor.getModel()) {
      const monacoMarkers = markers
        .filter((m) => !m.inPreamble)
        .map((m) => ({
          code: m.code || "E0000",
          message: m.help ? `${m.message}\nHelp: ${m.help}` : m.message,
          severity:
            m.severity === "error" ? monaco.MarkerSeverity.Error : monaco.MarkerSeverity.Warning,
          startLineNumber: m.startLineNumber || 1,
          startColumn: m.startColumn || 1,
          endLineNumber: m.endLineNumber || m.startLineNumber || 1,
          endColumn: m.endColumn || m.startColumn + 1 || 2,
        }));
      monaco.editor.setModelMarkers(pyEditor.getModel(), "qsyspy", monacoMarkers);
    }

    const stem = getBaseName(activePath).replace(/\.py$/i, "");

    if (res.success) {
      const kind = res.target_kind || "Compiled";
      if (statusPill) statusPill.className = "status-badge ok";
      if (statusText) statusText.textContent = `Compiled (${kind})`;

      luaEditor.setValue(res.lua);
      lastCompiledLua = res.lua;
      lastProgramKind = kind;

      const outExtStr = kind === "Plugin" ? "qplug" : "lua";
      lastArtifactFilename = `${stem}.${outExtStr}`;

      const outExt = document.getElementById("output-ext");
      const btnDl = document.getElementById("btn-download");
      if (outExt) outExt.textContent = `.${outExtStr}`;
      if (btnDl) btnDl.textContent = `Download .${outExtStr}`;
      if (artifactFilenameEl) artifactFilenameEl.textContent = `build/${lastArtifactFilename}`;
      if (artifactImportsEl) {
        artifactImportsEl.textContent =
          importedFiles.length > 0
            ? `Imports: ${importedFiles.map(getBaseName).join(", ")}`
            : "Standalone module";
      }
      if (errorDrawer) errorDrawer.classList.remove("open");
    } else {
      if (statusPill) statusPill.className = "status-badge error";
      if (statusText) statusText.textContent = `${markers.length} Diagnostic(s)`;

      const errorHeader = [
        "-- ============================================================",
        `-- DIAGNOSTICS: ${markers.length} issue(s) detected (showing last valid Lua below)`,
        "-- ============================================================",
        ...markers.map((m) => {
          const loc = m.inPreamble
            ? `Imported module`
            : `Line ${m.startLineNumber || "?"}:${m.startColumn || "?"}`;
          const help = m.help ? `\n--   Help: ${m.help}` : "";
          return `-- [${m.code || "ERROR"}] ${loc} - ${m.message}${help}`;
        }),
        "-- ============================================================",
      ].join("\n");

      luaEditor.setValue(lastCompiledLua ? `${errorHeader}\n\n${lastCompiledLua}` : errorHeader);

      if (errorDrawer) {
        errorDrawer.innerHTML = "";
        markers.forEach((m) => {
          const row = document.createElement("div");
          row.className = "error-item";
          row.title = "Click to jump to line in editor";

          const line = m.startLineNumber || 1;
          const col = m.startColumn || 1;

          const codeSpan = document.createElement("span");
          codeSpan.className = "code";
          codeSpan.textContent = m.code || "ERROR";

          const msgSpan = document.createElement("span");
          msgSpan.className = "error-msg";
          const locLabel = m.inPreamble ? "Imported Module" : `Line ${line}:${col}`;
          msgSpan.innerHTML = `<strong>${locLabel}</strong> &mdash; ${escapeHtml(m.message)}${
            m.help ? ` <span class="error-help">(${escapeHtml(m.help)})</span>` : ""
          }`;

          row.appendChild(codeSpan);
          row.appendChild(msgSpan);

          const didYouMean = m.message && m.message.match(/did you mean '([^']+)'\?/);
          if (didYouMean && !m.inPreamble) {
            const fixBtn = document.createElement("button");
            fixBtn.type = "button";
            fixBtn.className = "quickfix-btn";
            fixBtn.textContent = `Fix: '${didYouMean[1]}'`;
            fixBtn.addEventListener("click", (ev) => {
              ev.stopPropagation();
              applyQuickFix(m, didYouMean[1]);
            });
            row.appendChild(fixBtn);
          }

          row.addEventListener("click", () => {
            pyEditor.revealLineInCenter(line);
            pyEditor.setPosition({ lineNumber: line, column: col });
            pyEditor.focus();
          });

          errorDrawer.appendChild(row);
        });
        errorDrawer.classList.add("open");
      }
    }
  } catch (err) {
    console.error("Compilation threw an exception:", err);
    if (statusPill) statusPill.className = "status-badge error";
    if (statusText) statusText.textContent = "Execution Crash";
    luaEditor.setValue(
      `-- Compilation threw an unhandled JavaScript/WASM error:\n-- ${err.message || err}`,
    );
  } finally {
    if (res && typeof res.free === "function") {
      res.free();
    }
  }
}

/**
 * Compiles all `.py` files in the active project and returns a map of
 * `{ 'build/<name>.qplug': luaCode, ... }` for all valid targets.
 */
function compileAllProjectTargets() {
  const proj = workspace?.getActiveProject();
  if (!proj) return {};
  const outputs = {};

  for (const file of Object.values(proj.files)) {
    if (!file.path.endsWith(".py")) continue;
    let res = null;
    try {
      const { bundledSource } = workspace.bundlePythonFile(file.path, file.content);
      res = compile_qsyspy(bundledSource, file.path);
      if (res.success && res.lua) {
        const stem = getBaseName(file.path).replace(/\.py$/i, "");
        const ext = res.target_kind === "Plugin" ? "qplug" : "lua";
        outputs[`build/${stem}.${ext}`] = res.lua;
      }
    } catch (_e) {
      // Skip files with errors during batch build
    } finally {
      if (res && typeof res.free === "function") {
        res.free();
      }
    }
  }
  return outputs;
}

function downloadBlob(blob, filename) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

function openPathPrompt(mode, initialValue = "") {
  const dialog = document.getElementById("dialog-path-prompt");
  const title = document.getElementById("dialog-path-title");
  const label = document.getElementById("dialog-path-label");
  const input = document.getElementById("path-prompt-input");
  const errorEl = document.getElementById("dialog-path-error");
  const submitBtn = document.getElementById("dialog-path-submit");
  if (!dialog || !input) return;

  errorEl.classList.add("hidden");
  errorEl.textContent = "";

  const proj = workspace.getActiveProject();
  const activeDir = proj?.activeFile ? getParentFolder(proj.activeFile) : "src";
  const defaultPrefix = activeDir ? `${activeDir}/` : "";

  if (mode === "new-file") {
    title.textContent = "Create New File";
    label.textContent = "File path (defaults to .py if no extension is given)";
    input.value = initialValue || `${defaultPrefix}new_module.py`;
    submitBtn.textContent = "Create File";
  } else if (mode === "new-folder") {
    title.textContent = "Create New Folder";
    label.textContent = "Folder path (e.g. src/components)";
    input.value = initialValue || `${defaultPrefix}helpers`;
    submitBtn.textContent = "Create Folder";
  } else if (mode === "rename") {
    title.textContent = "Rename File or Folder";
    label.textContent = `New path for '${initialValue}'`;
    input.value = initialValue;
    submitBtn.textContent = "Rename";
  } else if (mode === "rename-project") {
    title.textContent = "Rename Project";
    label.textContent = "Project name";
    input.value = proj ? proj.name : "";
    submitBtn.textContent = "Rename Project";
  }

  pendingPathPromptAction = { mode, targetPath: initialValue };
  dialog.showModal();
  input.focus();
  input.select();
}

function setupUIEventListeners() {
  // Sidebar Toggle
  const btnToggleSidebar = document.getElementById("btn-toggle-sidebar");
  const sidebar = document.getElementById("sidebar");
  if (btnToggleSidebar && sidebar) {
    btnToggleSidebar.addEventListener("click", () => {
      sidebar.classList.toggle("collapsed");
      if (pyEditor) pyEditor.layout();
      if (luaEditor) luaEditor.layout();
    });
  }

  // Project Selector
  const projectSelect = document.getElementById("project-select");
  if (projectSelect) {
    projectSelect.addEventListener("change", (e) => {
      workspace.setActiveProject(e.target.value);
      showToast(`Switched to '${workspace.getActiveProject()?.name}'`);
    });
  }

  // New / Manage Project Dialog
  const dialogProject = document.getElementById("dialog-project");
  const btnNewProject = document.getElementById("btn-new-project");
  const formProject = document.getElementById("form-project");
  if (btnNewProject && dialogProject) {
    btnNewProject.addEventListener("click", () => {
      const nameInput = document.getElementById("new-project-name");
      if (nameInput) nameInput.value = "";
      dialogProject.showModal();
    });
  }

  if (formProject && dialogProject) {
    formProject.addEventListener("submit", (e) => {
      const submitter = e.submitter;
      if (!submitter || submitter.value !== "create") return;
      const nameInput = document.getElementById("new-project-name");
      const tplSelect = document.getElementById("new-project-template");
      const name = nameInput?.value.trim() || "New Q-SYS Project";
      const tpl = tplSelect?.value || "plugin";
      workspace.createProject(name, tpl);
      showToast(`Created project '${name}'`);
    });
  }

  const btnRenameProject = document.getElementById("btn-rename-project");
  if (btnRenameProject && dialogProject) {
    btnRenameProject.addEventListener("click", () => {
      dialogProject.close();
      openPathPrompt("rename-project");
    });
  }

  const btnDeleteProject = document.getElementById("btn-delete-project");
  if (btnDeleteProject && dialogProject) {
    btnDeleteProject.addEventListener("click", async () => {
      const proj = workspace.getActiveProject();
      if (!proj) return;
      const deletedName = proj.name;
      await workspace.deleteProject(proj.id);
      dialogProject.close();
      showToast(`Deleted project '${deletedName}'`);
    });
  }

  // File / Folder Path Prompt Modal
  const dialogPath = document.getElementById("dialog-path-prompt");
  const formPath = document.getElementById("form-path-prompt");
  if (formPath && dialogPath) {
    formPath.addEventListener("submit", (e) => {
      const submitter = e.submitter;
      if (!submitter || submitter.value !== "confirm" || !pendingPathPromptAction) return;

      const input = document.getElementById("path-prompt-input");
      const errorEl = document.getElementById("dialog-path-error");
      const val = input?.value.trim() || "";

      try {
        const { mode, targetPath } = pendingPathPromptAction;
        if (mode === "new-file") {
          const created = workspace.createFile(val);
          if (created) showToast(`Created '${created.path}'`);
        } else if (mode === "new-folder") {
          workspace.createFolder(val);
          showToast(`Created folder '${normalizePath(val)}'`);
        } else if (mode === "rename") {
          workspace.renamePath(targetPath, val);
          showToast(`Renamed to '${normalizePath(val)}'`);
        } else if (mode === "rename-project") {
          const proj = workspace.getActiveProject();
          if (proj) {
            workspace.renameProject(proj.id, val);
            showToast(`Renamed project to '${val}'`);
          }
        }
        pendingPathPromptAction = null;
      } catch (err) {
        e.preventDefault();
        if (errorEl) {
          errorEl.textContent = err.message || String(err);
          errorEl.classList.remove("hidden");
        }
      }
    });
  }

  // Sidebar +File / +Dir / Reload Disk buttons
  document.getElementById("btn-new-file")?.addEventListener("click", () => {
    openPathPrompt("new-file");
  });
  document.getElementById("btn-new-folder")?.addEventListener("click", () => {
    openPathPrompt("new-folder");
  });
  document.getElementById("btn-reload-disk")?.addEventListener("click", async () => {
    try {
      await workspace.reloadFromLocalDisk();
      showToast("Reloaded project files from local disk");
    } catch (err) {
      showToast(`Disk reload failed: ${err.message || err}`, 4000);
    }
  });

  // File Filter Search
  const filterInput = document.getElementById("file-filter-input");
  if (filterInput) {
    filterInput.addEventListener("input", (e) => {
      currentFilterQuery = e.target.value;
      renderFileTree();
    });
  }

  // Open Local Folder (File System Access API with <input webkitdirectory> fallback)
  const btnOpenFolder = document.getElementById("btn-open-folder");
  const inputImportFolder = document.getElementById("input-import-folder");
  if (btnOpenFolder) {
    btnOpenFolder.addEventListener("click", async () => {
      try {
        if (typeof window.showDirectoryPicker === "function") {
          const proj = await workspace.openLocalDirectoryPicker();
          if (proj) {
            showToast(`Mounted local folder '${proj.name}'`);
          }
        } else if (inputImportFolder) {
          inputImportFolder.value = "";
          inputImportFolder.click();
        }
      } catch (err) {
        if (err && err.name !== "AbortError") {
          showToast(`Could not open folder: ${err.message || err}`, 4000);
        }
      }
    });
  }

  if (inputImportFolder) {
    inputImportFolder.addEventListener("change", async (e) => {
      try {
        const proj = await workspace.importFromFileList(e.target.files, { asNewProject: true });
        if (proj) {
          showToast(`Imported folder '${proj.name}' (${Object.keys(proj.files).length} files)`);
        }
      } catch (err) {
        showToast(`Folder import error: ${err.message || err}`, 4000);
      }
    });
  }

  // Import Files / .zip / .qsysproj.json
  const btnImport = document.getElementById("btn-import");
  const inputImportFiles = document.getElementById("input-import-files");
  if (btnImport && inputImportFiles) {
    btnImport.addEventListener("click", () => {
      inputImportFiles.value = "";
      inputImportFiles.click();
    });
    inputImportFiles.addEventListener("change", async (e) => {
      try {
        const proj = await workspace.importFromFileList(e.target.files);
        if (proj) {
          showToast(`Imported into '${proj.name}'`);
        }
      } catch (err) {
        showToast(`Import error: ${err.message || err}`, 4000);
      }
    });
  }

  // Drag and Drop onto Sidebar File Tree
  const treeContainer = document.getElementById("file-tree-container");
  if (treeContainer) {
    treeContainer.addEventListener("dragover", (e) => {
      e.preventDefault();
      treeContainer.classList.add("drag-over");
    });
    treeContainer.addEventListener("dragleave", () => {
      treeContainer.classList.remove("drag-over");
    });
    treeContainer.addEventListener("drop", async (e) => {
      e.preventDefault();
      treeContainer.classList.remove("drag-over");
      if (e.dataTransfer?.files?.length) {
        try {
          await workspace.importFromFileList(e.dataTransfer.files);
          showToast(`Imported ${e.dataTransfer.files.length} dropped file(s)`);
        } catch (err) {
          showToast(`Drop import failed: ${err.message || err}`, 4000);
        }
      }
    });
  }

  // Sync Disk Button (Header)
  const handleSyncToLocalDisk = async () => {
    try {
      const includeCompiled = document.getElementById("export-include-compiled")?.checked ?? true;
      const includeStubs = document.getElementById("export-include-stubs")?.checked ?? true;
      const compiledOutputs = includeCompiled ? compileAllProjectTargets() : {};
      const result = await workspace.syncToLocalDisk({ compiledOutputs, includeStubs });
      if (result) {
        showToast(`Synced ${result.writtenCount} files to '${result.folderName}'`);
      }
    } catch (err) {
      if (err && err.name !== "AbortError") {
        showToast(`Disk sync failed: ${err.message || err}`, 4000);
      }
    }
  };

  document.getElementById("btn-sync-disk")?.addEventListener("click", handleSyncToLocalDisk);

  // Save Compiled Artifact into virtual FS `build/`
  document.getElementById("btn-save-artifact-to-fs")?.addEventListener("click", () => {
    if (!lastCompiledLua) {
      showToast("No valid compiled Lua output to save.");
      return;
    }
    const targetPath = `build/${lastArtifactFilename}`;
    workspace.upsertFile(targetPath, lastCompiledLua, false);
    showToast(`Saved '${targetPath}' in project explorer`);
  });

  // Export Modal & Export Actions
  const dialogExport = document.getElementById("dialog-export");
  document.getElementById("btn-export")?.addEventListener("click", () => {
    dialogExport?.showModal();
  });

  document.getElementById("btn-export-local-dir")?.addEventListener("click", async () => {
    await handleSyncToLocalDisk();
    dialogExport?.close();
  });

  document.getElementById("btn-export-zip")?.addEventListener("click", () => {
    const proj = workspace.getActiveProject();
    if (!proj) return;
    const includeCompiled = document.getElementById("export-include-compiled")?.checked ?? true;
    const includeStubs = document.getElementById("export-include-stubs")?.checked ?? true;
    const compiledOutputs = includeCompiled ? compileAllProjectTargets() : {};
    const blob = workspace.exportProjectZipBlob({ compiledOutputs, includeStubs });
    const safeName = proj.name.replace(/[^a-zA-Z0-9_-]/g, "_") || "qsyspy_project";
    downloadBlob(blob, `${safeName}.zip`);
    dialogExport?.close();
    showToast(`Downloaded '${safeName}.zip'`);
  });

  document.getElementById("btn-export-json")?.addEventListener("click", () => {
    const proj = workspace.getActiveProject();
    if (!proj) return;
    const blob = workspace.exportProjectJsonBlob();
    const safeName = proj.name.replace(/[^a-zA-Z0-9_-]/g, "_") || "qsyspy_project";
    downloadBlob(blob, `${safeName}.qsysproj.json`);
    dialogExport?.close();
    showToast(`Exported '${safeName}.qsysproj.json'`);
  });

  document.getElementById("btn-export-pyi")?.addEventListener("click", () => {
    const stubs = workspace.stubsContent || "";
    const blob = new Blob([stubs], { type: "text/plain" });
    downloadBlob(blob, "qsys.pyi");
    dialogExport?.close();
    showToast("Downloaded qsys.pyi type stubs");
  });

  // Copy Lua & Download single .qplug / .lua
  const btnCopy = document.getElementById("btn-copy");
  if (btnCopy) {
    btnCopy.addEventListener("click", async () => {
      await navigator.clipboard.writeText(lastCompiledLua);
      const orig = btnCopy.textContent;
      btnCopy.textContent = "Copied!";
      setTimeout(() => {
        btnCopy.textContent = orig;
      }, 1400);
    });
  }

  const btnDownload = document.getElementById("btn-download");
  if (btnDownload) {
    btnDownload.addEventListener("click", () => {
      const blob = new Blob([lastCompiledLua], { type: "text/plain" });
      downloadBlob(blob, lastArtifactFilename);
    });
  }

  // Global Keyboard Shortcuts (Ctrl+S / Cmd+S to Save & Sync, Ctrl+B / Cmd+B to toggle sidebar)
  window.addEventListener("keydown", async (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      await workspace.persistNow();
      const proj = workspace.getActiveProject();
      if (proj?.hasLocalHandle) {
        await handleSyncToLocalDisk();
      } else {
        showToast("Saved workspace to browser storage");
      }
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "b") {
      e.preventDefault();
      sidebar?.classList.toggle("collapsed");
      if (pyEditor) pyEditor.layout();
      if (luaEditor) luaEditor.layout();
    }
  });
}

main();
