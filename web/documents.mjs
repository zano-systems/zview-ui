export const MAX_DOCUMENT_BYTES = 16 * 1024 * 1024;

// Startup and picker loads share one ordering rule: only the newest request
// may install a document or report an error. Source metadata can arrive
// asynchronously; known sizes are checked before reading the file contents.
export function createDocumentLoader({ install, reportError }) {
  let generation = 0;

  return async function loadDocument(readSource) {
    const request = ++generation;
    try {
      const source = await readSource();
      if (request !== generation || source === null) return;
      if (source.size > MAX_DOCUMENT_BYTES) {
        throw new Error("Dump exceeds the 16 MiB limit.");
      }
      const bytes = await source.readBytes();
      if (request !== generation) return;
      if (bytes.byteLength > MAX_DOCUMENT_BYTES) {
        throw new Error("Dump exceeds the 16 MiB limit.");
      }
      const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      install(text, source.name);
    } catch (error) {
      if (request === generation) {
        reportError(String(error?.message ?? error));
      }
    }
  };
}
