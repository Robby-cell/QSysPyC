// Virtual Filesystem, Persistence (IndexedDB + localStorage), Multi-File Import Bundler,
// and Local Disk Synchronization (File System Access API + fallbacks) for QSysPy Studio.

import { createTemplateFiles, PYRIGHT_CONFIG } from "./templates.js";
import { createZipBlob, extractZipEntries } from "./zip.js";

const STORAGE_KEY = "qsyspy_workspace_v1";
const IDB_NAME = "qsyspy_studio_db";
const IDB_VERSION = 1;

const IGNORED_DIRS = new Set([
  ".git",
  "node_modules",
  "__pycache__",
  ".venv",
  "venv",
  ".mypy_cache",
  ".pytest_cache",
  ".ruff_cache",
  "target",
]);

const TEXT_EXTENSIONS = new Set([
  "py",
  "pyi",
  "lua",
  "qplug",
  "json",
  "md",
  "txt",
  "toml",
  "yaml",
  "yml",
  "cfg",
  "ini",
]);

export function normalizePath(rawPath) {
  if (!rawPath) return "";
  const parts = String(rawPath)
    .replace(/\\/g, "/")
    .split("/")
    .map((p) => p.trim())
    .filter((p) => p.length > 0 && p !== ".");
  const safe = [];
  for (const part of parts) {
    if (part === "..") {
      safe.pop();
    } else {
      safe.push(part);
    }
  }
  return safe.join("/");
}

export function getParentFolder(path) {
  const norm = normalizePath(path);
  const idx = norm.lastIndexOf("/");
  return idx === -1 ? "" : norm.slice(0, idx);
}

export function getBaseName(path) {
  const norm = normalizePath(path);
  const idx = norm.lastIndexOf("/");
  return idx === -1 ? norm : norm.slice(idx + 1);
}

export function getExtension(path) {
  const base = getBaseName(path);
  const idx = base.lastIndexOf(".");
  return idx === -1 ? "" : base.slice(idx + 1).toLowerCase();
}

function isSupportedTextFile(filename) {
  const ext = getExtension(filename);
  return TEXT_EXTENSIONS.has(ext) || filename === ".gitignore";
}

function openIdb() {
  return new Promise((resolve, reject) => {
    if (typeof indexedDB === "undefined") {
      resolve(null);
      return;
    }
    const req = indexedDB.open(IDB_NAME, IDB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("workspace")) {
        db.createObjectStore("workspace");
      }
      if (!db.objectStoreNames.contains("dir_handles")) {
        db.createObjectStore("dir_handles");
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function idbGet(storeName, key) {
  try {
    const db = await openIdb();
    if (!db) return null;
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(storeName, "readonly");
      const req = tx.objectStore(storeName).get(key);
      req.onsuccess = () => resolve(req.result ?? null);
      req.onerror = () => reject(req.error);
    });
  } catch (_e) {
    return null;
  }
}

async function idbSet(storeName, key, value) {
  try {
    const db = await openIdb();
    if (!db) return false;
    await new Promise((resolve, reject) => {
      const tx = db.transaction(storeName, "readwrite");
      tx.objectStore(storeName).put(value, key);
      tx.oncomplete = () => resolve(true);
      tx.onerror = () => reject(tx.error);
    });
    return true;
  } catch (_e) {
    return false;
  }
}

async function idbDelete(storeName, key) {
  try {
    const db = await openIdb();
    if (!db) return;
    await new Promise((resolve, reject) => {
      const tx = db.transaction(storeName, "readwrite");
      tx.objectStore(storeName).delete(key);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  } catch (_e) {
    // Ignore IDB deletion errors
  }
}

export class WorkspaceManager {
  constructor(stubsContent = "") {
    this.stubsContent = stubsContent;
    this.state = {
      version: 1,
      activeProjectId: "",
      projects: {},
    };
    this.dirHandles = new Map(); // projectId -> FileSystemDirectoryHandle
    this.listeners = new Set();
    this.saveTimer = null;
  }

  onChange(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  notify(event = "change") {
    for (const fn of this.listeners) {
      try {
        fn(event, this.getActiveProject());
      } catch (e) {
        console.error("Workspace listener error:", e);
      }
    }
  }

  async init() {
    let loaded = null;

    // 1. Try IndexedDB first
    const fromIdb = await idbGet("workspace", "state");
    if (fromIdb && fromIdb.projects && Object.keys(fromIdb.projects).length > 0) {
      loaded = fromIdb;
    }

    // 2. Fallback to localStorage mirror
    if (!loaded) {
      try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (raw) {
          const parsed = JSON.parse(raw);
          if (parsed && parsed.projects && Object.keys(parsed.projects).length > 0) {
            loaded = parsed;
          }
        }
      } catch (_e) {
        // Ignore parse error
      }
    }

    if (loaded) {
      this.state = loaded;
      // Restore any persisted FileSystemDirectoryHandles from IndexedDB
      for (const projId of Object.keys(this.state.projects)) {
        const proj = this.state.projects[projId];
        // Refresh typings/qsys.pyi with latest compiler stubs if present
        if (this.stubsContent && proj.files["typings/qsys.pyi"]) {
          proj.files["typings/qsys.pyi"].content = this.stubsContent;
        }
        const handle = await idbGet("dir_handles", projId);
        if (handle) {
          this.dirHandles.set(projId, handle);
          proj.hasLocalHandle = true;
        } else {
          proj.hasLocalHandle = false;
        }
      }
    } else {
      // Seed default Starter Examples project on first visit
      this.createProject("Q-SYS Starter Examples", "starter");
    }

    if (!this.state.projects[this.state.activeProjectId]) {
      this.state.activeProjectId = Object.keys(this.state.projects)[0];
    }

    await this.persistNow();
    this.notify("init");
  }

  schedulePersist() {
    clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      this.persistNow();
    }, 250);
  }

  async persistNow() {
    clearTimeout(this.saveTimer);
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.state));
    } catch (_e) {
      // Quota or private mode
    }
    await idbSet("workspace", "state", this.state);
    this.notify("persisted");
  }

  getProjectsList() {
    return Object.values(this.state.projects).sort((a, b) => a.createdAt - b.createdAt);
  }

  getActiveProject() {
    return this.state.projects[this.state.activeProjectId] || null;
  }

  setActiveProject(projectId) {
    if (!this.state.projects[projectId]) return null;
    this.state.activeProjectId = projectId;
    this.schedulePersist();
    this.notify("project-switched");
    return this.getActiveProject();
  }

  createProject(name, templateKind = "plugin", customFiles = null) {
    const id = `proj_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
    const cleanName = (name || "Untitled Project").trim();
    const tpl = customFiles || createTemplateFiles(templateKind, cleanName, this.stubsContent);

    const filesMap = {};
    const foldersSet = new Set(tpl.folders || []);

    for (const [rawPath, content] of Object.entries(tpl.files || {})) {
      const path = normalizePath(rawPath);
      if (!path) continue;
      filesMap[path] = {
        path,
        content: typeof content === "string" ? content : content.content || "",
        updatedAt: Date.now(),
        dirty: false,
      };
      let parent = getParentFolder(path);
      while (parent) {
        foldersSet.add(parent);
        parent = getParentFolder(parent);
      }
    }

    const allPaths = Object.keys(filesMap);
    const pyPaths = allPaths.filter((p) => p.endsWith(".py"));
    const defaultActive =
      tpl.activeFile && filesMap[tpl.activeFile] ? tpl.activeFile : pyPaths[0] || allPaths[0] || "";
    const openTabs = (tpl.openTabs || [defaultActive]).filter((p) => filesMap[p]);
    if (openTabs.length === 0 && defaultActive) {
      openTabs.push(defaultActive);
    }

    const project = {
      id,
      name: cleanName,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      files: filesMap,
      folders: Array.from(foldersSet).sort(),
      openTabs,
      activeFile: defaultActive,
      hasLocalHandle: false,
      localFolderName: null,
    };

    this.state.projects[id] = project;
    this.state.activeProjectId = id;
    this.schedulePersist();
    this.notify("project-created");
    return project;
  }

  renameProject(projectId, newName) {
    const proj = this.state.projects[projectId];
    if (!proj || !newName.trim()) return false;
    proj.name = newName.trim();
    proj.updatedAt = Date.now();
    this.schedulePersist();
    this.notify("project-renamed");
    return true;
  }

  async deleteProject(projectId) {
    if (!this.state.projects[projectId]) return false;
    delete this.state.projects[projectId];
    this.dirHandles.delete(projectId);
    await idbDelete("dir_handles", projectId);

    const remainingIds = Object.keys(this.state.projects);
    if (remainingIds.length === 0) {
      this.createProject("Q-SYS Starter Examples", "starter");
    } else if (this.state.activeProjectId === projectId) {
      this.state.activeProjectId = remainingIds[0];
    }

    await this.persistNow();
    this.notify("project-deleted");
    return true;
  }

  ensureParentFolders(proj, filePath) {
    const folderSet = new Set(proj.folders || []);
    let parent = getParentFolder(filePath);
    while (parent) {
      folderSet.add(parent);
      parent = getParentFolder(parent);
    }
    proj.folders = Array.from(folderSet).sort();
  }

  createFile(rawPath, initialContent = "") {
    const proj = this.getActiveProject();
    if (!proj) return null;
    let path = normalizePath(rawPath);
    if (!path) return null;
    if (!getExtension(path)) {
      path = `${path}.py`;
    }
    if (proj.files[path]) {
      throw new Error(`File '${path}' already exists.`);
    }

    let content = initialContent;
    if (!content && path.endsWith(".py")) {
      content = `from qsys import Control, ControlHandle, QSysPlugin\n\n`;
    }

    proj.files[path] = {
      path,
      content,
      updatedAt: Date.now(),
      dirty: Boolean(proj.hasLocalHandle),
    };
    this.ensureParentFolders(proj, path);

    if (!proj.openTabs.includes(path)) {
      proj.openTabs.push(path);
    }
    proj.activeFile = path;
    proj.updatedAt = Date.now();
    this.schedulePersist();
    this.notify("file-created");
    return proj.files[path];
  }

  createFolder(rawPath) {
    const proj = this.getActiveProject();
    if (!proj) return false;
    const path = normalizePath(rawPath);
    if (!path) return false;
    const folderSet = new Set(proj.folders || []);
    if (folderSet.has(path)) {
      throw new Error(`Folder '${path}' already exists.`);
    }
    let curr = path;
    while (curr) {
      folderSet.add(curr);
      curr = getParentFolder(curr);
    }
    proj.folders = Array.from(folderSet).sort();
    proj.updatedAt = Date.now();
    this.schedulePersist();
    this.notify("folder-created");
    return true;
  }

  updateFileContent(rawPath, content) {
    const proj = this.getActiveProject();
    if (!proj) return false;
    const path = normalizePath(rawPath);
    const file = proj.files[path];
    if (!file) return false;
    if (file.content === content) return false;

    file.content = content;
    file.updatedAt = Date.now();
    if (proj.hasLocalHandle) {
      file.dirty = true;
    }
    proj.updatedAt = Date.now();
    this.schedulePersist();
    return true;
  }

  upsertFile(rawPath, content, openInTab = false) {
    const proj = this.getActiveProject();
    if (!proj) return null;
    const path = normalizePath(rawPath);
    if (!path) return null;

    proj.files[path] = {
      path,
      content,
      updatedAt: Date.now(),
      dirty: Boolean(proj.hasLocalHandle),
    };
    this.ensureParentFolders(proj, path);

    if (openInTab) {
      if (!proj.openTabs.includes(path)) {
        proj.openTabs.push(path);
      }
      proj.activeFile = path;
    }
    proj.updatedAt = Date.now();
    this.schedulePersist();
    this.notify("file-upserted");
    return proj.files[path];
  }

  renamePath(oldRawPath, newRawPath) {
    const proj = this.getActiveProject();
    if (!proj) return false;
    const oldPath = normalizePath(oldRawPath);
    const newPath = normalizePath(newRawPath);
    if (!oldPath || !newPath || oldPath === newPath) return false;

    // Case 1: Exact file rename
    if (proj.files[oldPath]) {
      if (proj.files[newPath]) {
        throw new Error(`Target '${newPath}' already exists.`);
      }
      const entry = proj.files[oldPath];
      delete proj.files[oldPath];
      entry.path = newPath;
      entry.updatedAt = Date.now();
      entry.dirty = Boolean(proj.hasLocalHandle);
      proj.files[newPath] = entry;
      this.ensureParentFolders(proj, newPath);

      proj.openTabs = proj.openTabs.map((t) => (t === oldPath ? newPath : t));
      if (proj.activeFile === oldPath) {
        proj.activeFile = newPath;
      }
      proj.updatedAt = Date.now();
      this.schedulePersist();
      this.notify("path-renamed");
      return true;
    }

    // Case 2: Folder rename (recursive)
    const folderPrefix = `${oldPath}/`;
    const isFolder =
      (proj.folders || []).includes(oldPath) ||
      Object.keys(proj.files).some((k) => k.startsWith(folderPrefix));
    if (!isFolder) return false;

    const updatedFolders = new Set();
    for (const f of proj.folders || []) {
      if (f === oldPath) {
        updatedFolders.add(newPath);
      } else if (f.startsWith(folderPrefix)) {
        updatedFolders.add(newPath + f.slice(oldPath.length));
      } else {
        updatedFolders.add(f);
      }
    }
    let parent = getParentFolder(newPath);
    while (parent) {
      updatedFolders.add(parent);
      parent = getParentFolder(parent);
    }
    proj.folders = Array.from(updatedFolders).sort();

    for (const filePath of Object.keys(proj.files)) {
      if (filePath.startsWith(folderPrefix)) {
        const relocated = newPath + filePath.slice(oldPath.length);
        const entry = proj.files[filePath];
        delete proj.files[filePath];
        entry.path = relocated;
        entry.updatedAt = Date.now();
        entry.dirty = Boolean(proj.hasLocalHandle);
        proj.files[relocated] = entry;
      }
    }

    proj.openTabs = proj.openTabs.map((t) =>
      t.startsWith(folderPrefix) ? newPath + t.slice(oldPath.length) : t,
    );
    if (proj.activeFile && proj.activeFile.startsWith(folderPrefix)) {
      proj.activeFile = newPath + proj.activeFile.slice(oldPath.length);
    }

    proj.updatedAt = Date.now();
    this.schedulePersist();
    this.notify("path-renamed");
    return true;
  }

  duplicateFile(rawPath) {
    const proj = this.getActiveProject();
    if (!proj) return null;
    const path = normalizePath(rawPath);
    const orig = proj.files[path];
    if (!orig) return null;

    const ext = getExtension(path);
    const stem = ext ? path.slice(0, -(ext.length + 1)) : path;
    let candidate = ext ? `${stem}_copy.${ext}` : `${stem}_copy`;
    let counter = 2;
    while (proj.files[candidate]) {
      candidate = ext ? `${stem}_copy${counter}.${ext}` : `${stem}_copy${counter}`;
      counter++;
    }
    return this.createFile(candidate, orig.content);
  }

  deletePath(rawPath) {
    const proj = this.getActiveProject();
    if (!proj) return false;
    const path = normalizePath(rawPath);
    if (!path) return false;

    const folderPrefix = `${path}/`;
    let deletedAny = false;

    if (proj.files[path]) {
      delete proj.files[path];
      deletedAny = true;
    }

    for (const filePath of Object.keys(proj.files)) {
      if (filePath.startsWith(folderPrefix)) {
        delete proj.files[filePath];
        deletedAny = true;
      }
    }

    const beforeFolders = (proj.folders || []).length;
    proj.folders = (proj.folders || []).filter((f) => f !== path && !f.startsWith(folderPrefix));
    if (proj.folders.length !== beforeFolders) {
      deletedAny = true;
    }

    if (!deletedAny) return false;

    proj.openTabs = proj.openTabs.filter((t) => Boolean(proj.files[t]));
    if (!proj.files[proj.activeFile]) {
      const remaining = Object.keys(proj.files);
      const nextActive =
        proj.openTabs[proj.openTabs.length - 1] ||
        remaining.find((p) => p.endsWith(".py")) ||
        remaining[0] ||
        "";
      proj.activeFile = nextActive;
      if (nextActive && !proj.openTabs.includes(nextActive)) {
        proj.openTabs.push(nextActive);
      }
    }

    proj.updatedAt = Date.now();
    this.schedulePersist();
    this.notify("path-deleted");
    return true;
  }

  openFile(rawPath) {
    const proj = this.getActiveProject();
    if (!proj) return null;
    const path = normalizePath(rawPath);
    if (!proj.files[path]) return null;
    if (!proj.openTabs.includes(path)) {
      proj.openTabs.push(path);
    }
    proj.activeFile = path;
    this.schedulePersist();
    this.notify("file-selected");
    return proj.files[path];
  }

  closeTab(rawPath) {
    const proj = this.getActiveProject();
    if (!proj) return;
    const path = normalizePath(rawPath);
    const idx = proj.openTabs.indexOf(path);
    if (idx === -1) return;

    proj.openTabs.splice(idx, 1);
    if (proj.activeFile === path) {
      const next =
        proj.openTabs[idx] ||
        proj.openTabs[idx - 1] ||
        Object.keys(proj.files).find((p) => p.endsWith(".py")) ||
        Object.keys(proj.files)[0] ||
        "";
      proj.activeFile = next;
      if (next && !proj.openTabs.includes(next)) {
        proj.openTabs.push(next);
      }
    }
    this.schedulePersist();
    this.notify("tab-closed");
  }

  /**
   * Builds a hierarchical tree structure of folders and files for the current project.
   * @param {string} filterQuery Optional search substring
   */
  buildTree(filterQuery = "") {
    const proj = this.getActiveProject();
    if (!proj) return [];
    const q = filterQuery.trim().toLowerCase();

    const root = {
      name: "",
      path: "",
      kind: "folder",
      childrenMap: new Map(),
      files: [],
    };

    const ensureFolderNode = (folderPath) => {
      if (!folderPath) return root;
      const segments = folderPath.split("/");
      let curr = root;
      let accum = "";
      for (const seg of segments) {
        accum = accum ? `${accum}/${seg}` : seg;
        if (!curr.childrenMap.has(seg)) {
          curr.childrenMap.set(seg, {
            name: seg,
            path: accum,
            kind: "folder",
            childrenMap: new Map(),
            files: [],
          });
        }
        curr = curr.childrenMap.get(seg);
      }
      return curr;
    };

    if (!q) {
      for (const folderPath of proj.folders || []) {
        ensureFolderNode(folderPath);
      }
    }

    for (const file of Object.values(proj.files)) {
      if (q && !file.path.toLowerCase().includes(q)) {
        continue;
      }
      const parentPath = getParentFolder(file.path);
      const folderNode = ensureFolderNode(parentPath);
      folderNode.files.push({
        name: getBaseName(file.path),
        path: file.path,
        kind: "file",
        ext: getExtension(file.path),
        dirty: Boolean(file.dirty),
      });
    }

    const toSortedArray = (folderNode) => {
      const subfolders = Array.from(folderNode.childrenMap.values())
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((sub) => ({
          name: sub.name,
          path: sub.path,
          kind: "folder",
          children: toSortedArray(sub),
        }));
      const files = folderNode.files.sort((a, b) => a.name.localeCompare(b.name));
      return [...subfolders, ...files];
    };

    return toSortedArray(root);
  }

  /**
   * Resolves local Python module imports (`from foo import ...` / `import foo`) across the
   * current project and returns `{ bundledSource, lineOffset, importedFiles }`.
   *
   * Because Q-SYS Designer requires a single self-contained `.qplug` or `.lua` script,
   * helper classes and functions from imported sibling `.py` files are prepended in
   * topological order while returning `lineOffset` so Monaco diagnostics and LSP queries
   * map back to the exact line numbers in the active file.
   */
  bundlePythonFile(filePath, liveContent = null) {
    const proj = this.getActiveProject();
    if (!proj) {
      return { bundledSource: liveContent ?? "", lineOffset: 0, importedFiles: [] };
    }

    const normEntry = normalizePath(filePath);
    const entrySource = liveContent !== null ? liveContent : proj.files[normEntry]?.content || "";

    const resolveModulePath = (fromFile, rawSpec) => {
      const cleanSpec = rawSpec.replace(/^\.+/, "");
      if (
        !cleanSpec ||
        cleanSpec === "qsys" ||
        cleanSpec === "typing" ||
        cleanSpec === "__future__"
      ) {
        return null;
      }
      const relPath = cleanSpec.replace(/\./g, "/");
      const dir = getParentFolder(fromFile);
      const candidates = [];
      if (dir) {
        candidates.push(`${dir}/${relPath}.py`);
        candidates.push(`${dir}/${relPath}/__init__.py`);
      }
      candidates.push(`${relPath}.py`);
      candidates.push(`${relPath}/__init__.py`);
      candidates.push(`src/${relPath}.py`);

      for (const cand of candidates) {
        const norm = normalizePath(cand);
        if (norm !== normEntry && proj.files[norm]) {
          return norm;
        }
      }
      return null;
    };

    const extractImportSpecs = (src) => {
      const specs = [];
      const lines = src.split("\n");
      for (const line of lines) {
        const fromMatch = line.match(/^\s*from\s+([.a-zA-Z0-9_]+)\s+import\s+/);
        if (fromMatch) {
          specs.push(fromMatch[1]);
          continue;
        }
        const impMatch = line.match(/^\s*import\s+([a-zA-Z0-9_.]+)/);
        if (impMatch) {
          specs.push(impMatch[1]);
        }
      }
      return specs;
    };

    const visited = new Set([normEntry]);
    const orderedModules = [];

    const visit = (modPath) => {
      const src = proj.files[modPath]?.content || "";
      for (const spec of extractImportSpecs(src)) {
        const resolved = resolveModulePath(modPath, spec);
        if (resolved && !visited.has(resolved)) {
          visited.add(resolved);
          visit(resolved);
          orderedModules.push(resolved);
        }
      }
    };

    for (const spec of extractImportSpecs(entrySource)) {
      const resolved = resolveModulePath(normEntry, spec);
      if (resolved && !visited.has(resolved)) {
        visited.add(resolved);
        visit(resolved);
        orderedModules.push(resolved);
      }
    }

    if (orderedModules.length === 0) {
      return { bundledSource: entrySource, lineOffset: 0, importedFiles: [] };
    }

    const preambleLines = [];
    for (const modPath of orderedModules) {
      const modSource = proj.files[modPath]?.content || "";
      preambleLines.push(`# --- Bundled module: ${modPath} ---`);
      for (const line of modSource.split("\n")) {
        preambleLines.push(line);
      }
    }
    preambleLines.push("# --- End bundled modules ---");

    const lineOffset = preambleLines.length;
    const bundledSource = `${preambleLines.join("\n")}\n${entrySource}`;
    return {
      bundledSource,
      lineOffset,
      importedFiles: orderedModules,
    };
  }

  // =========================================================================
  // Local Filesystem Integration (File System Access API + Input Fallbacks)
  // =========================================================================

  /**
   * Opens a local directory using `window.showDirectoryPicker()` when available.
   * Reads all text/source files recursively and creates a linked project.
   */
  async openLocalDirectoryPicker() {
    if (typeof window.showDirectoryPicker !== "function") {
      return null; // Caller will fall back to <input type="file" webkitdirectory>
    }

    const dirHandle = await window.showDirectoryPicker({ mode: "readwrite" });
    const filesMap = await this.readDirectoryHandleRecursive(dirHandle, "");

    if (this.stubsContent && !filesMap["typings/qsys.pyi"]) {
      filesMap["typings/qsys.pyi"] = this.stubsContent;
    }
    if (!filesMap["pyrightconfig.json"]) {
      filesMap["pyrightconfig.json"] = PYRIGHT_CONFIG;
    }

    const project = this.createProject(dirHandle.name, "custom", {
      files: filesMap,
      folders: [],
    });
    project.hasLocalHandle = true;
    project.localFolderName = dirHandle.name;

    this.dirHandles.set(project.id, dirHandle);
    await idbSet("dir_handles", project.id, dirHandle);
    await this.persistNow();
    this.notify("local-folder-linked");
    return project;
  }

  async readDirectoryHandleRecursive(dirHandle, prefix = "") {
    const result = {};
    for await (const [name, entry] of dirHandle.entries()) {
      if (entry.kind === "directory") {
        if (IGNORED_DIRS.has(name) || name.startsWith(".")) continue;
        const subPrefix = prefix ? `${prefix}/${name}` : name;
        const subFiles = await this.readDirectoryHandleRecursive(entry, subPrefix);
        Object.assign(result, subFiles);
      } else if (entry.kind === "file") {
        if (!isSupportedTextFile(name)) continue;
        const file = await entry.getFile();
        if (file.size > 2 * 1024 * 1024) continue; // Skip >2MB files
        const content = await file.text();
        const relPath = prefix ? `${prefix}/${name}` : name;
        result[normalizePath(relPath)] = content;
      }
    }
    return result;
  }

  /**
   * Reloads files from the linked local directory handle on disk.
   */
  async reloadFromLocalDisk() {
    const proj = this.getActiveProject();
    if (!proj) return false;
    const dirHandle = this.dirHandles.get(proj.id);
    if (!dirHandle) {
      throw new Error("No local directory handle is linked to this project.");
    }

    const perm = await this.verifyDirectoryPermission(dirHandle, "read");
    if (!perm) {
      throw new Error("Read permission for the local folder was denied.");
    }

    const diskFiles = await this.readDirectoryHandleRecursive(dirHandle, "");
    const foldersSet = new Set();
    const newFilesMap = {};

    for (const [path, content] of Object.entries(diskFiles)) {
      newFilesMap[path] = {
        path,
        content,
        updatedAt: Date.now(),
        dirty: false,
      };
      let parent = getParentFolder(path);
      while (parent) {
        foldersSet.add(parent);
        parent = getParentFolder(parent);
      }
    }

    if (this.stubsContent && !newFilesMap["typings/qsys.pyi"]) {
      newFilesMap["typings/qsys.pyi"] = {
        path: "typings/qsys.pyi",
        content: this.stubsContent,
        updatedAt: Date.now(),
        dirty: false,
      };
      foldersSet.add("typings");
    }

    proj.files = newFilesMap;
    proj.folders = Array.from(foldersSet).sort();
    proj.openTabs = proj.openTabs.filter((t) => Boolean(proj.files[t]));
    if (!proj.files[proj.activeFile]) {
      const allPaths = Object.keys(proj.files);
      proj.activeFile =
        proj.openTabs[0] || allPaths.find((p) => p.endsWith(".py")) || allPaths[0] || "";
      if (proj.activeFile && !proj.openTabs.includes(proj.activeFile)) {
        proj.openTabs.push(proj.activeFile);
      }
    }

    await this.persistNow();
    this.notify("reloaded-from-disk");
    return true;
  }

  /**
   * Syncs/Exports the active project to a local folder on disk via the File System Access API.
   * If the project does not yet have a linked directory handle, prompts the user to pick one!
   */
  async syncToLocalDisk({ compiledOutputs = {}, includeStubs = true } = {}) {
    const proj = this.getActiveProject();
    if (!proj) return null;

    let dirHandle = this.dirHandles.get(proj.id);
    if (!dirHandle) {
      if (typeof window.showDirectoryPicker !== "function") {
        throw new Error(
          "Your browser does not support direct local folder writing (File System Access API). Use Export .zip instead.",
        );
      }
      dirHandle = await window.showDirectoryPicker({ mode: "readwrite" });
      this.dirHandles.set(proj.id, dirHandle);
      proj.hasLocalHandle = true;
      proj.localFolderName = dirHandle.name;
      await idbSet("dir_handles", proj.id, dirHandle);
    }

    const hasPerm = await this.verifyDirectoryPermission(dirHandle, "readwrite");
    if (!hasPerm) {
      throw new Error("Write permission to the local directory was not granted.");
    }

    let writtenCount = 0;

    // 1. Write all project files
    for (const file of Object.values(proj.files)) {
      if (!includeStubs && file.path === "typings/qsys.pyi") continue;
      await this.writeFileToDirectoryHandle(dirHandle, file.path, file.content);
      file.dirty = false;
      writtenCount++;
    }

    // 2. Write compiled .qplug / .lua artifacts if provided
    for (const [outPath, luaCode] of Object.entries(compiledOutputs)) {
      if (luaCode) {
        await this.writeFileToDirectoryHandle(dirHandle, outPath, luaCode);
        writtenCount++;
      }
    }

    await this.persistNow();
    this.notify("synced-to-disk");
    return { folderName: dirHandle.name, writtenCount };
  }

  async verifyDirectoryPermission(dirHandle, mode = "readwrite") {
    if (!dirHandle) return false;
    if (typeof dirHandle.queryPermission === "function") {
      const opts = { mode };
      if ((await dirHandle.queryPermission(opts)) === "granted") {
        return true;
      }
      if ((await dirHandle.requestPermission(opts)) === "granted") {
        return true;
      }
      return false;
    }
    return true;
  }

  async writeFileToDirectoryHandle(rootDirHandle, relativePath, content) {
    const norm = normalizePath(relativePath);
    const parts = norm.split("/");
    const fileName = parts.pop();
    let currDir = rootDirHandle;
    for (const dirName of parts) {
      currDir = await currDir.getDirectoryHandle(dirName, { create: true });
    }
    const fileHandle = await currDir.getFileHandle(fileName, { create: true });
    const writable = await fileHandle.createWritable();
    await writable.write(content);
    await writable.close();
  }

  /**
   * Fallback import for `<input type="file" webkitdirectory multiple>` or multi-file picker.
   */
  async importFromFileList(fileList, { asNewProject = false, projectName = "" } = {}) {
    const files = Array.from(fileList || []);
    if (files.length === 0) return null;

    // Check if single .qsysproj.json snapshot
    if (files.length === 1 && files[0].name.endsWith(".qsysproj.json")) {
      const text = await files[0].text();
      return this.importProjectJson(text);
    }

    // Check if single .zip archive
    if (files.length === 1 && files[0].name.toLowerCase().endsWith(".zip")) {
      const buf = await files[0].arrayBuffer();
      const entries = await extractZipEntries(buf);
      const zipProjName =
        projectName || files[0].name.replace(/\.zip$/i, "") || "Imported ZIP Project";
      const filesMap = {};
      for (const e of entries) {
        if (isSupportedTextFile(e.path)) {
          filesMap[normalizePath(e.path)] = e.content;
        }
      }
      if (this.stubsContent && !filesMap["typings/qsys.pyi"]) {
        filesMap["typings/qsys.pyi"] = this.stubsContent;
      }
      return this.createProject(zipProjName, "custom", {
        files: filesMap,
        folders: [],
      });
    }

    // Otherwise import directory tree (via webkitRelativePath) or individual files
    const collected = {};
    let detectedRootFolder = "";

    for (const file of files) {
      const rel = file.webkitRelativePath || file.name;
      const parts = rel.replace(/\\/g, "/").split("/");
      if (parts.some((p) => IGNORED_DIRS.has(p))) continue;
      if (!isSupportedTextFile(file.name)) continue;

      if (parts.length > 1 && !detectedRootFolder) {
        detectedRootFolder = parts[0];
      }
      const strippedPath = parts.length > 1 ? parts.slice(1).join("/") : file.name;
      collected[normalizePath(strippedPath)] = await file.text();
    }

    if (Object.keys(collected).length === 0) {
      throw new Error("No supported text or Python files found in selection.");
    }

    if (asNewProject || detectedRootFolder) {
      const name = projectName || detectedRootFolder || "Imported Project";
      if (this.stubsContent && !collected["typings/qsys.pyi"]) {
        collected["typings/qsys.pyi"] = this.stubsContent;
      }
      if (!collected["pyrightconfig.json"]) {
        collected["pyrightconfig.json"] = PYRIGHT_CONFIG;
      }
      return this.createProject(name, "custom", {
        files: collected,
        folders: [],
      });
    }

    // Add files into the currently active project
    let lastPath = "";
    for (const [path, content] of Object.entries(collected)) {
      this.upsertFile(path, content, true);
      lastPath = path;
    }
    if (lastPath) {
      this.openFile(lastPath);
    }
    return this.getActiveProject();
  }

  importProjectJson(jsonString) {
    const data = JSON.parse(jsonString);
    if (!data || typeof data.files !== "object") {
      throw new Error("Invalid .qsysproj.json file format.");
    }
    const filesMap = {};
    for (const [k, v] of Object.entries(data.files)) {
      filesMap[normalizePath(k)] = typeof v === "string" ? v : v.content || "";
    }
    if (this.stubsContent && !filesMap["typings/qsys.pyi"]) {
      filesMap["typings/qsys.pyi"] = this.stubsContent;
    }
    return this.createProject(data.name || "Imported Project", "custom", {
      files: filesMap,
      folders: data.folders || [],
      openTabs: data.openTabs,
      activeFile: data.activeFile,
    });
  }

  exportProjectJsonBlob() {
    const proj = this.getActiveProject();
    if (!proj) return null;
    const payload = {
      format: "qsyspy-project-v1",
      name: proj.name,
      exportedAt: new Date().toISOString(),
      folders: proj.folders,
      openTabs: proj.openTabs,
      activeFile: proj.activeFile,
      files: Object.fromEntries(Object.entries(proj.files).map(([k, v]) => [k, v.content])),
    };
    return new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
  }

  exportProjectZipBlob({ compiledOutputs = {}, includeStubs = true } = {}) {
    const proj = this.getActiveProject();
    if (!proj) return null;

    const entries = [];
    for (const file of Object.values(proj.files)) {
      if (!includeStubs && file.path === "typings/qsys.pyi") continue;
      entries.push({ path: file.path, content: file.content });
    }
    if (includeStubs && this.stubsContent && !proj.files["typings/qsys.pyi"]) {
      entries.push({ path: "typings/qsys.pyi", content: this.stubsContent });
    }
    for (const [outPath, luaCode] of Object.entries(compiledOutputs)) {
      if (luaCode && !proj.files[outPath]) {
        entries.push({ path: outPath, content: luaCode });
      }
    }

    return createZipBlob(entries);
  }
}
