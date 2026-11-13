// Tiny PCM16 WAV reader/writer, enough to join TTS clips with silence.

/** @returns {{rate:number, channels:number, bits:number, data:Buffer}} */
export function readWav(buf) {
  if (buf.toString("ascii", 0, 4) !== "RIFF" || buf.toString("ascii", 8, 12) !== "WAVE") throw new Error("not a WAV");
  let pos = 12;
  let fmt = null;
  while (pos + 8 <= buf.length) {
    const id = buf.toString("ascii", pos, pos + 4);
    let len = buf.readUInt32LE(pos + 4);
    if (id === "fmt ") {
      fmt = { tag: buf.readUInt16LE(pos + 8), channels: buf.readUInt16LE(pos + 10), rate: buf.readUInt32LE(pos + 12), bits: buf.readUInt16LE(pos + 22) };
    } else if (id === "data") {
      // espeak-ng streaming to a file leaves 0x7fffffff here.
      len = Math.min(len, buf.length - pos - 8);
      if (!fmt || fmt.tag !== 1 || fmt.bits !== 16) throw new Error("only PCM16 WAV is supported");
      return { rate: fmt.rate, channels: fmt.channels, bits: 16, data: buf.subarray(pos + 8, pos + 8 + len) };
    }
    pos += 8 + len + (len & 1);
  }
  throw new Error("WAV without data chunk");
}

export function writeWav({ rate, channels, data }) {
  const h = Buffer.alloc(44);
  h.write("RIFF", 0, "ascii");
  h.writeUInt32LE(36 + data.length, 4);
  h.write("WAVEfmt ", 8, "ascii");
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(channels, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * channels * 2, 28);
  h.writeUInt16LE(channels * 2, 32);
  h.writeUInt16LE(16, 34);
  h.write("data", 36, "ascii");
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

/** Join same-format clips with `gapMs` of silence between and around them. */
export function joinWavs(wavs, gapMs = 300) {
  const [first] = wavs;
  for (const w of wavs) if (w.rate !== first.rate || w.channels !== first.channels) throw new Error("clips differ in format");
  const silence = Buffer.alloc(Math.round((first.rate * gapMs) / 1000) * first.channels * 2);
  const parts = [silence];
  for (const w of wavs) parts.push(w.data, silence);
  return { rate: first.rate, channels: first.channels, data: Buffer.concat(parts) };
}

export function durationMs(w) {
  return Math.round((w.data.length / (w.channels * 2) / w.rate) * 1000);
}
