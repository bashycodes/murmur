// D31/D34: what the download confirm dialog shows. Runs on the built dist: `npm test`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { confirmFacts, dirname, LID_FILE } from "../dist/lib/download-confirm.js";

const lid = { name: LID_FILE, url: "https://huggingface.co/x/resolve/abc/ggml-tiny-q5_1.bin", extra: true };
const hebrew = {
  title: "Zipformer Hebrew",
  page: "https://huggingface.co/x",
  dest: "C:\\Data\\Murmur\\models\\zipformer-he",
  size: 104_000_000,
  installed: true,
  complete: false,
  missing: 32_152_673,
  files: [
    { name: "encoder.onnx", url: "https://huggingface.co/x/resolve/abc/encoder.onnx" },
    { name: "tokens.txt", url: "https://huggingface.co/x/resolve/abc/tokens.txt" },
    lid,
  ],
};

test("a Hebrew model missing only the language-ID model asks for just that file", () => {
  assert.deepEqual(confirmFacts(hebrew, "home server"), {
    title: "the language-detection model",
    from: lid.url,
    size: 32_152_673,
    files: 1,
    dest: "C:\\Data\\Murmur\\models",
  });
  assert.equal(confirmFacts({ ...hebrew, from_server: true }, "home server").from, "home server");
});

test("a model not yet installed, or complete, shows the whole model", () => {
  const fresh = confirmFacts({ ...hebrew, installed: false, missing: hebrew.size }, "s");
  assert.equal(fresh.title, "Zipformer Hebrew");
  assert.equal(fresh.size, hebrew.size);
  assert.equal(fresh.files, 3);
  assert.equal(fresh.from, hebrew.page);
  assert.equal(fresh.dest, hebrew.dest);
  // Older backend without `complete`: the whole model.
  assert.equal(confirmFacts({ ...hebrew, complete: undefined }, "s").title, "Zipformer Hebrew");
  const single = { ...hebrew, files: [{ name: "m.bin", url: "https://huggingface.co/y/resolve/abc/m.bin" }], complete: true, missing: 0 };
  assert.equal(confirmFacts(single, "s").from, "https://huggingface.co/y/resolve/abc/m.bin");
});

test("dirname handles Windows and POSIX paths", () => {
  assert.equal(dirname("C:\\a\\b\\c.bin"), "C:\\a\\b");
  assert.equal(dirname("/x/y/z"), "/x/y");
  assert.equal(dirname("plain"), "plain");
});
