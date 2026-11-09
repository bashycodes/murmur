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
for app, url in [("Notion.exe", ""), ("notion.exe", ""), ("chrome.exe", "www.notion.so"), ("chrome.exe", "notion.so"), ("Murmur.exe", ""), ("murmur-windows.exe", ""), ("dev.murmur", ""), ("Dictate Pro.exe", "")]:
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

# --- round 3 (review of the rewrite): contexts the first rounds missed ---
WA = {"app_id": "WhatsApp.exe"}
AWA = {"app_id": "com.whatsapp"}
AND = {"platform": "android"}
SEE = "See you soon."
# 1. chat period vs. sentence ends around the cursor
for n, c in [("b_ok", {"before": "Ok. "}), ("b_ok_nosp", {"before": "Ok."}), ("b_lower", {"before": "ok "}),
             ("b_prev_line", {"before": "Done.\n"}), ("b_prev_line_word", {"before": "Done.\nso "}),
             ("b_q", {"before": "what? so "}), ("b_bang", {"before": "Wow! "}), ("b_ell", {"before": "hmm... "}),
             ("a_ok", {"after": " Ok."}), ("a_word", {"after": " later"}), ("a_next_line", {"after": "\nDone."}),
             ("a_line_end", {"after": " then.\nmore"}), ("sel_period", {"selected": "Fine."}), ("sel_plain", {"selected": "fine"}),
             ("b_comma", {"before": "Sure, "}), ("b_mid_sent", {"before": "Hi. I think "})]:
    ap(f"r3_chat_{n}", SEE, {**WA, **c})
    ap(f"r3_achat_{n}", SEE, {**AWA, **c}, AND)
# 2. casual / very casual sentence limit
for k in [9, 10, 11, 14]:
    t = " ".join(f"Line {i} here." for i in range(k))
    ap(f"r3_casual_{k}", t, {}, sty("casual"))
    ap(f"r3_vcasual_{k}", t, {}, sty("very_casual"))
    ap(f"r3_default_chat_{k}", t, WA)
# 3. Notion skip conditions
for host_app, nm in [({"app_id": "Notion.exe"}, "app"), ({"app_id": "chrome.exe", "url_host": "notion.so"}, "host"),
                     ({"app_id": "chrome.exe", "url_host": "www.notion.so"}, "www"), ({"app_id": "chrome.exe", "url_host": "x.notion.site"}, "site"),
                     ({"app_id": "murmur-windows.exe"}, "own")]:
    for n, c in [("empty", {}), ("b1", {"before": "so "}), ("b2", {"before": "so then "}), ("b3", {"before": "one two three "}),
                 ("b5", {"before": "we went to the shop "}), ("bell", {"before": "we went to the shop..."}),
                 ("b1_after", {"before": "so ", "after": "x"}), ("b5_after", {"before": "we went to the shop ", "after": " more"}),
                 ("b2_nosp", {"before": "so then"}), ("bell_sp", {"before": "we went to the shop... "})]:
        ap(f"r3_notion_{nm}_{n}", T, {**host_app, **c})
# 4. sentence count with newlines
ap("r3_nl_list3", "Milk.\nEggs.\nBread.", WA)
ap("r3_nl_list3_nop", "Milk\nEggs\nBread.", WA)
ap("r3_nl_two_lines_3s", "One. Two.\nThree.", WA)
ap("r3_nl_blank", "Milk.\n\nEggs.\n\nBread.", WA)
ap("r3_nl_casual_list", "\n".join(f"Item {i}." for i in range(12)), {}, sty("casual"))
# 5. cursor characters
for n, b in [("paren_open", "see ("), ("paren_open_sp", "see ( "), ("paren_close", "done) "), ("paren_close_nosp", "done)"),
             ("emdash", "wait — "), ("emdash_nosp", "wait —"), ("endash", "wait – "), ("emoji", "great 😀 "), ("emoji_nosp", "great 😀"),
             ("rdquo", "he said” "), ("rsquo", "the ’ "), ("dquote_close", "\"ok\" "), ("squote", "'ok' "),
             ("a_dash", "a -"), ("a_dash_sp", "a - "), ("dollar", "$"), ("dollar_sp", "$ "), ("hash", "#"), ("hash_sp", "# "),
             ("under", "_"), ("star", "*"), ("star_sp", "* "), ("eq", "="), ("eq_sp", "= "), ("tilde", "~"), ("tilde_sp", "~ "),
             ("bracket_close", "done] "), ("brace_close", "done} "), ("pct", "50% "), ("amp", "Tom & "), ("plus", "1 + "),
             ("lt", "a < "), ("gt", "a > "), ("pipe", "a | "), ("backslash", "a\\"), ("caret", "^ "), ("bullet", "• "),
             ("guill_close", "dit» "), ("ellipsis_char", "wait… "), ("colon_nosp", "Note:"), ("semi", "ok;"), ("digit", "room 4"),
             ("hebrew", "שלום "), ("cyr", "привет ")]:
    ap(f"r3_cur_{n}", T, {"before": b})
for n, a in [("emdash_and", "— and"), ("emdash_and_sp", " — and"), ("endash_and", "– and"), ("hyphen_and", "- and"),
             ("hyphen_and_sp", " - and"), ("emdash_alone", "—"), ("paren_close", ")"), ("rdquo", "”"), ("colon", ": x"),
             ("upper", "Then"), ("digit", "42"), ("emoji", "😀"), ("ellipsis", "…"), ("dollar", "$5")]:
    ap(f"r3_after_{n}", T, {"after": a})
# 6. snippet + signature, empty cleaned text
SN = {"word": "sign off", "replacement": "Thanks for the update, talk soon.", "isSnippet": True}
SN2 = {"word": "my address", "replacement": "Hi Tom, my address is 12 Elm Road.", "isSnippet": True}
for app in ["outlook.exe", "chrome.exe"]:
    ctx = {"app_id": app, "url_host": "mail.google.com" if app == "chrome.exe" else ""}
    ap(f"r3_snip_sig_{app}", "Sign off.", ctx, {**D(SN), **SIG}, raw="sign off")
    ap(f"r3_snip_sig_greet_{app}", "My address.", ctx, {**D(SN2), **SIG}, raw="my address")
    ap(f"r3_snip_nosig_{app}", "My address.", ctx, D(SN2), raw="my address")
ap("r3_sig_empty_text", "", {"app_id": "outlook.exe"}, SIG, raw="hi tom")
ap("r3_sig_space_text", "  ", {"app_id": "outlook.exe"}, SIG, raw="hi tom")
ap("r3_sig_empty_raw", LONG, {"app_id": "outlook.exe"}, SIG, raw="")
ap("r3_snip_android", "My address.", {}, {**D(SN2), "platform": "android"}, raw="my address")
ap("r3_snip_android_sig", "My address.", {"app_id": "com.google.android.gm"}, {**D(SN2), **SIG, "platform": "android"}, raw="my address")
# 7. learn
L("r3_learn_long_corr", "go lumo now", "go one two three four five six now")
L("r3_learn_corr5", "go lumo now", "go a b c d e now")
L("r3_learn_corr4", "go lumo now", "go Ab Cd Ef Gh now")
L("r3_learn_corr5_names", "go lumo now", "go Ab Cd Ef Gh Ij now")
for w in ["um", "uh", "like", "okay", "ok", "yes", "no", "yeah", "just", "very", "really", "there", "here", "so", "well", "oh",
          "hmm", "right", "actually", "basically", "literally", "then", "now", "also", "too", "not", "the", "a", "an", "and",
          "or", "but", "to", "of", "in", "on", "at", "for", "with", "is", "it", "this", "that", "i", "you", "we", "they", "he",
          "she", "my", "your", "be", "have", "do", "was", "are", "will", "can", "would", "should", "if", "what", "who",
          "how", "when", "where", "why", "all", "some", "any", "one", "two", "more", "much", "many", "up", "out", "about",
          "hi", "hello", "thanks", "please", "sure", "maybe", "kind", "sort", "mean", "know", "guess", "anyway", "stuff",
          "thing", "things", "gonna", "wanna", "lot", "bit", "pretty", "quite", "even", "still", "only", "again", "back"]:
    L(f"r3_learn_stop_orig_{w}", f"we {w} fine", "we Zentra fine")
    L(f"r3_learn_stop_corr_{w}", "we zentra fine", f"we {w} fine")
for n, p_, e_ in [("dont", "i dont go", "i don't go"), ("dont_rev", "i don't go", "i dont go"), ("its", "its lumo", "it's Lumio"),
                  ("its_rev", "it's lumo here", "its Lumio here"), ("orig_contr", "we can't zentra", "we cannot Zentra"),
                  ("orig_contr2", "we'll go", "Lumio go"), ("orig_contr3", "so we're lumo", "so we Lumio"),
                  ("orig_poss", "the lumo's car", "the Lumio's car"), ("orig_poss2", "the lumo's car", "the Lumio car")]:
    L(f"r3_learn_contr_{n}", p_, e_)
for n, p_, e_ in [("start", "lumo is here", "Lumo is here"), ("after_period", "ok. lumo is here", "ok. Lumo is here"),
                  ("after_q", "ok? lumo is here", "ok? Lumo is here"), ("mid", "ok lumo is here", "ok Lumo is here"),
                  ("start_two", "lumo hub is here", "Lumo Hub is here"), ("start_diff", "lumo is here", "Lumio is here"),
                  ("lower_start", "Lumo is here", "lumo is here"), ("lower_mid", "ok Lumo is here", "ok lumo is here")]:
    L(f"r3_learn_case_{n}", p_, e_)
RK = [{"word": "lumo", "replacement": "Lumio"}]
L("r3_learn_key_orig", "see lumo now", "see Lumia now", RK)
L("r3_learn_key_corr", "see lumi now", "see Lumio now", RK)
L("r3_learn_word_corr", "see lumi now", "see Lumio now", [{"word": "Lumio"}])
L("r3_learn_word_orig", "see Lumio now", "see Lumia now", [{"word": "Lumio"}])
# 8. acronyms and mixed case mid-sentence
for w in ["NASA called.", "IPhone works.", "IPad works.", "McKay called.", "USA's team won.", "OK then.", "A plan works.",
          "AI works.", "I.B.M. works.", "EBay works.", "JavaScript works.", "MacBook works.", "PhD done.", "X is fine.",
          "NASA's plan works.", "U.S. troops.", "CEO called.", "TV is on."]:
    ap(f"r3_case_{w}", w, {"before": "and "})

# --- round 3b: settle the cursor-character and stopword rules ---
for i, b in enumerate(["Done. (", "! (", "42 (", "( (", "x(", "Done. 😀 ", "😀 ", "😀", "Done.) ", ") ", ")", "— ", "—", "Done. — ",
          "\"", "x\"", "x\" ", "\" ", "x' ", "' ", "x’ ", "x’", "“x” ", "«x» ", "x]", "x}", "x'", "wait—", "wait– ", "a –", "a —",
          "x -", "x - ", "x —", "50%", "Tom &", "1 +", "a <", "a >", "a |", "^", "•", "dit»", "wait…", "great 🎉 ", "great ❤️ ",
          "great 👍🏽 ", "great 🎉", "x ) ", "x ( ", "Done.\"", "Done.\" ", "Done.’ ", "Done.” ", "Done.)", "Done!) ",
          "so, (", "so: (", "so — (", "x)]", "x;) ", "a ‘", "a ’", "a “", "a ”", "a \"", "a '", "x»", "x« ", "x‘ ", "x“ ",
          "x [", "x {", "x <", "x ¿", "x ¡", "x ¿ ", "a/ ", "a@ ", "a@", "a- ", "a-", "a—", "a— ", "a–", "a– "]):
    ap(f"r3b_cur_{i}", T, {"before": b})
    ap(f"r3b_cur_pron_{i}", "I left.", {"before": b})
for i, a in enumerate(["-and", "—and", "- And", "— And", "-", " -", "-5", "— 5", "– and", " – and", "—  and", "-- and", "— (and)",
          "— ,", "…and", "... and", "(and", ") and", "\" and", "” and", "’ and", "😀 and", "x", "X", "5", "_x", "#x", "$x",
          "@x", "/x", "&x", "*x", "'x", "‘x", "“x", "«x", "[x", "\tand", " and"]):
    ap(f"r3b_after_{i}", T, {"after": a})
for w in ["um", "uh", "er", "erm", "ah", "eh", "hm", "mm", "huh", "yep", "yup", "nope", "nah", "yea", "alright", "k", "okey",
          "me", "us", "him", "her", "them", "his", "hers", "its", "our", "ours", "their", "theirs", "mine", "yours", "myself",
          "these", "those", "who", "whom", "whose", "which", "what", "where", "there", "here", "then", "than", "as", "by",
          "from", "into", "onto", "upon", "over", "under", "off", "down", "via", "per", "nor", "yet", "because", "since",
          "while", "though", "although", "unless", "until", "till", "am", "were", "been", "being", "has", "had", "having",
          "does", "did", "doing", "done", "could", "may", "might", "must", "shall", "ought", "got", "get", "go", "went",
          "say", "said", "see", "let", "make", "take", "way", "very", "too", "so", "such", "just", "only", "own", "same",
          "other", "each", "every", "both", "few", "most", "no", "not", "yes", "ok", "okay", "like", "oh", "well", "now",
          "how", "why", "when", "all", "any", "some", "more", "less", "s", "t", "d", "ll", "re", "ve", "m", "o", "y",
          "x", "b", "c", "e", "f", "g", "h", "j", "l", "n", "p", "q", "r", "u", "v", "w", "z", "a", "i", "an", "the"]:
    L(f"r3b_stop_{w}", "we zentra fine", f"we {w} fine")
    L(f"r3b_stopcap_{w}", "we zentra fine", f"we {w.capitalize()} fine")
L("r3b_stop_two", "we zentra fine", "we the um fine")
L("r3b_stop_mixed", "we zentra fine", "we the Lumio fine")
L("r3b_stop_orig_two", "we the um fine", "we Lumio fine")

# --- round 3c: style vs. context, remaining characters ---
for st in ["casual", "very_casual", "excited", "formal"]:
    for n, c in [("b_ok", {"before": "Ok. "}), ("a_ok", {"after": " Ok."}), ("sel", {"selected": "Fine."}), ("none", {"before": "ok "})]:
        ap(f"r3c_{st}_wa_{n}", SEE, {**WA, **c}, sty(st))
        ap(f"r3c_{st}_other_{n}", SEE, c, sty(st))
for k in [10, 11]:
    ap(f"r3c_casual_nl_{k}", "\n".join(f"Item {i}" for i in range(k - 1)) + "\nLast one.", {}, sty("casual"))
    ap(f"r3c_casual_q_{k}", " ".join(f"Is {i} ok?" for i in range(k - 1)) + " Last one.", {}, sty("casual"))
ap("r3c_chat_q3", "Is it? Are you? Fine.", WA)
ap("r3c_chat_ell", "Wait... then go.", WA)
ap("r3c_chat_b_unknown", SEE, {**AWA, "before": "Ok. ", "context_known": False}, AND)
for i, b in enumerate(["- ", "-", "Done. - ", ", ", ",", ": ", "; ", "/ ", "Done. ( ", "x ]", "x } ", "x ", "x (", "x ) )",
                       "Done.\n(", "x\n) ", "x ’ ", "x” ", "x”", "😀😀 ", "x 😀😀", "x 🇮🇱 ", "x ✅ ", "x ★ ", "x © ", "x ° ", "x ™ "]):
    ap(f"r3c_cur_{i}", T, {"before": b})
for i, a in enumerate(["{x", "]x", "}x", " (and", "( and", "-(and", "—", "— ", "-  and", " -and", " — and", "—\tand", "-\nand"]):
    ap(f"r3c_after_{i}", T, {"after": a})
L("r3c_learn_curly", "i dont go", "i don’t go")
L("r3c_learn_curly_orig", "the lumo’s car", "the Lumio car")
L("r3c_learn_start_three", "lumo hub one is here", "Lumo Hub One is here")
L("r3c_learn_after_nl", "ok\nlumo is here", "ok\nLumo is here")
L("r3c_learn_after_excl", "ok! lumo is here", "ok! Lumo is here")
L("r3c_learn_after_colon", "ok: lumo is here", "ok: Lumo is here")
L("r3c_learn_orig5", "go a b c d e now", "go Lumio now")
L("r3c_learn_orig4", "go a b c d now", "go Lumio now")

# --- round 3d: unknown context and the chat rule ---
for plat, app in [("android", "com.whatsapp"), ("windows", "WhatsApp.exe")]:
    for n, c in [("empty", {}), ("ok", {"before": "Ok. "}), ("lower", {"before": "ok "}), ("after", {"after": " Ok."})]:
        ap(f"r3d_unk_{plat}_{n}", SEE, {"app_id": app, "context_known": False, **c}, {"platform": plat})
        ap(f"r3d_unkcas_{plat}_{n}", SEE, {"app_id": app, "context_known": False, **c}, {"platform": plat, **sty("casual")})
for n, b in [("uell", "wait… "), ("nbsp", "Ok. "), ("cjk", "好。"), ("tab", "Ok.\t"), ("cr", "Ok.\r\n"), ("colon", "Note: ")]:
    ap(f"r3d_chat_{n}", SEE, {**WA, "before": b})

# --- round 3e: which apps count as chat / email for the style and signature steps ---
for n, ctx, cfg in [("host_wa", {"app_id": "chrome.exe", "url_host": "web.whatsapp.com"}, {}),
                    ("host_slack", {"app_id": "chrome.exe", "url_host": "app.slack.com"}, {}),
                    ("host_discord", {"app_id": "chrome.exe", "url_host": "discord.com"}, {}),
                    ("signal", {"app_id": "signal.exe"}, {}), ("teams_w", {"app_id": "ms-teams.exe"}, {}),
                    ("a_teams", {"app_id": "com.microsoft.teams"}, AND), ("a_discord", {"app_id": "com.discord"}, AND),
                    ("a_signal", {"app_id": "org.thoughtcrime.securesms"}, AND), ("a_msgs", {"app_id": "com.google.android.apps.messaging"}, AND),
                    ("a_insta", {"app_id": "com.instagram.android"}, AND), ("a_orca", {"app_id": "com.facebook.orca"}, AND),
                    ("a_host_wa", {"app_id": "com.android.chrome", "url_host": "web.whatsapp.com"}, AND),
                    ("wa_cat_other", {"app_id": "WhatsApp.exe", "app_category": "other"}, {}),
                    ("wa_cat_email", {"app_id": "WhatsApp.exe", "app_category": "email"}, {}),
                    ("np_cat_pc", {"app_id": "notepad.exe", "app_category": "personal_chat"}, {})]:
    ap(f"r3e_chat_{n}", SEE, ctx, cfg)
for n, ctx in [("outlook_live", {"app_id": "chrome.exe", "url_host": "outlook.live.com"}),
               ("outlook_cat_wc", {"app_id": "outlook.exe", "app_category": "work_chat"}),
               ("np_cat_email", {"app_id": "notepad.exe", "app_category": "email"}),
               ("hxoutlook", {"app_id": "hxoutlook.exe"}), ("mailbird", {"app_id": "mailbird.exe"})]:
    ap(f"r3e_sig_{n}", "Hi Tom, thanks.", ctx, SIG)
WIN_IDS = ["outlook", "olk", "thunderbird", "betterbird", "mailbird", "mailspring", "hxoutlook", "slack", "ms-teams",
           "teams", "zoom", "mattermost", "whatsapp", "whatsapp.root", "telegram", "discord", "signal", "messenger", "viber",
           "line", "skype", "element", "chatgpt", "claude", "perplexity", "cursor", "windsurf", "code", "windowsterminal",
           "wt", "cmd", "powershell", "pwsh", "conhost", "alacritty", "wezterm-gui", "mintty", "putty", "winword", "notion",
           "obsidian", "onenote", "soffice", "swriter"]
AND_IDS = ["com.google.android.gm", "com.microsoft.office.outlook", "ch.protonmail.android",
           "com.samsung.android.email.provider", "com.fsck.k9", "net.thunderbird.android",
           "com.yahoo.mobile.client.android.mail", "com.slack", "com.microsoft.teams", "us.zoom.videomeetings",
           "com.mattermost.rn", "com.whatsapp", "com.whatsapp.w4b", "org.telegram.messenger", "org.thoughtcrime.securesms",
           "com.discord", "com.facebook.orca", "com.instagram.android", "com.viber.voip", "jp.naver.line.android",
           "com.google.android.apps.messaging", "com.samsung.android.messaging", "im.vector.app", "com.openai.chatgpt",
           "com.anthropic.claude", "ai.perplexity.app.android", "com.google.android.apps.bard", "com.termux",
           "org.connectbot", "com.server.auditor.ssh.client"]
HOSTS = ["mail.google.com", "outlook.live.com", "outlook.office.com", "outlook.office365.com", "mail.proton.me",
         "mail.yahoo.com", "app.slack.com", "teams.microsoft.com", "teams.live.com", "web.whatsapp.com",
         "web.telegram.org", "discord.com", "messenger.com", "chatgpt.com", "chat.openai.com", "claude.ai",
         "gemini.google.com", "perplexity.ai", "x.slack.com", "www.messenger.com"]
for i in WIN_IDS:
    C.append({"id": f"r3e_cat_w_{i}", "kind": "category", "platform": "windows", "ctx": {"app_id": i + ".exe", "url_host": ""}})
    ap(f"r3e_chatw_{i}", SEE, {"app_id": i + ".exe"})
for i in AND_IDS:
    C.append({"id": f"r3e_cat_a_{i}", "kind": "category", "platform": "android", "ctx": {"app_id": i, "url_host": ""}})
    ap(f"r3e_chata_{i}", SEE, {"app_id": i}, AND)
for h in HOSTS:
    for plat, app in [("windows", "chrome.exe"), ("android", "com.android.chrome")]:
        C.append({"id": f"r3e_cat_{plat[0]}h_{h}", "kind": "category", "platform": plat, "ctx": {"app_id": app, "url_host": h}})
    ap(f"r3e_chath_{h}", SEE, {"app_id": "chrome.exe", "url_host": h})
    ap(f"r3e_sigh_{h}", "Hi Tom, thanks.", {"app_id": "chrome.exe", "url_host": h}, SIG)
# --- round 4: shared app groups (A3): own-app and Notion skips, new members ---
for app, plat in [("murmur-windows.exe", "windows"), ("murmur.exe", "windows"), ("Murmur.exe", "windows"),
                  ("notion.id", "android"), ("com.notion.id", "android"), ("Notion.exe", "windows"),
                  ("dev.murmur", "android")]:
    for n, c in [("empty", {}), ("so", {"before": "so "}), ("ok", {"before": "ok "}), ("long", {"before": "we went to the shop "}),
                 ("after", {"after": "x"}), ("so_after", {"before": "so ", "after": "x"}), ("sel", {"selected": "bad"}),
                 ("nosp", {"before": "Done."}), ("ell", {"before": "we went..."})]:
        ap(f"r4_{app}_{n}", T, {"app_id": app, **c}, {"platform": plat})
    ap(f"r4_{app}_casual", T, {"app_id": app, "before": "so "}, {"platform": plat, **sty("casual")})
    ap(f"r4_{app}_enter", "Send it press enter.", {"app_id": app}, {"platform": plat})
for h in ["notion.so", "notion.so/page", "www.notion.so", "x.notion.site", "Notion.so"]:
    for n, c in [("so", {"before": "so "}), ("long", {"before": "we went to the shop "})]:
        ap(f"r4_host_{h}_{n}", T, {"app_id": "chrome.exe", "url_host": h, **c})
        ap(f"r4_ahost_{h}_{n}", T, {"app_id": "com.android.chrome", "url_host": h, **c}, AND)
for i in ["olk", "ms-teams", "teams", "zoom", "hxoutlook", "notepad", "winword", "perplexity", "kitty"]:
    ap(f"r4_chatw_{i}", SEE, {"app_id": i + ".exe"})
    ap(f"r4_sigw_{i}", "Hi Tom, thanks.", {"app_id": i + ".exe"}, SIG)
for i in ["com.slack", "us.zoom.videomeetings", "com.tencent.mm", "com.microsoft.outlook.lite", "com.example.mail"]:
    ap(f"r4_chata_{i}", SEE, {"app_id": i}, AND)
# --- round 5: own-app window with a selection keeps the period (any style) ---
for app in ["murmur-windows.exe", "murmur.exe"]:
    for n, c in [("sel", {"selected": "abc"}), ("sel_ba", {"before": "so ", "selected": "abc", "after": " x"}),
                 ("sel_ws", {"selected": "  "}), ("sel_dot", {"selected": "Fine."}), ("nosel", {"before": "so "}),
                 ("line_dot", {"before": "Done.\nso "}), ("line_q", {"before": "so ", "after": " ok?"})]:
        for s in ["casual", "very_casual", "formal"]:
            ap(f"r5_{app}_{n}_{s}", T, {"app_id": app, **c}, {"platform": "windows", **sty(s)})
        ap(f"r5_{app}_{n}_default", T, {"app_id": app, **c}, {"platform": "windows"})
ap("r3e_snip_sig_empty_text", "", {"app_id": "outlook.exe"}, {**D(SN2), **SIG}, raw="my address")

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cases.json")
with open(out, "w") as f:
    json.dump(C, f, ensure_ascii=False, indent=0)
print(len(C))
