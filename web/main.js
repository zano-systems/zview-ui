import init, { start, load_json, show_file_error } from "./pkg/zview_desktop.js";
import { createDocumentLoader } from "./documents.mjs";

const status = document.querySelector("#status");
const picker = document.querySelector("#json-file");
const loadDocument = createDocumentLoader({
  install: load_json,
  reportError: show_file_error,
});

function showError(error) {
  console.error(error);
  status.textContent = `ZView: ${error.message ?? error}`;
  status.hidden = false;
}

async function getBytes(path) {
  const response = await fetch(new URL(path, import.meta.url));
  if (!response.ok) throw new Error(`Cannot load ${path}: HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

// Called by the shared GPUI session menu during a user-initiated click.
window.zviewOpenJson = () => {
  picker.value = "";
  picker.click();
};

picker.addEventListener("change", async () => {
  const file = picker.files[0];
  if (!file) return;
  try {
    await loadDocument(() => ({
      name: file.name,
      size: file.size,
      readBytes: () => file.arrayBuffer(),
    }));
  } catch (error) {
    // File errors use the shared in-app banner. This is only a host failure,
    // such as losing the GPUI window while trying to report that error.
    showError(error);
  }
});

try {
  const [, uiFont, monoFont] = await Promise.all([
    init(),
    getBytes("./fonts/IBMPlexSans-Regular.source.ttf"),
    getBytes("./fonts/JetBrainsMono-Regular.source.ttf"),
  ]);
  await Promise.race([
    start(uiFont, monoFont),
    new Promise((_, reject) => setTimeout(
      () => reject(new Error("Graphics initialization timed out. Try a browser with WebGPU or WebGL2 enabled.")),
      60000,
    )),
  ]);

  // The window is usable now, even if the initial JSON is still being fetched.
  // Recoverable load errors must not restore the startup overlay over its menu.
  status.hidden = true;
  // Only the local launcher provides a session. Static hosting uses the sample.
  await loadDocument(async () => {
    const session = await fetch(new URL("./session.json", import.meta.url));
    if (session.ok) {
      const { name } = await session.json();
      return name ? {
        name,
        readBytes: () => getBytes("./initial.json"),
      } : null;
    }
    if (session.status !== 404) {
      throw new Error(`Cannot load initial session: HTTP ${session.status}`);
    }
    return null;
  });
  // Signals host startup, not proof that the GPU has finished its first frame.
  document.documentElement.dataset.zviewReady = "true";
} catch (error) {
  showError(error);
}
