#!/usr/bin/env python3
"""Generate crates/flow-contract-tests/fixtures/sessions/*.json.

Each fixture is one dictation the equivalence suite runs both locally
(FlowCore) and through Router -> flow-remote -> flow-server, comparing the
results byte for byte (minus timings/route):

  {name, asr: {"mock": text} | "whisper-tiny" | "parakeet",
   audio: "speech:MS" | "silence:MS" | "noise:MS" | "noise+speech:MS" | "jfk.wav",
   session_config: <SessionConfig>, contexts: [{at_ms, ctx: <FieldContext>}],
   learn?: {pasted, edited}}

Sources: the Wispr ground-truth post-processing vectors in
crates/flow-core/tests/vectors/ (every case expressible as ASR text + config
+ context) plus hand-written core-p1 cases (spoken cleanup, empty reasons).
Re-run after the vectors change; the output is deterministic.
"""
import json
import os
import re
import shutil
import sys
from urllib.parse import urlparse

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
VEC = os.path.join(ROOT, "crates/flow-core/tests/vectors")
OUT = os.path.join(ROOT, "crates/flow-contract-tests/fixtures/sessions")

STAGES = {"pipeline", "match_context", "dictionary_replace", "press_enter",
          "cursor_tag", "slack_tag", "app_type"}
STYLE = {"formal": "formal", "casual": "casual", "veryCasual": "very_casual",
         "excited": "excited"}


def slug(s):
    return re.sub(r"[^a-z0-9_]+", "_", s.lower()).strip("_")


def ctx_from(inp, platform):
    app = inp.get("app") or {}
    url = app.get("url") or ""
    host = urlparse(url).hostname or "" if url else ""
    c = {
        "app_id": app.get("bundle_id") or inp.get("package") or "",
        "url_host": host,
        "before": inp.get("before", ""),
        "after": inp.get("after", ""),
        "selected": inp.get("selected", ""),
        "screen_words": inp.get("screen_words", []),
        "file_names": inp.get("file_names", []),
    }
    return {k: v for k, v in c.items() if v not in ("", [])}


def session_config(inp, platform):
    sc = {"platform": platform, "dictionary": inp.get("dictionary", [])}
    st = STYLE.get(inp.get("style", ""))
    if st:
        sc["styles"] = {"personal": st, "work": st, "email": st, "other": st}
    names = [inp[k] for k in ("first_name", "last_name") if inp.get(k)]
    if names:
        sc["user_names"] = names
    if inp.get("format_mode") == "all_lowercase":
        sc["all_lowercase"] = True
    return sc


def from_vectors():
    out = []
    win = json.load(open(os.path.join(VEC, "windows.json")))["vectors"]
    for v in win:
        inp = v["input"]
        if v["stage"] not in STAGES or not inp.get("text"):
            continue
        text = inp.get("raw_asr") or inp["text"]
        ctx = ctx_from(inp, "windows")
        late = len(out) % 3 == 0  # every third case: context after the audio
        out.append({
            "name": f"win_{slug(v['name'])}",
            "asr": {"mock": text},
            "audio": "speech:1200",
            "session_config": session_config(inp, "windows"),
            "contexts": [{"at_ms": 1200 if late else 0, "ctx": ctx}] if ctx else [],
        })
    andr = json.load(open(os.path.join(VEC, "android.json")))["vectors"]
    for v in andr:
        inp = v["input"]
        if v["stage"] != "insert_text" or not inp.get("text"):
            continue
        ctx = ctx_from(inp, "android")
        out.append({
            "name": f"and_{slug(v['name'])}",
            "asr": {"mock": inp["text"]},
            "audio": "speech:1200",
            "session_config": session_config(inp, "android"),
            "contexts": [{"at_ms": 0, "ctx": ctx}] if ctx else [],
        })
    return out


def core_p1():
    m = lambda t: {"mock": t}
    return [
        {"name": "p1_spoken_cleanup_fillers_backtrack", "asr": m("um so we meet at five no wait six pm comma okay"),
         "audio": "speech:2000", "session_config": {}, "contexts": []},
        {"name": "p1_spoken_cleanup_money_date", "asr": m("it costs twenty five dollars on march third"),
         "audio": "speech:1500", "session_config": {}, "contexts": []},
        {"name": "p1_spoken_cleanup_off", "asr": m("um so uh hello"),
         "audio": "speech:1000", "session_config": {"spoken_cleanup": False}, "contexts": []},
        {"name": "p1_cleanup_level_none_verbatim", "asr": m("um so period hello"),
         "audio": "speech:1000", "session_config": {"cleanup_level": "none"}, "contexts": []},
        {"name": "p1_only_filler", "asr": m("um uh"), "audio": "speech:800",
         "session_config": {}, "contexts": []},
        {"name": "p1_no_speech_silence", "asr": m("never used"), "audio": "silence:1500",
         "session_config": {}, "contexts": []},
        {"name": "p1_no_speech_room_noise", "asr": m("never used"), "audio": "noise:1500",
         "session_config": {}, "contexts": []},
        {"name": "p1_no_audio", "asr": m("never used"), "audio": "speech:0",
         "session_config": {}, "contexts": []},
        {"name": "p1_blank_asr", "asr": m(""), "audio": "speech:1000",
         "session_config": {}, "contexts": []},
        {"name": "p1_hallucination_thank_you", "asr": m("Thank you."), "audio": "speech:60",
         "session_config": {}, "contexts": []},
        {"name": "p1_hallucination_guard_off", "asr": m("Thank you."), "audio": "speech:60",
         "session_config": {"hallucination_guard": False}, "contexts": []},
        {"name": "p1_vad_off_noise", "asr": m("noise words"), "audio": "noise:1000",
         "session_config": {"vad": False}, "contexts": []},
        {"name": "p1_noise_plus_speech", "asr": m("hello there"), "audio": "noise+speech:1500",
         "session_config": {}, "contexts": []},
        {"name": "p1_press_enter_segments", "asr": m("send it now press enter"),
         "audio": "speech:1200", "session_config": {}, "contexts": [{"at_ms": 0, "ctx": {"app_id": "slack"}}]},
        {"name": "p1_press_enter_disabled", "asr": m("send it now press enter"),
         "audio": "speech:1200", "session_config": {"press_enter_enabled": False}, "contexts": []},
        {"name": "p1_break_and_tab_markers", "asr": m("first line new line second tab third"),
         "audio": "speech:1200", "session_config": {}, "contexts": [{"at_ms": 0, "ctx": {"app_id": "cursor", "file_names": ["main.rs"]}}]},
        {"name": "p1_all_lowercase", "asr": m("Hello There Bob"), "audio": "speech:1000",
         "session_config": {"all_lowercase": True}, "contexts": []},
        {"name": "p1_email_signature", "asr": m("hi team the report is attached thanks"),
         "audio": "speech:1500", "session_config": {"email_signature": "Sent by voice"},
         "contexts": [{"at_ms": 0, "ctx": {"app_id": "outlook", "app_category": "email"}}]},
        {"name": "p1_forced_language", "asr": m("bonjour tout le monde"), "audio": "speech:1000",
         "session_config": {"languages": ["fr"]}, "contexts": []},
        {"name": "p1_two_contexts_last_wins", "asr": m("hello world"), "audio": "speech:1000",
         "session_config": {}, "contexts": [{"at_ms": 0, "ctx": {"app_id": "notepad", "before": "Dear "}},
                                             {"at_ms": 1000, "ctx": {"app_id": "slack", "before": ""}}]},
        {"name": "p1_learn_edit", "asr": m("I use whisper flow daily"), "audio": "speech:1000",
         "session_config": {"dictionary": [{"word": "Kubernetes"}]}, "contexts": [],
         "learn": {"pasted": "I use whisper flow daily", "edited": "I use Wispr Flow daily"}},
    ]


def main():
    cases = from_vectors() + core_p1()
    names = [c["name"] for c in cases]
    assert len(names) == len(set(names)), "duplicate fixture names"
    if os.path.isdir(OUT):
        shutil.rmtree(OUT)
    os.makedirs(OUT)
    for c in cases:
        with open(os.path.join(OUT, c["name"] + ".json"), "w") as f:
            json.dump(c, f, indent=1, sort_keys=True)
            f.write("\n")
    print(f"wrote {len(cases)} fixtures to {os.path.relpath(OUT, ROOT)}", file=sys.stderr)


if __name__ == "__main__":
    main()
