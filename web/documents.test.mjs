import assert from "node:assert/strict";
import { test } from "node:test";
import { createDocumentLoader, MAX_DOCUMENT_BYTES } from "./documents.mjs";

const bytes = (text) => new TextEncoder().encode(text);
const source = (name, text = '{"threads":[]}') => ({
  name,
  readBytes: async () => bytes(text),
});

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function setup() {
  const installed = [];
  const errors = [];
  const load = createDocumentLoader({
    install: (text, name) => installed.push({ text, name }),
    reportError: (error) => errors.push(error),
  });
  return { load, installed, errors };
}

test("loads decoded UTF-8 text and accepts an absent initial source", async () => {
  const { load, installed, errors } = setup();
  await load(() => null);
  await load(() => source("snapshot.json", '{"name":"mémoire"}'));
  assert.deepEqual(installed, [{ name: "snapshot.json", text: '{"name":"mémoire"}' }]);
  assert.deepEqual(errors, []);
});

test("oversized files are rejected before reading and a new selection recovers", async () => {
  const { load, installed, errors } = setup();
  await load(() => source("previous.json"));
  await load(() => ({
    name: "large.json",
    size: MAX_DOCUMENT_BYTES + 1,
    readBytes: () => { assert.fail("must reject before reading"); },
  }));
  assert.deepEqual(installed.map(({ name }) => name), ["previous.json"]);
  assert.match(errors[0], /16 MiB/);
  await load(() => source("recovery.json"));
  assert.deepEqual(installed.map(({ name }) => name), ["previous.json", "recovery.json"]);
});

test("checks actual byte length when the size is not known in advance", async () => {
  const { load, installed, errors } = setup();
  await load(() => ({
    name: "large.json",
    readBytes: async () => new Uint8Array(MAX_DOCUMENT_BYTES + 1),
  }));
  assert.deepEqual(installed, []);
  assert.match(errors[0], /16 MiB/);
});

test("accepts exactly the byte limit", async () => {
  const { load, installed, errors } = setup();
  const text = " ".repeat(MAX_DOCUMENT_BYTES);
  await load(() => ({ ...source("boundary.json", text), size: MAX_DOCUMENT_BYTES }));
  assert.equal(installed[0].text.length, MAX_DOCUMENT_BYTES);
  assert.deepEqual(errors, []);
});

test("invalid UTF-8 and read failures preserve the document and allow recovery", async () => {
  const { load, installed, errors } = setup();
  await load(() => source("previous.json"));
  await load(() => ({ name: "invalid.json", readBytes: async () => new Uint8Array([255]) }));
  await load(() => ({
    name: "unreadable.json",
    readBytes: async () => { throw new Error("Cannot read file."); },
  }));
  assert.equal(installed.length, 1);
  assert.equal(errors.length, 2);
  assert.equal(errors[1], "Cannot read file.");
  await load(() => source("recovery.json"));
  assert.equal(installed.at(-1).name, "recovery.json");
});

test("a slower earlier picker selection cannot overwrite the latest selection", async () => {
  const { load, installed, errors } = setup();
  const olderBytes = deferred();
  const older = load(() => ({ name: "older.json", readBytes: () => olderBytes.promise }));
  // Let the earlier source begin reading before making another selection.
  await Promise.resolve();
  await load(() => source("newer.json"));
  olderBytes.resolve(bytes('{"old":true}'));
  await older;
  assert.deepEqual(installed.map(({ name }) => name), ["newer.json"]);
  assert.deepEqual(errors, []);
});

test("an explicit picker selection invalidates pending initial metadata", async () => {
  const { load, installed, errors } = setup();
  const metadata = deferred();
  const initial = load(() => metadata.promise);
  await load(() => source("user-choice.json"));
  metadata.resolve({
    name: "initial.json",
    readBytes: () => { assert.fail("stale metadata must not start reading"); },
  });
  await initial;
  assert.deepEqual(installed.map(({ name }) => name), ["user-choice.json"]);
  assert.deepEqual(errors, []);
});

test("an explicit picker selection invalidates pending initial bytes", async () => {
  const { load, installed, errors } = setup();
  const initialBytes = deferred();
  const initial = load(() => ({ name: "initial.json", readBytes: () => initialBytes.promise }));
  await Promise.resolve();
  await load(() => source("user-choice.json"));
  initialBytes.resolve(bytes('{"initial":true}'));
  await initial;
  assert.deepEqual(installed.map(({ name }) => name), ["user-choice.json"]);
  assert.deepEqual(errors, []);
});

test("stale failures cannot replace a newer document or its error state", async () => {
  const { load, installed, errors } = setup();
  const olderBytes = deferred();
  const older = load(() => ({ name: "older.json", readBytes: () => olderBytes.promise }));
  await Promise.resolve();
  await load(() => source("newer.json"));
  olderBytes.reject(new Error("Stale read error."));
  await older;
  assert.deepEqual(installed.map(({ name }) => name), ["newer.json"]);
  assert.deepEqual(errors, []);
});

test("a failed newer selection still invalidates an older successful load", async () => {
  const { load, installed, errors } = setup();
  const olderBytes = deferred();
  const older = load(() => ({ name: "older.json", readBytes: () => olderBytes.promise }));
  await Promise.resolve();
  await load(() => ({
    name: "newer.json",
    readBytes: async () => { throw new Error("New selection failed."); },
  }));
  olderBytes.resolve(bytes("{}"));
  await older;
  assert.deepEqual(installed, []);
  assert.deepEqual(errors, ["New selection failed."]);
});
