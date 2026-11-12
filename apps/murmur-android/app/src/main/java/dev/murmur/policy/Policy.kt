package dev.murmur.policy

/**
 * `EditorInfo.inputType` predicates (spec 07 §4.1). ALL number fields count as sensitive
 * (OTP / PIN / card numbers), as do phone fields and every password variation.
 */
object EditorInputType {
    private const val CLASS_MASK = 0x0F
    private const val VARIATION_MASK = 0xFF0
    private const val CLASS_TEXT = 1
    private const val CLASS_NUMBER = 2
    private const val CLASS_PHONE = 3

    private fun cls(t: Int) = t and CLASS_MASK
    private fun variation(t: Int) = t and VARIATION_MASK

    fun isText(t: Int) = cls(t) == CLASS_TEXT
    fun isNumber(t: Int) = cls(t) == CLASS_NUMBER
    fun isPhone(t: Int) = cls(t) == CLASS_PHONE
    fun isPassword(t: Int) =
        (isNumber(t) && variation(t) == 0x10) || (isText(t) && variation(t) in setOf(0x80, 0x90, 0xE0))
    fun isSensitive(t: Int) = isNumber(t) || isPhone(t) || isPassword(t)
    fun isUri(t: Int) = isText(t) && variation(t) == 0x10
    fun isWebEditText(t: Int) = isText(t) && variation(t) == 0xA0
    fun isFilter(t: Int) = isText(t) && variation(t) == 0xB0
    fun isContentExtractionExcluded(t: Int) = isSensitive(t) || isUri(t)
}

/** Bubble show/hide decision (spec 07 §2.2, §4.2, clone deviations 1-3). */
object BubbleVisibilityPolicy {
    enum class Override { NotificationReply, UrlBar }

    data class Input(
        val inputType: Int,
        val imeOptions: Int,
        val packageName: String?,
        val nodePassword: Boolean?,
        val imeVisible: Boolean,
        val notificationReplyContext: Boolean,
        /** `AccessibilityNodeInfo.isAccessibilityDataSensitive` (API 34+); clone deviation 2. */
        val accessibilityDataSensitive: Boolean = false,
    )

    data class Decision(val show: Boolean, val override: Override?, val isSearch: Boolean)

    private const val IME_ACTION_MASK = 0xFF
    private const val IME_ACTION_GO = 2
    private const val IME_ACTION_SEARCH = 3

    /** Banking and payment apps: no bubble, no reads ([PolicyLists.isRestricted]). */
    fun isRestricted(pkg: String?) = PolicyLists.isRestricted(pkg)

    fun isNotificationReplyContext(pkg: String?, imeWindowPresent: Boolean, focusedAppWindowPresent: Boolean) =
        PolicyLists.isSystemUi(pkg) || (imeWindowPresent && !focusedAppWindowPresent)

    fun evaluate(i: Input, userDenylist: Set<String> = emptySet()): Decision {
        val t = i.inputType
        val password = EditorInputType.isPassword(t) || i.nodePassword == true
        val action = i.imeOptions and IME_ACTION_MASK
        val override = when {
            i.notificationReplyContext -> Override.NotificationReply
            // Deviation 1: the URL-bar override never applies to a password field.
            !password && (EditorInputType.isUri(t) || EditorInputType.isWebEditText(t) ||
                (PolicyLists.isBrowser(i.packageName) && action == IME_ACTION_GO)) -> Override.UrlBar
            else -> null
        }
        val allowed = !(EditorInputType.isSensitive(t) || i.nodePassword == true) || override != null
        val show = i.imeVisible && allowed && !isRestricted(i.packageName) &&
            (i.packageName == null || i.packageName !in userDenylist) &&
            !(i.accessibilityDataSensitive && override != Override.NotificationReply)
        val isSearch = show && override != Override.UrlBar &&
            (action == IME_ACTION_SEARCH || EditorInputType.isFilter(t))
        return Decision(show, override, isSearch)
    }
}

/** Package / browser-host → style bucket (groups and rules in [PolicyLists]). */
object AppCategorizer {
    enum class Category(val coreName: String) {
        EMAIL("email"), WORK("work_chat"), PERSONAL("personal_chat"), OTHER("other")
    }

    fun categorize(packageName: String?, browserHost: String?): Category {
        val pkg = packageName?.takeIf { it.isNotBlank() }
        val host = browserHost?.lowercase()?.removePrefix("www.")?.takeIf { it.isNotBlank() }
        return when {
            PolicyLists.isPersonalApp(pkg) || PolicyLists.isPersonalHost(host) -> Category.PERSONAL
            PolicyLists.isEmailApp(pkg) || PolicyLists.isEmailHost(host) -> Category.EMAIL
            PolicyLists.isWorkApp(pkg) || PolicyLists.isWorkHost(host) -> Category.WORK
            else -> Category.OTHER
        }
    }
}

/**
 * Review S12: the transcript goes only into the field that was focused when the dictation
 * started. Right before inserting, the shell compares that pinned field with the one focused
 * now and re-runs the bubble policy (banking list, user denylist, password / sensitive input).
 */
object InsertGuard {
    /** Package + loose identity hash (package, class, view id, description) of a focused field. */
    data class Field(val pkg: String?, val identity: Long?)

    fun check(start: Field?, now: Field, policyAllows: Boolean, explicit: Boolean): dev.murmur.session.InsertTarget {
        if (!policyAllows) return dev.murmur.session.InsertTarget.PROTECTED
        if (explicit || start == null) return dev.murmur.session.InsertTarget.OK
        if (start.pkg != null && now.pkg != null && start.pkg != now.pkg) return dev.murmur.session.InsertTarget.CHANGED
        if (start.identity != null && now.identity != null && start.identity != now.identity) return dev.murmur.session.InsertTarget.CHANGED
        return dev.murmur.session.InsertTarget.OK
    }
}

/**
 * This app's own fields. A settings field is not a place to dictate: it gives no Ready bubble and
 * takes no insert, except while a screen made for dictation is in front (a note editor, a
 * [dev.murmur.bubble.DictationScreen], per [dev.murmur.bubble.OwnScreens]). Keeping the bubble off
 * our screens in every other state is [dev.murmur.bubble.BubblePresence.visible]'s job.
 */
object OwnAppGate {
    /** True when [pkg] is this app and no dictation screen of ours is in front: no Ready bubble, no insert. */
    fun blocks(pkg: String?, ownPackage: String, dictationScreenInFront: Boolean) = pkg == ownPackage && !dictationScreenInFront
}
