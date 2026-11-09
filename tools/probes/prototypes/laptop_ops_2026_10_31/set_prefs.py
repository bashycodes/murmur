"""Edit %APPDATA%/Murmur/prefs.json for the laptop model setup (app must be closed).
Local cleanup on with Llama-3.2-3B (Vulkan picked by the app's llm_backend probe),
cleanup order local > server > cloud (enabled flags kept; cloud stays off via
allow_cloud=false), Hebrew = zipformer, keyboard language on, hands-free pause 0."""
import json, os, sys
d = os.path.join(os.environ["APPDATA"], "Murmur")
p = os.path.join(d, "prefs.json")
j = json.load(open(p, encoding="utf-8"))
models = os.path.join(d, "models")
j["llm_enabled"] = True
j["llm_model_path"] = os.path.join(models, "Llama-3.2-3B-Instruct-Q4_K_M.gguf")
j["llm_lib_path"] = None
j["hebrew_model"] = "zipformer"
j["keyboard_language"] = True
j["hands_free_silence_ms"] = 0
dev = j["developer"]
eng = {e["route"]: e for e in dev.get("cleanup_engines", [])}
order = ["local", "server", "cloud"]
dev["cleanup_engines"] = [eng.get(r, {"route": r, "enabled": r == "local"}) for r in order]
dev["cleanup_engines"][0]["enabled"] = True
dev["allow_cloud"] = False

tmp = p + ".tmp"
json.dump(j, open(tmp, "w", encoding="utf-8"), ensure_ascii=False, indent=2)
os.replace(tmp, p)
print(json.dumps({k: j[k] for k in ["llm_enabled", "llm_model_path", "hebrew_model", "keyboard_language",
                                    "hands_free_silence_ms", "languages", "asr_engine", "prefs_rev"]}, ensure_ascii=False))
print(dev["cleanup_engines"], "allow_cloud", dev["allow_cloud"], "debug_log(kept)", dev.get("debug_log"))
