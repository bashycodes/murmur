"""FLEURS float32 WAV -> 16 kHz mono PCM16 for the clips in a manifest (stdlib only).
usage: python3 -I fleurs_to_pcm16.py MANIFEST.json SRC_DIR OUT_DIR"""
import array, json, os, struct, sys, wave

man, src, out = sys.argv[1:4]
os.makedirs(out, exist_ok=True)
for u in json.load(open(man, encoding="utf-8"))["utterances"]:
    b = open(os.path.join(src, u["id"] + ".wav"), "rb").read()
    fmt, ch, sr = struct.unpack("<HHI", b[20:28])
    bits = struct.unpack("<H", b[34:36])[0]
    assert fmt == 3 and ch == 1 and sr == 16000 and bits == 32, (u["id"], fmt, ch, sr, bits)
    i = 12
    while b[i:i + 4] != b"data":
        i += 8 + struct.unpack("<I", b[i + 4:i + 8])[0]
    n = struct.unpack("<I", b[i + 4:i + 8])[0]
    f = array.array("f", b[i + 8:i + 8 + n])
    pcm = array.array("h", (max(-32768, min(32767, int(round(x * 32767)))) for x in f))
    with wave.open(os.path.join(out, u["id"] + ".wav"), "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000); w.writeframes(pcm.tobytes())
print("ok")
