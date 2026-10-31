"""Headless check of Settings -> Users & keys (D39) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_users_keys.py [port]
The page shows only when this app's saved server key is an admin key
(`admin_status`): nav item + a "Manage keys" group on Your server. Covers the
key list (usage, badges, revoked rows, the last admin key's Revoke disabled),
create (empty / bad / duplicate names refused; the secret shown once in a copy
box, copied through `clipboard_copy`, gone from the page after Done or Esc),
revoke with a confirm (cancel = no call), revoking this app's own key (warning,
then the page goes away), a non-admin key (?admin=0) and an unreachable server
(?admin=down). Writes shots/keys-*.png (gitignored).
"""
import pathlib
import sys

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
port = sys.argv[1] if len(sys.argv) > 1 else "8765"
out = HERE / "shots"
out.mkdir(exist_ok=True)
fails = []
SECRET = "MOCKSECRET-"


def check(name, cond, extra=""):
    print(("ok   " if cond else "FAIL ") + name + (f"  ({extra})" if extra and not cond else ""))
    if not cond:
        fails.append(name)


def calls(pg, cmd):
    return pg.evaluate("(c) => window.__mockCalls.filter(x => x[0] === c).map(x => x[1])", cmd)


def open_page(b, scheme, query=""):
    pg = b.new_page(viewport={"width": 1280, "height": 900}, color_scheme=scheme)
    pg.on("pageerror", lambda e: errors.append(str(e)))
    pg.goto(f"http://127.0.0.1:{port}/settings.html{query}")
    pg.wait_for_timeout(600)
    return pg


def rows(pg):
    return pg.eval_on_selector_all("#keys_rows tr", "rs => rs.map(r => r.innerText)")


def row_of(pg, name):
    return f"#keys_rows tr:has(.lw-key-name:text-is('{name}'))"


with sync_playwright() as p:
    b = p.chromium.launch()
    errors = []
    for scheme in ["light", "dark"]:
        tag = f"[{scheme}] "
        pg = open_page(b, scheme)
        pg.evaluate("window.__goPage('settings-server')")
        pg.wait_for_timeout(300)
        check(tag + "admin key: nav item shown", pg.is_visible("#tab_keys") and pg.inner_text("#tab_keys").strip() == "Users & keys")
        check(tag + "Your server: Manage keys group", pg.is_visible("#srv_keys") and "owner" in pg.inner_text("#srv_keys_who"), pg.inner_text("#srv_keys"))
        pg.click("#srv_keys button[data-go='settings-keys']")
        pg.wait_for_timeout(300)
        check(tag + "Manage keys opens the page", pg.is_visible("#page_settings-keys") and len(calls(pg, "admin_list_keys")) == 1)
        r = rows(pg)
        check(tag + "three keys, revoked last", len(r) == 3 and "old-tablet" in r[2], str(r))
        owner = pg.inner_text(row_of(pg, "owner"))
        check(tag + "owner row: badges and usage", "admin" in owner and "this app" in owner and "412" in owner and "66.0" in owner and "flow-windows 0.12.0" in owner, owner)
        check(tag + "revoked row: no button, says revoked", pg.locator(row_of(pg, "old-tablet") + " button").count() == 0 and "Revoked" in pg.inner_text(row_of(pg, "old-tablet")) and "is-revoked" in pg.get_attribute(row_of(pg, "old-tablet"), "class"))
        check(tag + "last admin: Revoke disabled with a reason", pg.is_disabled(row_of(pg, "owner") + " button") and "last admin" in (pg.get_attribute(row_of(pg, "owner") + " button", "title") or ""))
        if scheme == "light":
            pg.locator("#page_settings-keys").screenshot(path=str(out / "keys-page.png"))
        else:
            pg.locator("#page_settings-keys").screenshot(path=str(out / "keys-page-dark.png"))

        # --- create: refused names
        pg.click("#keys_create")
        pg.wait_for_timeout(100)
        check(tag + "empty name: no call", calls(pg, "admin_create_key") == [] and "name" in pg.inner_text("#keys_status"))
        pg.fill("#keys_name", "bad name")
        pg.click("#keys_create")
        pg.wait_for_timeout(150)
        check(tag + "bad name: server's message", "1-64 characters" in pg.inner_text("#keys_status") and not pg.is_visible("#keys_secret_dlg"), pg.inner_text("#keys_status"))
        pg.fill("#keys_name", "phone")
        pg.click("#keys_create")
        pg.wait_for_timeout(150)
        check(tag + "duplicate: already exists", "already exists" in pg.inner_text("#keys_status"), pg.inner_text("#keys_status"))

        # --- create an admin key: secret once
        pg.fill("#keys_name", "laptop")
        pg.check("#keys_admin")
        pg.click("#keys_create")
        pg.wait_for_timeout(250)
        check(tag + "create sent name + admin", calls(pg, "admin_create_key")[-1] == {"name": "laptop", "admin": True}, str(calls(pg, "admin_create_key")))
        check(tag + "secret dialog shows the key", pg.is_visible("#keys_secret_dlg") and pg.input_value("#keys_secret").startswith(SECRET) and pg.inner_text("#keys_secret_name") == "laptop")
        check(tag + "says shown once", "only time" in pg.inner_text("#keys_secret_dlg"))
        if scheme == "light":
            pg.locator("#keys_secret_dlg").screenshot(path=str(out / "keys-secret.png"))
        pg.click("#keys_secret_copy")
        pg.wait_for_timeout(100)
        cp = calls(pg, "clipboard_copy")
        check(tag + "Copy -> clipboard_copy(secret)", len(cp) == 1 and cp[0]["text"].startswith(SECRET) and pg.is_visible("#keys_secret_copied"))
        pg.click("#keys_secret_done")
        pg.wait_for_timeout(150)
        html = pg.evaluate("document.documentElement.outerHTML + [...document.querySelectorAll('input')].map(i => i.value).join(' ')")
        check(tag + "Done: secret gone from the page", not pg.is_visible("#keys_secret_dlg") and SECRET not in html)
        check(tag + "form cleared, list reloaded", pg.input_value("#keys_name") == "" and not pg.is_checked("#keys_admin") and len(rows(pg)) == 4)
        check(tag + "two admins: owner's Revoke enabled", pg.is_enabled(row_of(pg, "owner") + " button"))
        # Esc closes the secret box too, and still drops the secret.
        pg.fill("#keys_name", "desk")
        pg.click("#keys_create")
        pg.wait_for_timeout(250)
        pg.keyboard.press("Escape")
        pg.wait_for_timeout(150)
        check(tag + "Esc: dialog closed, secret dropped, Settings still open", not pg.is_visible("#keys_secret_dlg") and pg.input_value("#keys_secret") == "" and pg.is_visible("#page_settings-keys"))

        # --- revoke with confirm
        pg.click(row_of(pg, "phone") + " button")
        pg.wait_for_timeout(100)
        check(tag + "confirm names the key, no self warning", pg.is_visible("#keys_revoke_dlg") and pg.inner_text("#keys_revoke_title") == "Revoke phone?" and not pg.is_visible("#keys_revoke_self"))
        pg.click("#keys_revoke_cancel")
        pg.wait_for_timeout(100)
        check(tag + "cancel: no call", calls(pg, "admin_revoke_key") == [])
        pg.click(row_of(pg, "phone") + " button")
        pg.wait_for_timeout(100)
        pg.click("#keys_revoke_ok")
        pg.wait_for_timeout(250)
        check(tag + "revoked by id", calls(pg, "admin_revoke_key") == [{"id": "k-1b2c3d4e5f607182"}], str(calls(pg, "admin_revoke_key")))
        check(tag + "phone now revoked", "is-revoked" in pg.get_attribute(row_of(pg, "phone"), "class") and "Revoked phone" in pg.inner_text("#keys_status"))

        # --- revoking this app's own key
        pg.click(row_of(pg, "owner") + " button")
        pg.wait_for_timeout(100)
        check(tag + "own key: warning shown", pg.is_visible("#keys_revoke_self") and "this app" in pg.inner_text("#keys_revoke_self"))
        if scheme == "light":
            pg.locator("#keys_revoke_dlg").screenshot(path=str(out / "keys-revoke-self.png"))
        pg.click("#keys_revoke_ok")
        pg.wait_for_timeout(400)
        check(tag + "own key revoked: page and nav gone", not pg.is_visible("#tab_keys") and not pg.is_visible("#page_settings-keys") and pg.is_visible("#page_settings-server") and not pg.is_visible("#srv_keys"))
        pg.close()

        # --- not an admin key / server down
        for q, what in [("?admin=0", "non-admin key"), ("?admin=down", "server down")]:
            pg = open_page(b, scheme, q)
            pg.evaluate("window.__goPage('settings-server')")
            pg.wait_for_timeout(300)
            check(tag + what + ": no nav item, no group", not pg.is_visible("#tab_keys") and not pg.is_visible("#srv_keys"))
            check(tag + what + ": never listed keys", calls(pg, "admin_list_keys") == [] and len(calls(pg, "admin_status")) >= 1)
            pg.close()

    check("no page errors", not errors, "; ".join(errors))
    b.close()

print(f"\n{len(fails)} failed" if fails else "\nall ok")
sys.exit(1 if fails else 0)
