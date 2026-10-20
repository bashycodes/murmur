package dev.flowclone.policy

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

    fun isBanking(pkg: String?) = pkg != null && pkg in PolicyLists.RESTRICTED_BANKING

    fun isNotificationReplyContext(pkg: String?, imeWindowPresent: Boolean, focusedAppWindowPresent: Boolean) =
        (pkg != null && pkg in PolicyLists.SYSTEM_UI) || (imeWindowPresent && !focusedAppWindowPresent)

    fun evaluate(i: Input, userDenylist: Set<String> = emptySet()): Decision {
        val t = i.inputType
        val password = EditorInputType.isPassword(t) || i.nodePassword == true
        val action = i.imeOptions and IME_ACTION_MASK
        val override = when {
            i.notificationReplyContext -> Override.NotificationReply
            // Deviation 1: the URL-bar override never applies to a password field.
            !password && (EditorInputType.isUri(t) || EditorInputType.isWebEditText(t) ||
                (i.packageName in PolicyLists.BROWSERS && action == IME_ACTION_GO)) -> Override.UrlBar
            else -> null
        }
        val allowed = !(EditorInputType.isSensitive(t) || i.nodePassword == true) || override != null
        val show = i.imeVisible && allowed && !isBanking(i.packageName) &&
            (i.packageName == null || i.packageName !in userDenylist) &&
            !(i.accessibilityDataSensitive && override != Override.NotificationReply)
        val isSearch = show && override != Override.UrlBar &&
            (action == IME_ACTION_SEARCH || EditorInputType.isFilter(t))
        return Decision(show, override, isSearch)
    }
}

/** Package / browser-host → style bucket (Wispr `AppCategorizer`, app_categories.json). */
object AppCategorizer {
    enum class Category(val coreName: String) {
        EMAIL("email"), WORK("work_chat"), PERSONAL("personal_chat"), OTHER("other")
    }

    private fun matchesAny(host: String?, hosts: Set<String>) =
        host != null && hosts.any { host == it || host.endsWith(".$it") }

    fun categorize(packageName: String?, browserHost: String?): Category {
        val pkg = packageName?.takeIf { it.isNotBlank() }
        val host = browserHost?.lowercase()?.removePrefix("www.")?.takeIf { it.isNotBlank() }
        return when {
            pkg in PolicyLists.PERSONAL_PACKAGES || matchesAny(host, PolicyLists.PERSONAL_WEB_HOSTS) -> Category.PERSONAL
            pkg in PolicyLists.EMAIL_PACKAGES || matchesAny(host, PolicyLists.EMAIL_WEB_HOSTS) -> Category.EMAIL
            pkg in PolicyLists.WORK_PACKAGES || matchesAny(host, PolicyLists.WORK_WEB_HOSTS) -> Category.WORK
            else -> Category.OTHER
        }
    }
}
