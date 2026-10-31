package dev.flowclone.ui

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.flowclone.R
import dev.flowclone.admin.AdminApi
import dev.flowclone.admin.AdminResult
import dev.flowclone.admin.CreatedKey
import dev.flowclone.admin.KeyView
import dev.flowclone.admin.SecretClip
import dev.flowclone.admin.UsersKeys
import dev.flowclone.admin.WhoAmI
import kotlin.concurrent.thread

/**
 * Settings → Your server → Users & keys (D39; Windows: pages/settings/keys.ts). Reached from the
 * Server page's USERS & KEYS row, which only appears when this app's saved token is an admin key.
 *   KEYS     every key with its usage (client, last seen, dictations, audio, model downloads);
 *            tap / "Revoke" → confirm (warns when it is this app's own key). The last admin key
 *            can't be revoked.
 *   NEW KEY  name + admin switch → Create key → the secret, shown once in a dialog with Copy
 *            (a sensitive clip). The dialog's window is FLAG_SECURE, and the secret leaves the
 *            view (and this activity) when it closes; it is never logged or stored.
 * A `forbidden` / `unauthorized` answer (key no longer admin, revoked) closes the page.
 */
class UsersKeysActivity : Activity() {
    private lateinit var ui: DevUi
    private var api: AdminApi? = null
    private lateinit var listCard: LinearLayout
    private lateinit var banner: DevUi.Result
    private lateinit var status: DevUi.Result
    private lateinit var name: DevUi.FieldRow
    private lateinit var admin: DevUi.ToggleRow
    private var keys: List<KeyView> = emptyList()
    private var busy = false
    /** The open secret dialog (tests read it; cleared on dismiss). */
    internal var secretDialog: AlertDialog? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = UsersKeys.TITLE
        ui = DevUi(this)
        api = UsersKeys.factory(this)
        if (api == null) { finish(); return }
        render()
        load()
    }

    override fun onDestroy() {
        secretDialog?.dismiss()
        super.onDestroy()
    }

    private val alive get() = !isFinishing && !isDestroyed

    private fun render() = with(ui) {
        screen {
            topBar(UsersKeys.TITLE, DevUi.TopNav.BACK, onNav = ::finish,
                actions = listOf(DevUi.TopAction(R.drawable.ic_dev_reset, "Refresh the keys") { load() }))
            note("Give each person or device its own key, so you can see who uses the server and switch one off without touching the others. Revoking a key stops its next connection; a dictation already running finishes.")
            banner = banner()
            sectionLabel("KEYS")
            listCard = card { emptyState("Loading…", "Asking the server for its keys") }
            sectionLabel("NEW KEY")
            card {
                name = fieldRow("Name", "", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS,
                    help = "Who or what it is for. Letters, digits, . _ -", hint = "phone") { name.setError(null) }
                admin = toggleRow("Admin", "Can open this page and create or revoke keys. Only for devices you own", false)
            }
            action("Create key", DevUi.Style.PRIMARY) { onCreateKey() }
            status = result()
        }
    }

    /** Off the main thread; [then] runs on it while the page is open. Errors that end the page close it. */
    private fun <T> request(what: String, call: (AdminApi) -> AdminResult<T>, then: (AdminResult<T>) -> Unit) {
        val a = api ?: return
        thread(name = "users-keys-$what") {
            val r = call(a)
            runOnUiThread {
                if (!alive) return@runOnUiThread
                if (r is AdminResult.Err && UsersKeys.mustLeave(r.code)) {
                    Toast.makeText(this, UsersKeys.errorText(r), Toast.LENGTH_LONG).show()
                    finish()
                    return@runOnUiThread
                }
                then(r)
            }
        }
    }

    private fun load() {
        request("list", AdminApi::listKeys) { r ->
            when (r) {
                is AdminResult.Ok -> { keys = r.value; banner.show(""); renderKeys() }
                is AdminResult.Err -> {
                    banner.show(UsersKeys.errorText(r), Tone.BAD)
                    if (keys.isEmpty()) { listCard.removeAllViews(); with(ui) { listCard.emptyState("Couldn't load the keys", "Check the connection, then Refresh") } }
                }
            }
        }
    }

    private fun renderKeys() = with(ui) {
        listCard.removeAllViews()
        if (keys.isEmpty()) { listCard.emptyState("No keys yet", "Create one below"); return@with }
        for (k in UsersKeys.sorted(keys)) {
            val can = UsersKeys.canRevoke(k, keys)
            val revoke = if (can) chip("Revoke") { confirmRevoke(k) }.apply { contentDescription = "Revoke ${k.name}" } else null
            listCard.listRow(
                title = k.name, meta = UsersKeys.meta(k), badge = UsersKeys.badge(k),
                flag = when {
                    !k.active -> "Revoked"
                    k.admin && !can -> "Last admin key: create another admin key to revoke it"
                    else -> null
                },
                flagTone = if (k.active) Tone.WARN else Tone.BAD,
                trailing = revoke,
                actions = if (can) listOf("Revoke ${k.name}" to { confirmRevoke(k) }) else emptyList(),
            ) { if (can) confirmRevoke(k) }
        }
    }

    private fun onCreateKey() {
        if (busy) return
        val n = name.text.trim()
        UsersKeys.nameError(n)?.let { name.setError(it); return }
        busy = true
        status.show("Creating $n…")
        request("create", { it.createKey(n, admin.isChecked) }) { r ->
            busy = false
            when (r) {
                is AdminResult.Ok -> {
                    name.edit.setText(""); admin.isChecked = false
                    status.show("Created ${r.value.key.name}", Tone.OK)
                    showSecret(r.value)
                    load()
                }
                is AdminResult.Err -> status.show(UsersKeys.errorText(r), Tone.BAD)
            }
        }
    }

    /** The secret, once: monospace, Copy (sensitive clip), Done. Gone from the view on close. */
    private fun showSecret(c: CreatedKey) = with(ui) {
        val secret = tv(c.secret, 16f).apply {
            typeface = Typeface.MONOSPACE
            setPadding(dp(4), dp(12), dp(4), dp(12))
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
            importantForContentCapture = View.IMPORTANT_FOR_CONTENT_CAPTURE_NO_EXCLUDE_DESCENDANTS
        }
        val copied = tv("", TypeScale.SUB, p.muted).apply { visibility = View.GONE; accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE }
        val label = c.key.name
        val box = LinearLayout(this@UsersKeysActivity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(8), dp(24), 0)
            addView(tv("Copy it now: this is the only time it is shown. Paste it into the app on \"$label\" as the server token.", TypeScale.SUB, p.text))
            addView(secret)
            addView(outlinedButton("Copy") {
                val ok = SecretClip.copy(this@UsersKeysActivity, "Server key for $label", secret.text.toString())
                copied.text = if (ok) "Copied." else "Couldn't copy: no clipboard."
                copied.visibility = View.VISIBLE
            }.apply { contentDescription = "Copy the new key"; minHeight = dp(48); minimumHeight = dp(48) })
            addView(copied)
        }
        val d = AlertDialog.Builder(this@UsersKeysActivity)
            .setTitle("Key created")
            .setView(box)
            .setCancelable(false)
            .setPositiveButton("Done", null)
            .create()
        d.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        d.setOnDismissListener { secret.text = ""; secretDialog = null }
        secretDialog = d
        d.show()
    }

    private fun confirmRevoke(k: KeyView) {
        ui.confirm("Revoke ${k.name}?", UsersKeys.revokeMessage(k), "Revoke", onYes = { revoke(k) })
    }

    private fun revoke(k: KeyView) {
        status.show("Revoking ${k.name}…")
        request("revoke", { it.revokeKey(k.id) }) { r ->
            when (r) {
                is AdminResult.Ok -> { status.show("Revoked ${k.name}", Tone.OK); load() }
                is AdminResult.Err -> status.show(UsersKeys.errorText(r), Tone.BAD)
            }
        }
    }

    companion object {
        fun open(ctx: Context) = ctx.startActivity(Intent(ctx, UsersKeysActivity::class.java))

        /**
         * The Server page's USERS & KEYS section: hidden until `whoami` says the saved key is an
         * admin key (403, 401, offline, an old server, the local flavour: stays hidden). Call
         * [Entry.check] again after a save; it asks only when the saved server changed.
         */
        internal fun entry(a: Activity, ui: DevUi, parent: LinearLayout): Entry = with(ui) {
            val label = parent.sectionLabel("USERS & KEYS").apply { visibility = View.GONE }
            lateinit var row: DevUi.NavRow
            val card = parent.card { row = navRow(R.drawable.ic_dev_lock, UsersKeys.TITLE, "") { open(a) } }.apply { visibility = View.GONE }
            Entry(a, label, card, row).also { it.check() }
        }
    }

    internal class Entry(private val a: Activity, private val label: View, private val card: View, private val row: DevUi.NavRow) {
        private var askedFor: String? = null

        fun check() {
            val api = UsersKeys.factory(a)
            if (api == null) { show(null); askedFor = null; return }
            val key = dev.flowclone.FlowApp.of(a).dev.load().toServerConfigJson().toString()
            if (key == askedFor) return
            askedFor = key
            thread(name = "users-keys-whoami") {
                val r = api.whoami()
                a.runOnUiThread { if (!a.isDestroyed && askedFor == key) show(r) }
            }
        }

        private fun show(r: AdminResult<WhoAmI>?) {
            val on = UsersKeys.showEntry(r)
            label.visibility = if (on) View.VISIBLE else View.GONE
            card.visibility = if (on) View.VISIBLE else View.GONE
            if (on) row.setSubtitle("This app's key \"${(r as AdminResult.Ok).value.name}\" is an admin key · see usage, create and revoke keys")
        }
    }
}
