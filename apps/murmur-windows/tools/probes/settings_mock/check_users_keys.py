"""Headless check of Settings -> Users & keys (D39) against mock-tauri.js.

Probe (Playwright + its bundled Chromium):
    python3 tools/probes/settings_mock/serve.py 8765 &
    python3 tools/probes/settings_mock/check_users_keys.py [port]
The page shows only when this app's saved server key is an admin key
(`admin_status`): nav item + a "Manage keys" group on Your server. Covers the
key list (Name / Last used / Dictations / Revoke, the build string
and usage on hover, revoked keys folded under "Show revoked (N)", no
horizontal scroll at 1040px, the solid header under the close button, the last admin key's Revoke disabled),
create (empty / bad / duplicate names refused; the secret shown once in a copy
box, copied through `clipboard_copy`, gone from the page after Done or Esc),
revoke with a confirm (cancel = no call), revoking this app's own key (warning,
then the page goes away), a non-admin key (?admin=0) and an unreachable server
(?admin=down). Writes shots/keys-*.png (gitignored).
"""
import pathlib
import re
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
        check(tag + "two active keys listed", len(r) == 2 and "owner" in r[0] and "phone" in r[1], str(r))
        owner = pg.inner_text(row_of(pg, "owner"))
        check(tag + "owner row: badges and dictations", "admin" in owner and "this app" in owner and "412" in owner, owner)
        tip = pg.get_attribute(row_of(pg, "owner") + " td.name", "title") or ""
        check(tag + "build string and usage on hover, not in the row", tip.startswith("murmur-windows 0.12.0") and "Audio 66.0 min" in tip and "1620.0 MB" in tip and "murmur-windows" not in owner, tip)
        heads = pg.eval_on_selector_all("#keys_table thead th", "hs => hs.map(h => h.innerText.trim())")
        check(tag + "columns: Name, Last used, Dictations, Revoke", heads == ["Name", "Last used", "Dictations", ""] and pg.get_attribute("#keys_table thead th.act", "aria-label") == "Revoke", str(heads))
        cells = lambda n: pg.eval_on_selector_all(row_of(pg, n) + " td", "ts => ts.map(t => t.innerText.trim())")
        date_re = r"^\d{1,2} [A-Z][a-z]{2} \d{4}, \d{2}:\d{2}$"
        check(tag + "owner: last used in the one date format", re.match(date_re, cells("owner")[1]) is not None, str(cells("owner")))
        # Revoked keys fold away under "Show revoked (N)".
        check(tag + "revoked folded: summary with count, closed", pg.inner_text("#keys_revoked_sum") == "Show revoked (1)" and not pg.is_visible("#keys_revoked_rows tr") and not pg.evaluate("document.getElementById('keys_revoked').open"))
        pg.click("#keys_revoked_sum")
        pg.wait_for_timeout(100)
        old = pg.eval_on_selector_all("#keys_revoked_rows tr td", "ts => ts.map(t => t.innerText.trim())")
        check(tag + "pre-split revoked key: last used a dash, revoked date, no button", old[0] == "old-tablet" and old[1] == "—" and re.match(date_re, old[3]) is not None and pg.locator("#keys_revoked_rows button").count() == 0, str(old))
        check(tag + "last admin: Revoke disabled with a reason", pg.is_disabled(row_of(pg, "owner") + " button") and "last admin" in (pg.get_attribute(row_of(pg, "owner") + " button", "title") or ""))
        # At about 1040px the settings body has no horizontal scroll and names don't break mid-word.
        pg.set_viewport_size({"width": 1040, "height": 760})
        pg.wait_for_timeout(150)
        check(tag + "1040px: no horizontal scroll", pg.evaluate("() => { const b = document.getElementById('settings_body'); return b.scrollWidth <= b.clientWidth; }"), pg.evaluate("() => { const b = document.getElementById('settings_body'); return b.scrollWidth + '>' + b.clientWidth; }"))
        check(tag + "1040px: names on one line", pg.evaluate("() => [...document.querySelectorAll('#page_settings-keys .lw-key-name')].every(n => n.getClientRects().length === 1)"))
        # The solid header: scrolled content passes under it, not under the close button.
        pg.evaluate("document.getElementById('settings_body').scrollTop = 200")
        pg.wait_for_timeout(100)
        under = pg.evaluate("() => { const r = document.getElementById('settings_close').getBoundingClientRect(); const e = document.elementFromPoint(r.left - 4, r.top + r.height / 2); return e === document.getElementById('settings_body'); }")
        check(tag + "scrolled: the header covers the strip by the close button", under)
        if scheme == "light":
            pg.screenshot(path=str(out / "keys-1040-scrolled.png"))
        pg.evaluate("document.getElementById('settings_body').scrollTop = 0")
        pg.set_viewport_size({"width": 1280, "height": 900})
        pg.wait_for_timeout(100)
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
        check(tag + "form cleared, list reloaded", pg.input_value("#keys_name") == "" and not pg.is_checked("#keys_admin") and len(rows(pg)) == 3)
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
        check(tag + "phone moves to the revoked list", pg.locator(row_of(pg, "phone")).count() == 0 and pg.locator("#keys_revoked_rows tr:has(.lw-key-name:text-is('phone'))").count() == 1 and pg.inner_text("#keys_revoked_sum") == "Show revoked (2)" and "Revoked phone" in pg.inner_text("#keys_status"))

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
