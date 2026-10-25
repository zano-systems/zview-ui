import init, { start, load_json } from "./pkg/zview_desktop.js";

const LIMIT = 16 * 1024 * 1024;
const status = document.querySelector("#status");
const picker = document.querySelector("#json-file");

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
    if (file.size > LIMIT) throw new Error("Dump exceeds the 16 MiB limit.");
    const text = new TextDecoder("utf-8", { fatal: true }).decode(await file.arrayBuffer());
    load_json(text, file.name);
    status.hidden = true;
  } catch (error) {
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

  // Only the local launcher provides a session. Static hosting uses the sample.
  const session = await fetch(new URL("./session.json", import.meta.url));
  if (session.ok) {
    const { name } = await session.json();
    if (name) {
      const bytes = await getBytes("./initial.json");
      if (bytes.length > LIMIT) throw new Error("Dump exceeds the 16 MiB limit.");
      load_json(new TextDecoder("utf-8", { fatal: true }).decode(bytes), name);
    }
  } else if (session.status !== 404) {
    throw new Error(`Cannot load initial session: HTTP ${session.status}`);
  }
  status.hidden = true;
  // Signals host startup, not proof that the GPU has finished its first frame.
  document.documentElement.dataset.zviewReady = "true";
} catch (error) {
  showError(error);
}
