package dev.murmur.policy

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

// android.text.InputType constants, spelled out so the test documents the bit layout.
private const val TEXT = 0x1
private const val NUMBER = 0x2
private const val PHONE = 0x3
private const val TEXT_PASSWORD = TEXT or 0x80
private const val TEXT_VISIBLE_PASSWORD = TEXT or 0x90
private const val TEXT_WEB_PASSWORD = TEXT or 0xE0
private const val TEXT_URI = TEXT or 0x10
private const val TEXT_WEB_EDIT = TEXT or 0xA0
private const val TEXT_FILTER = TEXT or 0xB0
private const val NUMBER_PASSWORD = NUMBER or 0x10
private const val TEXT_MULTILINE = TEXT or 0x20000
private const val IME_ACTION_GO = 2
private const val IME_ACTION_SEARCH = 3

class EditorInputTypeTest {
    @Test fun plainTextIsNotSensitive() = assertFalse(EditorInputType.isSensitive(TEXT or TEXT_MULTILINE))

    @Test fun allPasswordVariationsAreSensitive() {
        for (t in listOf(TEXT_PASSWORD, TEXT_VISIBLE_PASSWORD, TEXT_WEB_PASSWORD, NUMBER_PASSWORD))
            assertTrue("0x${t.toString(16)}", EditorInputType.isSensitive(t))
    }

    @Test fun everyNumberAndPhoneFieldIsSensitive() {
        assertTrue(EditorInputType.isSensitive(NUMBER))
        assertTrue(EditorInputType.isSensitive(PHONE))
    }

    @Test fun uriIsContentExtractionExcludedButNotSensitive() {
        assertFalse(EditorInputType.isSensitive(TEXT_URI))
        assertTrue(EditorInputType.isContentExtractionExcluded(TEXT_URI))
    }
}

class BubbleVisibilityPolicyTest {
    private fun eval(
        inputType: Int = TEXT,
        imeOptions: Int = 0,
        pkg: String? = "com.example.notes",
        nodePassword: Boolean? = false,
        imeVisible: Boolean = true,
        replyContext: Boolean = false,
        userDenylist: Set<String> = emptySet(),
        dataSensitive: Boolean = false,
    ) = BubbleVisibilityPolicy.evaluate(
        BubbleVisibilityPolicy.Input(inputType, imeOptions, pkg, nodePassword, imeVisible, replyContext, dataSensitive),
        userDenylist,
    )

    @Test fun plainFieldWithKeyboardShowsBubble() {
        val d = eval()
        assertTrue(d.show)
        assertNull(d.override)
    }

    @Test fun keyboardHiddenHidesBubble() = assertFalse(eval(imeVisible = false).show)

    @Test fun passwordFieldHidesBubble() = assertFalse(eval(inputType = TEXT_PASSWORD).show)

    @Test fun nodeReportedPasswordHidesBubble() = assertFalse(eval(nodePassword = true).show)

    @Test fun numberFieldHidesBubble() = assertFalse(eval(inputType = NUMBER).show)

    @Test fun bankingAppHidesBubbleEvenForPlainText() = assertFalse(eval(pkg = "com.chase.sig.android").show)

    @Test fun userDenylistHidesBubble() = assertFalse(eval(userDenylist = setOf("com.example.notes")).show)

    @Test fun accessibilityDataSensitiveHidesBubble() = assertFalse(eval(dataSensitive = true).show)

    @Test fun notificationReplyOverridesSensitiveType() {
        val d = eval(inputType = NUMBER, pkg = "com.android.systemui", replyContext = true)
        assertTrue(d.show)
        assertEquals(BubbleVisibilityPolicy.Override.NotificationReply, d.override)
    }

    @Test fun uriFieldGetsUrlBarOverride() =
        assertEquals(BubbleVisibilityPolicy.Override.UrlBar, eval(inputType = TEXT_URI).override)

    @Test fun webEditTextGetsUrlBarOverride() =
        assertEquals(BubbleVisibilityPolicy.Override.UrlBar, eval(inputType = TEXT_WEB_EDIT).override)

    @Test fun browserGoActionGetsUrlBarOverride() =
        assertEquals(BubbleVisibilityPolicy.Override.UrlBar, eval(pkg = "com.android.chrome", imeOptions = IME_ACTION_GO).override)

    @Test fun cloneDeviationBrowserGoNeverUnhidesPassword() =
        assertFalse(eval(inputType = TEXT_WEB_PASSWORD, pkg = "com.android.chrome", imeOptions = IME_ACTION_GO).show)

    @Test fun searchActionIsSearchField() = assertTrue(eval(imeOptions = IME_ACTION_SEARCH).isSearch)

    @Test fun filterVariationIsSearchField() = assertTrue(eval(inputType = TEXT_FILTER).isSearch)

    @Test fun urlBarIsNeverSearchField() =
        assertFalse(eval(inputType = TEXT_URI, imeOptions = IME_ACTION_SEARCH).isSearch)

    @Test fun replyContextFromSystemUiPackage() =
        assertTrue(BubbleVisibilityPolicy.isNotificationReplyContext("com.android.systemui", imeWindowPresent = false, focusedAppWindowPresent = true))

    @Test fun replyContextFromImeWithoutFocusedAppWindow() =
        assertTrue(BubbleVisibilityPolicy.isNotificationReplyContext("com.foo", imeWindowPresent = true, focusedAppWindowPresent = false))

    @Test fun normalAppIsNotReplyContext() =
        assertFalse(BubbleVisibilityPolicy.isNotificationReplyContext("com.foo", imeWindowPresent = true, focusedAppWindowPresent = true))
}

class AppCategorizerTest {
    @Test fun gmailIsEmail() = assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("com.google.android.gm", null))
    @Test fun slackIsWork() = assertEquals(AppCategorizer.Category.WORK, AppCategorizer.categorize("com.Slack", null))
    @Test fun whatsappIsPersonal() = assertEquals(AppCategorizer.Category.PERSONAL, AppCategorizer.categorize("com.whatsapp", null))
    @Test fun unknownIsOther() = assertEquals(AppCategorizer.Category.OTHER, AppCategorizer.categorize("com.example", null))
    @Test fun browserHostStripsWwwAndMatchesSubdomains() {
        assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("com.android.chrome", "www.outlook.office.com"))
        assertEquals(AppCategorizer.Category.WORK, AppCategorizer.categorize("com.android.chrome", "myteam.slack.com"))
        assertEquals(AppCategorizer.Category.PERSONAL, AppCategorizer.categorize("com.android.chrome", "old.reddit.com"))
    }
    @Test fun mailHostRuleIsEmail() {
        assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("org.mozilla.firefox", "mail.example.org"))
        assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("org.mozilla.firefox", "webmail.example.org"))
        assertEquals(AppCategorizer.Category.OTHER, AppCategorizer.categorize("org.mozilla.firefox", "mailchimp.example.org"))
    }
    @Test fun mailNamePartIsEmail() {
        assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("com.example.android.mail", null))
        assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("com.example.email.provider", null))
        assertEquals(AppCategorizer.Category.OTHER, AppCategorizer.categorize("com.example.dailymail", null))
    }
    @Test fun familiesCoverVariants() {
        assertEquals(AppCategorizer.Category.PERSONAL, AppCategorizer.categorize("com.whatsapp.w4b", null))
        assertEquals(AppCategorizer.Category.PERSONAL, AppCategorizer.categorize("org.telegram.messenger.web", null))
        assertEquals(AppCategorizer.Category.WORK, AppCategorizer.categorize("com.google.android.apps.docs.editors.sheets", null))
        assertEquals(AppCategorizer.Category.WORK, AppCategorizer.categorize("com.atlassian.android.jira.core", null))
        // Outlook sits in the Office family but email wins.
        assertEquals(AppCategorizer.Category.EMAIL, AppCategorizer.categorize("com.microsoft.office.outlook", null))
    }
    @Test fun coreCategoryNames() {
        assertEquals("email", AppCategorizer.Category.EMAIL.coreName)
        assertEquals("work_chat", AppCategorizer.Category.WORK.coreName)
        assertEquals("personal_chat", AppCategorizer.Category.PERSONAL.coreName)
        assertEquals("other", AppCategorizer.Category.OTHER.coreName)
    }
}
