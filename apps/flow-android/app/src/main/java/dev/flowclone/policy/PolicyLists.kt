package dev.flowclone.policy

/**
 * Which apps get special handling, by package name (and, for the style buckets, by the host
 * of the page open in a browser).
 *
 * Method: each group is written from what our code does with it (the doc on each group says
 * which caller), using publicly known package names of popular Android apps. Where a naming
 * rule covers a group (launchers contain "launcher", system UI ends in ".systemui", banking
 * apps say "bank"), the rule is used and the list keeps only the apps the rule misses. Lists
 * stay short on purpose: a missing app falls back to the default path (bubble shown, style
 * "other", normal insert verification), except [isRestricted], which errs toward hiding.
 * A user who needs more uses the per-app settings (denylist, style bucket).
 */
object PolicyLists {
    private fun String.inFamily(root: String) = this == root || startsWith("$root.")
    private fun String.segments() = lowercase().split('.')

    // ---- Insertion (V3Inserter, FlowAccessibilityService keep-alive) ----

    /**
     * Chat and assistant apps whose compose boxes (custom or web-based editors) drop the IME
     * session or report no surrounding text while the text did land. They get the stale-session
     * probe and keep-alive nudge, and a first null read after commit counts as success.
     */
    val MESSAGING: Set<String> = setOf(
        "com.whatsapp",
        "com.whatsapp.w4b",
        "org.telegram.messenger",
        "org.thoughtcrime.securesms",
        "com.facebook.orca",
        "com.instagram.android",
        "com.snapchat.android",
        "com.discord",
        "com.viber.voip",
        "jp.naver.line.android",
        "com.tencent.mm",
        "ch.threema.app",
        "im.vector.app",
        "com.google.android.apps.messaging",
        "com.samsung.android.messaging",
        "com.Slack",
        "com.microsoft.teams",
        "com.openai.chatgpt",
        "com.anthropic.claude",
        "com.google.android.apps.bard",
    )

    /** Chrome and its channels: web-content editors, the other source of legitimately null reads. */
    val CHROME_FAMILY: Set<String> = setOf(
        "com.android.chrome",
        "com.chrome.beta",
        "com.chrome.dev",
        "com.chrome.canary",
        "org.chromium.chrome",
    )

    /** Gmail's compose body is web content too. */
    private const val GMAIL = "com.google.android.gm"

    /** Stale-session probe + keep-alive nudge. */
    fun isRestartProne(pkg: String?) = pkg != null && pkg in MESSAGING

    /** getSurroundingText may be null after a good commit: trust the commit on the first null read. */
    fun isKnownNullSurroundingText(pkg: String?) =
        pkg != null && (pkg in MESSAGING || pkg in CHROME_FAMILY || pkg == GMAIL)

    /** Launchers, Settings search and the Google search bar: re-acquire the input surface. */
    fun isSystemSearch(pkg: String?): Boolean {
        val p = pkg?.lowercase() ?: return false
        return "launcher" in p || p == "com.miui.home" || p.endsWith(".settings") ||
            p == "com.google.android.googlequicksearchbox" || isSystemUi(pkg)
    }

    /** Dialer and in-call screens: re-acquire the input surface. */
    fun isInCall(pkg: String?): Boolean {
        val p = pkg?.lowercase() ?: return false
        return "dialer" in p || "incallui" in p
    }

    /** WhatsApp's compose box shows "Message" as real text until you type; read it as empty. */
    fun usesComposePlaceholders(pkg: String?) = pkg != null && pkg.inFamily("com.whatsapp")

    /** The localized "Message" placeholder, normalized (lowercase, no trailing dots). */
    val COMPOSE_PLACEHOLDERS: Set<String> = setOf(
        "message",
        "mensaje",
        "mensagem",
        "nachricht",
        "messaggio",
        "bericht",
        "wiadomość",
        "mesaj",
        "pesan",
        "сообщение",
        "رسالة",
        "הודעה",
        "संदेश",
        "メッセージ",
        "메시지",
        "消息",
    )

    // ---- Bubble visibility (BubbleVisibilityPolicy) ----

    /** Notification shade / quick reply: any OEM's System UI package. */
    fun isSystemUi(pkg: String?): Boolean {
        val p = pkg?.lowercase() ?: return false
        return p.endsWith(".systemui") || ".systemui." in p
    }

    /** Browsers the URL-bar override (IME action Go) applies to, beyond the rule in [isBrowser]. */
    val BROWSERS: Set<String> = CHROME_FAMILY + setOf(
        "com.microsoft.emmx",
        "com.duckduckgo.mobile.android",
        "com.opera.mini.native",
    )

    /** [BROWSERS], any Mozilla app (Firefox, Focus) and any package with "browser" in a name part. */
    fun isBrowser(pkg: String?): Boolean {
        if (pkg == null) return false
        return pkg in BROWSERS || pkg.inFamily("org.mozilla") || pkg.segments().any { "browser" in it }
    }

    /**
     * Banking, payment, password-manager and authenticator apps: the bubble stays hidden and
     * nothing is read. Errs toward hiding: any package whose name says bank / banco / banca /
     * banque / wallet / authenticator / password / keepass is restricted, plus [RESTRICTED].
     */
    private val RESTRICTED_WORDS = listOf("bank", "banco", "banca", "banque", "wallet", "authenticator", "password", "keepass")

    /** Restricted apps whose package names the word rule misses. */
    val RESTRICTED: Set<String> = setOf(
        // Banks
        "com.chase.sig.android",
        "com.infonow.bofa",
        "com.wf.wellsfargomobile",
        "com.citi.citimobile",
        "com.konylabs.capitalone",
        "com.americanexpress.android.acctsvcs.us",
        "com.discoverfinancial.mobile",
        "com.usaa.mobile.android.usaa",
        "com.schwab.mobile",
        "com.td",
        "com.rbc.mobile.android",
        "com.ing.mobile",
        "com.sbi.lotusintouch",
        "com.axis.mobile",
        "com.ideomobile.hapoalim",
        // Payments, neobanks, brokers, crypto
        "com.paypal.android.p2pmobile",
        "com.venmo",
        "com.squareup.cash",
        "com.zellepay.zelle",
        "com.google.android.apps.nbu.paisa.user",
        "com.samsung.android.spay",
        "com.phonepe.app",
        "net.one97.paytm",
        "com.revolut.revolut",
        "com.transferwise.android",
        "com.monzo.android",
        "de.number26.android",
        "com.nu.production",
        "com.robinhood.android",
        "com.coinbase.android",
        "com.binance.dev",
        // Password managers, 2FA
        "com.x8bit.bitwarden",
        "com.onepassword.android",
        "com.lastpass.lpandroid",
        "com.dashlane",
        "proton.android.pass",
        "com.authy.authy",
    )

    fun isRestricted(pkg: String?): Boolean {
        if (pkg == null) return false
        if (pkg in RESTRICTED) return true
        val p = pkg.lowercase()
        return RESTRICTED_WORDS.any { it in p }
    }

    // ---- Style buckets (AppCategorizer) ----

    val EMAIL_PACKAGES: Set<String> = setOf(
        GMAIL,
        "com.microsoft.office.outlook",
        "ch.protonmail.android",
        "de.tutao.tutanota",
        "net.thunderbird.android",
        "com.fsck.k9",
        "com.readdle.spark",
        "com.fastmail.app",
    )

    /** [EMAIL_PACKAGES], or a package with a "mail" / "email" name part (Yahoo Mail, Samsung Email, FairEmail). */
    fun isEmailApp(pkg: String?): Boolean {
        if (pkg == null) return false
        return pkg in EMAIL_PACKAGES || pkg.segments().any { it == "mail" || it == "email" }
    }

    val WORK_PACKAGES: Set<String> = setOf(
        "com.Slack",
        "com.microsoft.teams",
        "com.google.android.apps.dynamite",
        "com.google.android.apps.meetings",
        "us.zoom.videomeetings",
        "com.cisco.webex.meetings",
        "com.mattermost.rn",
        "com.zulipmobile",
        "com.linkedin.android",
        "notion.id",
        "com.asana.app",
        "com.trello",
        "com.github.android",
        "com.foxdebug.acode",
    )

    /** Package families that are all work tools: Atlassian, Office, Google Docs/Sheets/Slides/Drive, Zoho, Salesforce. */
    private val WORK_FAMILIES = listOf(
        "com.atlassian",
        "com.microsoft.office",
        "com.google.android.apps.docs",
        "com.zoho",
        "com.salesforce",
    )

    fun isWorkApp(pkg: String?): Boolean {
        if (pkg == null) return false
        return pkg in WORK_PACKAGES || WORK_FAMILIES.any { pkg.inFamily(it) }
    }

    val PERSONAL_PACKAGES: Set<String> = setOf(
        "org.thoughtcrime.securesms",
        "com.facebook.orca",
        "com.discord",
        "com.viber.voip",
        "jp.naver.line.android",
        "com.tencent.mm",
        "com.kakao.talk",
        "ch.threema.app",
        "com.google.android.apps.messaging",
        "com.samsung.android.messaging",
        "com.android.mms",
        "com.instagram.android",
        "com.instagram.barcelona",
        "com.snapchat.android",
        "com.facebook.katana",
        "com.twitter.android",
        "com.reddit.frontpage",
        "com.zhiliaoapp.musically",
        "xyz.blueskyweb.app",
        "org.joinmastodon.android",
    )

    /** [PERSONAL_PACKAGES], WhatsApp (incl. Business) and Telegram (incl. its official forks). */
    fun isPersonalApp(pkg: String?): Boolean {
        if (pkg == null) return false
        return pkg in PERSONAL_PACKAGES || pkg.inFamily("com.whatsapp") || pkg.inFamily("org.telegram")
    }

    /** Host suffixes (a host matches itself and its subdomains). */
    val PERSONAL_WEB_HOSTS: Set<String> = setOf(
        "whatsapp.com",
        "telegram.org",
        "discord.com",
        "messenger.com",
        "instagram.com",
        "facebook.com",
        "x.com",
        "twitter.com",
        "reddit.com",
        "threads.net",
        "bsky.app",
        "tiktok.com",
    )

    val WORK_WEB_HOSTS: Set<String> = setOf(
        "slack.com",
        "teams.microsoft.com",
        "teams.live.com",
        "chat.google.com",
        "meet.google.com",
        "docs.google.com",
        "linkedin.com",
        "github.com",
        "gitlab.com",
        "atlassian.net",
        "notion.so",
        "linear.app",
        "figma.com",
        "zoom.us",
        "office.com",
        "sharepoint.com",
    )

    /** Webmail that the "mail." / "webmail." host rule in [isEmailHost] misses. */
    val EMAIL_WEB_HOSTS: Set<String> = setOf(
        "outlook.live.com",
        "outlook.office.com",
        "outlook.office365.com",
        "app.fastmail.com",
        "app.tuta.com",
    )

    private fun matchesAny(host: String?, hosts: Set<String>) =
        host != null && hosts.any { host == it || host.endsWith(".$it") }

    /** [host] is lowercase with any "www." removed. */
    fun isPersonalHost(host: String?) = matchesAny(host, PERSONAL_WEB_HOSTS)
    fun isWorkHost(host: String?) = matchesAny(host, WORK_WEB_HOSTS)
    fun isEmailHost(host: String?) =
        host != null && (host.startsWith("mail.") || host.startsWith("webmail.") || matchesAny(host, EMAIL_WEB_HOSTS))
}
