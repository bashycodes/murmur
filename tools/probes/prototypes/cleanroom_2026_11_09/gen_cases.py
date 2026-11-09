"""Writes cases.json: invented inputs for probe.rs (see its header)."""
import json
import os

C = []


def ap(id, text, ctx=None, cfg=None, raw=None):
    c = {"id": id, "kind": "apply", "text": text, "ctx": ctx or {}, "cfg": cfg or {}}
    if raw is not None:
        c["raw"] = raw
    C.append(c)


def sty(s):
    return {"styles": {"personal": s, "work": s, "email": s, "other": s}}


T = "The parcel arrived early."
# --- cursor context ---
ap("ctx_empty", T)
ap("ctx_before_space", T, {"before": "I think "})
ap("ctx_before_nospace", T, {"before": "I think"})
ap("ctx_before_comma", T, {"before": "Well,"})
ap("ctx_before_comma_sp", T, {"before": "Well, "})
ap("ctx_before_period", T, {"before": "Done."})
ap("ctx_before_period_sp", T, {"before": "Done. "})
ap("ctx_before_q", T, {"before": "Really? "})
ap("ctx_before_newline", T, {"before": "Line one\n"})
ap("ctx_before_colon", T, {"before": "Note: "})
ap("ctx_before_lower_noperiod", "the parcel arrived early.", {"before": "Done. "})
ap("ctx_after_letter", T, {"after": "and then"})
ap("ctx_after_space_letter", T, {"after": " and then"})
ap("ctx_after_upper", T, {"after": "Then we left."})
ap("ctx_after_period", T, {"after": "."})
ap("ctx_after_comma", T, {"after": ", then"})
ap("ctx_after_paren", T, {"after": ")"})
ap("ctx_after_paren_sp", T, {"after": ") more"})
ap("ctx_after_digit", T, {"after": "42"})
ap("ctx_after_space", T, {"after": " "})
ap("ctx_after_newline", T, {"after": "\nNext line"})
ap("ctx_mid_both", T, {"before": "so ", "after": " and"})
ap("ctx_mid_q", "Is it ready?", {"before": "ask ", "after": " now"})
ap("ctx_mid_excl", "Great work!", {"before": "say ", "after": " to all"})
ap("ctx_pron_I", "I left early.", {"before": "and then "})
ap("ctx_pron_Im", "I'm on my way.", {"before": "and "})
ap("ctx_name", "Mara called.", {"before": "and "}, {"user_names": ["Mara"]})
ap("ctx_name_noname", "Mara called.", {"before": "and "})
ap("ctx_screen_word", "Quillon is down.", {"before": "and ", "screen_words": ["Quillon"]})
ap("ctx_dict_word", "Kubernetes failed.", {"before": "and "}, {"dictionary": [{"word": "Kubernetes"}]})
ap("ctx_acronym", "NASA called.", {"before": "and "})
ap("ctx_selection", "New words.", {"before": "Old ", "selected": "bad words", "after": " end"})
ap("ctx_selection_edges", "New words.", {"selected": " bad words "})
ap("ctx_selection_period", "New words.", {"before": "x ", "selected": "bad", "after": ""})
ap("ctx_unknown", T, {"context_known": False})
ap("ctx_unknown_android", T, {"context_known": False}, {"platform": "android"})
ap("ctx_android", T, {"before": "I think "}, {"platform": "android"})
ap("ctx_android_empty", T, {}, {"platform": "android"})
ap("ctx_cjk_before", "世界很好。", {"before": "你好"})
ap("ctx_cjk_end", "Hello 世界。")
ap("ctx_cjk_after", "世界很好。", {"after": "再见"})
ap("ctx_latin_after_cjk", "Hello there.", {"before": "你好"})
ap("ctx_ws_text", "  spaced text.  ")
ap("ctx_multi_sentence_mid", "First one. Second one.", {"before": "so "})
ap("ctx_ellipsis_before", T, {"before": "Wait... "})
ap("ctx_quote_before", T, {"before": "He said \""})
ap("ctx_paren_before", T, {"before": "("})
ap("ctx_dash_before", T, {"before": "so - "})
ap("ctx_number_start", "42 people came.", {"before": "about "})
ap("ctx_after_quote", T, {"after": "\""})
ap("ctx_after_semicolon", T, {"after": "; more"})
ap("ctx_after_q", T, {"after": "?"})
ap("ctx_text_noperiod_after", "the parcel arrived", {"after": " today"})
# --- styles x categories ---
for s in ["default", "formal", "casual", "very_casual", "excited"]:
    for cat in ["personal_chat", "work_chat", "email", "other", "ai_prompt", "terminal", "document"]:
        for t, tag in [("See you soon.", "1s"), ("See you soon. Bring the map.", "2s"),
                       ("See you soon. Bring the map. Call Mara first.", "3s"),
                       ("Are you coming?", "q"), ("That is great!", "x"),
                       ("Hello there, Mara. I hope the trip went well.", "mix")]:
            ap(f"sty_{s}_{cat}_{tag}", t, {"app_category": cat}, sty(s))
ap("sty_default_personal_after", "See you soon.", {"app_category": "personal_chat", "after": " ok"})
ap("sty_casual_personal_before", "See you soon.", {"app_category": "personal_chat", "before": "ok "}, sty("casual"))
ap("sty_vc_personal_I", "I think I will. Ok.", {"app_category": "personal_chat"}, sty("very_casual"))
ap("sty_vc_names", "Mara and NASA agree.", {"app_category": "personal_chat"}, {**sty("very_casual"), "user_names": ["Mara"]})
ap("sty_excited_comma", "We won, finally.", {"app_category": "personal_chat"}, sty("excited"))
ap("sty_formal_personal_noperiod", "see you soon", {"app_category": "personal_chat"}, sty("formal"))
ap("sty_casual_dots", "Well... see you.", {"app_category": "personal_chat"}, sty("casual"))
ap("sty_casual_url", "Visit example.com.", {"app_category": "personal_chat"}, sty("casual"))
ap("sty_default_multiline", "Line one.\nLine two.", {"app_category": "personal_chat"})
ap("sty_default_list", "Plan:\n\n1. Buy milk.\n2. Call Mara.", {"app_category": "work_chat"})
# --- all lowercase ---
ap("lower_basic", "Mara Sent The NASA Report.", {}, {"all_lowercase": True})
ap("lower_mid", "The Report.", {"before": "Done. "}, {"all_lowercase": True})
ap("lower_I", "I think so.", {}, {"all_lowercase": True})


# --- dictionary ---
def D(*e):
    return {"dictionary": list(e)}


K = {"word": "cubernetes", "replacement": "Kubernetes"}
ap("dict_repl", "deploy the cubernetes job.", {}, D(K))
ap("dict_repl_case", "deploy the Cubernetes job.", {}, D(K))
ap("dict_repl_upper", "deploy the CUBERNETES job.", {}, D(K))
ap("dict_repl_start", "cubernetes failed.", {}, D(K))
ap("dict_repl_inside_word", "the cubernetesx job.", {}, D(K))
ap("dict_repl_punct", "is it cubernetes?", {}, D(K))
ap("dict_repl_possessive", "the cubernetes's job.", {}, D(K))
ap("dict_repl_twice", "cubernetes and cubernetes.", {}, D(K))
ap("dict_repl_phrase", "we use zen trix daily.", {}, D({"word": "zen trix", "replacement": "Zentrix"}))
ap("dict_repl_phrase_ws", "we use zen  trix daily.", {}, D({"word": "zen trix", "replacement": "Zentrix"}))
ap("dict_overlap_longest", "we use zen trix hub daily.", {}, D({"word": "zen trix", "replacement": "Zentrix"}, {"word": "zen trix hub", "replacement": "ZentrixHub"}))
ap("dict_overlap_order", "we use zen trix hub daily.", {}, D({"word": "zen trix hub", "replacement": "ZentrixHub"}, {"word": "zen trix", "replacement": "Zentrix"}))
ap("dict_t_guard", "I don't know.", {}, D({"word": "don", "replacement": "Don"}))
ap("dict_t_guard2", "ask don today.", {}, D({"word": "don", "replacement": "Don"}))
ap("dict_apos_word", "it's the dont rule.", {}, D({"word": "dont", "replacement": "DONT"}))
ap("dict_vocab_case", "deploy the kubernetes job.", {}, D({"word": "Kubernetes"}))
ap("dict_vocab_case_upper", "deploy the KUBERNETES job.", {}, D({"word": "Kubernetes"}))
ap("dict_vocab_phrase", "we use zentrix hub.", {}, D({"word": "Zentrix Hub"}))
ap("dict_vocab_lower_entry", "Deploy the Lumo job.", {}, D({"word": "lumo"}))
ap("dict_vocab_camel", "the iphone broke.", {}, D({"word": "iPhone"}))
ap("dict_vocab_start", "kubernetes failed.", {}, D({"word": "Kubernetes"}))
ap("dict_special_chars", "email me at c plus plus.", {}, D({"word": "c plus plus", "replacement": "C++"}))
ap("dict_special_key", "we like c++ a lot.", {}, D({"word": "c++", "replacement": "C plus plus"}))
ap("dict_dot_key", "open node.js now.", {}, D({"word": "node.js", "replacement": "Node.js"}))
ap("dict_unicode", "café time.", {}, D({"word": "café", "replacement": "Café Lumo"}))
ap("dict_unicode_boundary", "the écafé time.", {}, D({"word": "café", "replacement": "X"}))
ap("dict_number_key", "room 42 is free.", {}, D({"word": "42", "replacement": "forty-two"}))
ap("dict_empty_word", "hello there.", {}, D({"word": "", "replacement": "X"}))
ap("dict_ws_word", "hello there.", {}, D({"word": "  hello  ", "replacement": "Hi"}))
ap("dict_repl_dollar", "pay the fee.", {}, D({"word": "fee", "replacement": "$5 fee"}))
ap("dict_repl_backref", "pay the fee.", {}, D({"word": "fee", "replacement": "$1 fee"}))
S = {"word": "my address", "replacement": "12 Elm Road", "isSnippet": True}
ap("dict_snippet_inline", "send my address please.", {}, D(S))
ap("dict_snippet_whole", "My address.", {}, D(S), raw="my address")
ap("dict_snippet_whole_raw_punct", "My address!", {}, D(S), raw="My address!")
ap("dict_snippet_whole_android", "My address.", {}, {**D(S), "platform": "android"}, raw="my address")
ap("dict_snippet_whole_ctx", "My address.", {"before": "so "}, D(S), raw="my address")
ap("dict_snippet_multiline", "My sig.", {}, D({"word": "my sig", "replacement": "Thanks,\nAnn", "isSnippet": True}), raw="my sig")
ap("dict_repl_then_ctx", "cubernetes failed.", {"before": "and "}, D(K))
ap("dict_repl_lower_then_ctx", "Zfoo failed.", {"before": "and "}, D({"word": "zfoo", "replacement": "zbar"}))
for t in ["my address", "My address", "my address.", "  my address  ", "my  address", "My Address!", "say my address", "my addresses", "MY ADDRESS?"]:
    C.append({"id": f"snip_{t}", "kind": "snippet", "text": t, "dict": [S, {"word": "zen", "replacement": "Zen"}]})
C.append({"id": "snip_nonsnippet", "kind": "snippet", "text": "zen", "dict": [{"word": "zen", "replacement": "Zen"}]})
C.append({"id": "snip_two", "kind": "snippet", "text": "sig two", "dict": [{"word": "sig", "replacement": "A", "isSnippet": True}, {"word": "sig two", "replacement": "B", "isSnippet": True}]})
# --- press enter ---
PE = {"press_enter_enabled": True}
for t in ["Send it press enter.", "Send it. Press enter.", "Send it, press enter", "press enter", "Press Enter.", "Press enter now.", "Send it press enter!", "Send it press the enter.", "Send it hit enter.", "Send it press return."]:
    ap(f"enter_{t}", t, {}, PE)
ap("enter_disabled", "Send it press enter.", {}, {})
ap("enter_android", "Send it press enter.", {}, {**PE, "platform": "android"})
ap("enter_raw_only", "Send it.", {}, PE, raw="send it press enter")
ap("enter_ctx", "Send it press enter.", {"before": "ok "}, PE)
ap("enter_personal", "Send it press enter.", {"app_category": "personal_chat"}, PE)
# --- IDE tagging ---
for app in ["Cursor.exe", "cursor.exe", "Windsurf.exe", "Code.exe"]:
    ap(f"ide_{app}", "Open main.rs and fix it.", {"app_id": app, "file_names": ["main.rs", "lib.rs"]})
F = {"app_id": "Cursor.exe", "file_names": ["main.rs", "lib.rs", "README.md", "user_service.py"]}
for t in ["Open main.rs and fix it.", "Look at main dot rs.", "Check lib.rs, then main.rs.", "Read readme.md first.", "Edit user service dot py.", "Edit user_service.py now.", "Edit main.", "Open main.rsx too.", "Open @main.rs please.", "Open main.rs", "main.rs is broken."]:
    ap(f"ide_{t}", t, F)
ap("ide_android", "Open main.rs and fix it.", F, {"platform": "android"})
ap("ide_nofiles", "Open main.rs and fix it.", {"app_id": "Cursor.exe"})
# --- signature ---
SIG = {"email_signature": "— Ann"}
LONG = "Hi Tom, the report is attached and ready for your review today."
for app in ["outlook.exe", "olk.exe", "thunderbird.exe", "OUTLOOK.EXE", "chrome.exe"]:
    ap(f"sig_app_{app}", LONG, {"app_id": app}, SIG)
ap("sig_gmail", LONG, {"app_id": "chrome.exe", "url_host": "mail.google.com"}, SIG)
E = {"app_id": "outlook.exe"}
ap("sig_greeting_short", "Hi Tom, thanks.", E, SIG)
ap("sig_dear", "Dear Tom, thanks.", E, SIG)
ap("sig_hey", "Hey! Thanks.", E, SIG)
ap("sig_short", "Thanks for that.", E, SIG)
ap("sig_ten", "one two three four five six seven eight nine ten.", E, SIG)
ap("sig_nine", "one two three four five six seven eight nine.", E, SIG)
ap("sig_ten_numbers", "one two three four five 6 7 8 9 10.", E, SIG)
ap("sig_already_before", "Hi Tom, thanks.", {**E, "before": "Body\n— Ann"}, SIG)
ap("sig_already_after", "Hi Tom, thanks.", {**E, "after": "\n\nann"}, SIG)
ap("sig_in_text", "Hi Tom, thanks. Ann", E, SIG)
ap("sig_nbsp", "Hi Tom, thanks.", {**E, "after": "— Ann"}, {"email_signature": "— Ann"})
ap("sig_plain", "Hi Tom, thanks.", E, {"email_signature": "Ann"})
ap("sig_dash", "Hi Tom, thanks.", E, {"email_signature": "-- Ann"})
ap("sig_multiline", "Hi Tom, thanks.", E, {"email_signature": "Ann\nLumo Ltd"})
ap("sig_unknown_ctx", "Hi Tom, thanks.", {**E, "context_known": False}, SIG)
ap("sig_android", "Hi Tom, thanks.", {"app_id": "com.google.android.gm"}, {**SIG, "platform": "android"})
ap("sig_cat_email_only", "Hi Tom, thanks.", {"app_category": "email"}, SIG)
ap("sig_empty", "Hi Tom, thanks.", E, {"email_signature": ""})
ap("sig_with_before", "Hi Tom, thanks.", {**E, "before": "Draft: "}, SIG)
ap("sig_with_after", "Hi Tom, thanks.", {**E, "after": "Old text"}, SIG)
# --- app skip lists via context steps ---
for app, url in [("Notion.exe", ""), ("notion.exe", ""), ("chrome.exe", "www.notion.so"), ("chrome.exe", "notion.so"), ("Flow Clone.exe", ""), ("flow-windows.exe", ""), ("dev.flowclone", ""), ("Dictate Pro.exe", "")]:
    ap(f"skip_{app}_{url}", T, {"app_id": app, "url_host": url, "before": "so "})
# --- stacked messages / chat apps by id ---
for app in ["WhatsApp.exe", "Telegram.exe", "slack.exe", "Discord.exe", "ms-teams.exe", "com.whatsapp", "org.telegram.messenger"]:
    ap(f"chat_{app}", "See you soon. Bring the map. Call Mara first.", {"app_id": app})
    ap(f"chat1_{app}", "See you soon.", {"app_id": app})
ap("stack_vc_personal_id", "See you soon. Bring the map. Call Mara first.", {"app_id": "WhatsApp.exe"}, sty("very_casual"))
ap("stack_casual_personal_cat", "See you soon. Bring the map. Call Mara first.", {"app_category": "personal_chat"}, sty("casual"))
# --- terminals ---
for app in ["WindowsTerminal.exe", "cmd.exe", "powershell.exe", "com.termux"]:
    ap(f"term_{app}", "List the files.", {"app_id": app, "before": "$ "})
# --- categories ---
for app, url, plat in [
    ("slack.exe", "", "windows"), ("Slack.exe", "", "windows"), ("outlook.exe", "", "windows"), ("olk.exe", "", "windows"),
    ("thunderbird.exe", "", "windows"), ("WhatsApp.exe", "", "windows"), ("Telegram.exe", "", "windows"), ("Discord.exe", "", "windows"),
    ("ms-teams.exe", "", "windows"), ("Teams.exe", "", "windows"), ("signal.exe", "", "windows"), ("chrome.exe", "", "windows"),
    ("chrome.exe", "mail.google.com", "windows"), ("chrome.exe", "outlook.live.com", "windows"), ("chrome.exe", "app.slack.com", "windows"),
    ("chrome.exe", "web.whatsapp.com", "windows"), ("chrome.exe", "chatgpt.com", "windows"), ("chrome.exe", "claude.ai", "windows"),
    ("chrome.exe", "docs.google.com", "windows"), ("chrome.exe", "github.com", "windows"), ("msedge.exe", "teams.microsoft.com", "windows"),
    ("WindowsTerminal.exe", "", "windows"), ("cmd.exe", "", "windows"), ("powershell.exe", "", "windows"), ("pwsh.exe", "", "windows"),
    ("Code.exe", "", "windows"), ("Cursor.exe", "", "windows"), ("WINWORD.EXE", "", "windows"), ("winword.exe", "", "windows"),
    ("Notion.exe", "", "windows"), ("ChatGPT.exe", "", "windows"), ("Claude.exe", "", "windows"), ("notepad.exe", "", "windows"),
    ("com.whatsapp", "", "android"), ("com.slack", "", "android"), ("com.google.android.gm", "", "android"),
    ("com.microsoft.office.outlook", "", "android"), ("org.telegram.messenger", "", "android"), ("com.discord", "", "android"),
    ("com.termux", "", "android"), ("com.openai.chatgpt", "", "android"), ("com.anthropic.claude", "", "android"),
    ("com.android.chrome", "mail.google.com", "android"), ("com.android.chrome", "", "android"),
    ("com.google.android.apps.docs.editors.docs", "", "android"), ("com.samsung.android.messaging", "", "android"),
    ("com.google.android.apps.messaging", "", "android"), ("org.thoughtcrime.securesms", "", "android"), ("com.facebook.orca", "", "android"),
    ("com.microsoft.teams", "", "android"), ("com.linkedin.android", "", "android"), ("com.instagram.android", "", "android"),
    ("com.whatsapp", "", "windows"), ("slack.exe", "", "android"), ("", "mail.google.com", "android"), ("", "", "windows")]:
    C.append({"id": f"cat_{plat}_{app}_{url}", "kind": "category", "platform": plat, "ctx": {"app_id": app, "url_host": url}})
C.append({"id": "cat_shell_wins", "kind": "category", "platform": "windows", "ctx": {"app_id": "outlook.exe", "app_category": "work_chat"}})


# --- learn ---
def L(id, p, e, d=None):
    C.append({"id": id, "kind": "learn", "pasted": p, "edited": e, "dict": d or []})


L("learn_basic", "ping the zentrix team", "ping the Zentrix team")
L("learn_two_runs", "ask gina about lumo", "ask Jenna about Lumio")
L("learn_insert", "call me", "call me now")
L("learn_delete", "call me now", "call me")
L("learn_five", "a b c d e", "v w x y z")
L("learn_punct_kept", "meet at lumo.", "meet at Lumio.")
L("learn_case_only_initial", "zentrix is down", "Zentrix is down")
L("learn_case_only_mid", "the zentrix is down", "the Zentrix is down")
L("learn_stop_mixed", "go to the lumo", "go to a Lumio")
L("learn_dict_repl_target", "ping the zentrics team", "ping the Zentrix team", [{"word": "zentrics", "replacement": "Zentrix"}])
L("learn_dict_word_case", "ping the zentrix team", "ping the Zentrix team", [{"word": "zentrix"}])
L("learn_numbers", "room 42", "room 43")
L("learn_contraction_edit", "it is lumo", "it's Lumio")
L("learn_empty", "", "something")
L("learn_whitespace", "a  b", "a b")
L("learn_dup", "lumo and lumo", "Lumio and Lumio")
L("learn_after_typing", "see you at lumo", "see you at Lumio tomorrow then bye")
L("learn_unicode", "meet at cafe", "meet at Café")
L("learn_adjacent_words", "the lumo nova kit", "the Lumio Novva kit")
L("learn_mixed_run", "the big lumo box", "the small Lumio box")

# --- round 2: chat apps by id x style x platform, more cursor context ---
texts = {"3s": "See you soon. Bring the map. Call Mara first.", "2s": "See you soon. Bring the map.",
         "q": "Are you there? Call me.", "nl": "See you soon.\nBring the map.",
         "long": "I got the tickets for tonight and they were cheaper than I thought. We should leave by six so we are not late. Bring the map."}
for s in ["default", "casual", "very_casual", "excited", "formal"]:
    for app, plat in [("WhatsApp.exe", "windows"), ("com.whatsapp", "android"), ("slack.exe", "windows"), ("com.slack", "android")]:
        for k, t in texts.items():
            ap(f"st_{s}_{app}_{k}", t, {"app_id": app}, {**sty(s), "platform": plat})
for app, plat in [("com.whatsapp", "android"), ("com.google.android.gm", "android"), ("com.slack.Slack", "windows"), ("com.tinyspeck.slackmacgap", "windows")]:
    ap(f"pl_{app}", "See you soon.", {"app_id": app, "before": "ok "}, {"platform": plat})
    ap(f"pl_at_{app}", "Ask at Mara about it.", {"app_id": app, "screen_words": ["Mara"]}, {"platform": plat})
ap("ctx_bad_sel", "New.", {"selected": "x", "before": "Hello.", "after": "World"})
ap("ctx_after_lower_period", "Done.", {"after": "and more"})
ap("ctx_after_lower_q", "Done?", {"after": "and more"})
ap("ctx_after_upper_ws", "Done.", {"after": " Next"})
ap("ctx_after_cjk_punct", "好的。", {"after": "x"})
ap("ctx_before_digit", "items.", {"before": "42"})
ap("ctx_before_open_quote", "Items.", {"before": "“"})
ap("ctx_before_tab", "Items.", {"before": "a\t"})
ap("ctx_before_nbsp", "Items.", {"before": "a "})
ap("ctx_before_ellipsis_nospace", "Items.", {"before": "wait..."})
ap("ctx_before_excl_nospace", "Items.", {"before": "wow!"})
ap("ctx_before_hyphen", "Items.", {"before": "well-"})
ap("ctx_before_slash", "Items.", {"before": "a/"})
ap("ctx_before_at", "Items.", {"before": "mail@"})
ap("ctx_mid_two", "Items. More.", {"before": "so ", "after": " and"})
ap("ctx_mid_both_comma", "Items, more.", {"before": "so ", "after": " and"})
ap("ctx_text_trailing_space", "Items. ")
ap("ctx_text_empty", "")
ap("ctx_text_only_space", "  ", {"before": "x "})
ap("ctx_upper_word_mid", "The NASA plan.", {"before": "so "})
ap("ctx_mixed_iphone", "IPhone works.", {"before": "so "})
ap("ctx_mixed_iphone2", "iPhone works.", {"before": "so "})
ap("ctx_casual_after", "See you.", {"app_category": "personal_chat", "after": "Bye"}, sty("casual"))
ap("ctx_name_multi", "Mara Quill called.", {"before": "and "}, {"user_names": ["Mara Quill"]})
ap("ctx_name_second", "Quill called.", {"before": "and "}, {"user_names": ["Mara Quill"]})
ap("ctx_dict_repl_target", "Kubernetes failed.", {"before": "and "}, {"dictionary": [{"word": "cube", "replacement": "Kubernetes"}]})
ap("ctx_screen_case", "quillon is down.", {"before": "and ", "screen_words": ["Quillon"]})
ap("ctx_I_contraction_curly", "I’ll go.", {"before": "and "})
ap("ctx_quote_start", "\"Hello\" she said.", {"before": "and "})
ap("ctx_after_close_quote_sp", "Done.", {"after": "\" said"})
ap("ctx_before_lower_word_period_in_text", "Done. Next step.", {"before": "ok "})
ap("ctx_after_lower_multi", "First. Second.", {"after": " and"})
ap("ctx_selection_cap", "new words.", {"before": "Done. ", "selected": "x"})
ap("ctx_selection_cap2", "New words.", {"before": "so ", "selected": "x", "after": "."})
for sel in ["Bad words.", "Bad words", "bad words.", "BAD", "x", "Bad words?", " Bad words. "]:
    ap(f"sel_{sel}", "New words.", {"selected": sel})
    ap(f"sel_mid_{sel}", "New words.", {"before": "Hello. ", "selected": sel, "after": " Then"})
ap("sel_two_sentences", "New words. More here.", {"selected": "bad"})
ap("sel_question", "Is it new?", {"selected": "bad"})
ap("sel_casual", "New words.", {"selected": "Bad words.", "app_category": "personal_chat"}, sty("casual"))
ap("enter_off", "Send it press enter.", {}, {"press_enter_enabled": False})
ap("enter_off_raw", "Send it.", {}, {"press_enter_enabled": False}, raw="send it press enter")
ap("enter_casual", "Send it press enter.", {"app_category": "personal_chat"}, {**PE, **sty("casual")})
ap("enter_after", "Send it press enter.", {"after": " more"}, PE)
ap("enter_mid_nl", "Send it.\nPress enter.", {}, PE)
ap("lower_snippet", "My address.", {}, {**D(S), "all_lowercase": True}, raw="my address")
ap("lower_dict", "deploy the cubernetes job.", {}, {**D(K), "all_lowercase": True})
ap("lower_casual", "See You Soon.", {"app_category": "personal_chat"}, {**sty("casual"), "all_lowercase": True})
for app in ["olk.exe", "WINWORD.EXE"]:
    ap(f"sig2_{app}", LONG, {"app_id": app}, SIG)

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cases.json")
with open(out, "w") as f:
    json.dump(C, f, ensure_ascii=False, indent=0)
print(len(C))
