package dev.murmur.session

import dev.murmur.core.CommitResult
import dev.murmur.core.FakeMurmurCore
import dev.murmur.core.FieldContext
import dev.murmur.core.SessionConfig
import dev.murmur.core.TranslateOutcome
import dev.murmur.data.showsTranslate
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** D90 Translate button in the controller: dictation → one AI call → the translation is typed. */
@OptIn(ExperimentalCoroutinesApi::class)
class TranslateModeTest {
    private class FakeAudio : AudioSource {
        var sink: ((ShortArray, Int) -> Unit)? = null
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean { sink = onFrame; return true }
        override fun stop() { sink = null }
        fun speak(ms: Int = 1000) { repeat(ms / 100) { sink?.invoke(ShortArray(1600) { 3000 }, 1600) } }
    }

    private class Host : DictationController.Host {
        val inserted = mutableListOf<String>()
        val history = mutableListOf<CommitResult>()
        val learned = mutableListOf<String>()
        var target: String? = null
        override fun feedback(e: FeedbackEvent) = Unit
        override suspend fun insert(text: String): Boolean { inserted += text; return true }
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) { history += result }
        override fun hasMicPermission() = true
        override fun captureContext() = FieldContext(appId = "com.example.notes")
        override fun sessionConfig() = SessionConfig(languages = listOf("en"))
        override fun onRecordingActive(active: Boolean) = Unit
        override fun endpointSilenceMs(): Int? = 2000
        override fun insertTarget(explicit: Boolean) = InsertTarget.OK
        override fun onInserted(pasted: String, context: FieldContext?, sentKey: Boolean) { learned += pasted }
        override fun translateTarget() = target
        override fun keyboardLanguages() = listOf("he", "en")
    }

    private val audio = FakeAudio()
    private val host = Host()
    private val core = FakeMurmurCore(transcript = "the demo is on thursday")

    private fun TestScope.controller() = DictationController(
        scope = backgroundScope, audio = audio, host = host, core = { core },
        commitDispatcher = StandardTestDispatcher(testScheduler), now = { currentTime },
    ).also { it.onFieldFocusChanged(true) }

    private fun TestScope.speakTranslated(c: DictationController) {
        c.toggleTranslate()
        audio.speak()
        advanceTimeBy(600)
        c.toggleTranslate()
        advanceTimeBy(1_000); runCurrent()
    }

    @Test fun theTranslationIsTypedInsteadOfTheDictation() = runTest {
        core.translateReply = { _, _ -> TranslateOutcome.Text("translated words", "en") }
        val c = controller()
        c.toggleTranslate()
        val rec = c.state.value as DictationState.Recording
        assertTrue("hands-free, and a dictation (not a command)", !rec.hold && !rec.command)
        assertTrue("one AI call cleans and translates", core.lastConfig!!.rawMode)
        audio.speak(); advanceTimeBy(600)
        c.toggleTranslate()
        advanceTimeBy(1_000); runCurrent()
        val call = core.translations.single()
        assertTrue(call[0].toString(), call[0].toString().contains("demo"))
        assertEquals("automatic target", null, call[2])
        assertEquals(listOf("he", "en"), call[3])
        assertEquals(listOf("translated words"), host.inserted)
        val saved = host.history.single()
        assertTrue("History keeps the dictation", saved.historyText.contains("demo"))
        assertEquals(dev.murmur.core.AppliedTransform("Translate to English", "translated words"), saved.applied)
        assertEquals(null, c.hint.value)
        assertTrue("the AI's wording is not learned from", host.learned.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun theChosenTargetIsSent() = runTest {
        host.target = "fr"
        core.translateReply = { _, t -> TranslateOutcome.Text("des mots en $t") }
        val c = controller()
        speakTranslated(c)
        assertEquals("fr", core.translations.single()[2])
        assertEquals(listOf("des mots en fr"), host.inserted)
    }

    @Test fun anOnDeviceTranslationIsTypedAndSaysItsLowerQuality() = runTest {
        core.translateReply = { _, _ -> TranslateOutcome.Text("translated words", "en", onDevice = true) }
        val c = controller()
        speakTranslated(c)
        assertEquals(listOf("translated words"), host.inserted)
        assertEquals(DictationHint.TranslatedOnDevice, c.hint.value)
        assertEquals(
            dev.murmur.bubble.PreviewContent.TRANSLATED_ON_DEVICE,
            dev.murmur.bubble.PreviewContent.line(DictationState.Ready, null, c.hint.value, enabled = false)?.text,
        )
    }

    @Test fun theButtonShowsByItselfOnlyWithTwoKeyboardLanguages() {
        val auto = dev.murmur.data.Settings()
        assertFalse(auto.showsTranslate(1))
        assertTrue(auto.showsTranslate(2))
        assertFalse("the setting overrides", auto.copy(translateButton = false).showsTranslate(2))
        assertTrue(auto.copy(translateButton = true).showsTranslate(0))
    }

    @Test fun aFailedTranslationTypesTheDictationAndSaysSo() = runTest {
        val c = controller() // no reply = no AI
        speakTranslated(c)
        assertEquals(1, host.inserted.size)
        assertTrue(host.inserted.single(), host.inserted.single().contains("demo"))
        assertEquals(DictationHint.NotTranslated(TranslateOutcome.NO_LLM), c.hint.value)
        assertEquals(dev.murmur.bubble.PreviewContent.NO_AI_FOR_TRANSLATION, bar(c))
        assertEquals(DictationState.Ready, c.state.value)
        assertFalse("the dictation itself may be learned from", host.learned.isEmpty())
    }

    private fun bar(c: DictationController) =
        dev.murmur.bubble.PreviewContent.line(DictationState.Ready, null, c.hint.value, enabled = false)?.text

    @Test fun aWrongScriptAnswerSaysWhichLanguageFailed() = runTest {
        core.translateReply = { _, _ -> TranslateOutcome.Failed(TranslateOutcome.NOT_TRANSLATED, "Couldn't translate to Hebrew", "he") }
        val c = controller()
        speakTranslated(c)
        assertTrue("typed as said", host.inserted.single().contains("demo"))
        assertEquals("Couldn't translate to Hebrew. Typed as said.", bar(c))
    }

    @Test fun anOlderLibraryWithoutTheTargetNamesTheChosenOne() = runTest {
        host.target = "fr"
        core.translateReply = { _, _ -> TranslateOutcome.Failed(TranslateOutcome.NOT_TRANSLATED, "Couldn't translate the text") }
        val c = controller()
        speakTranslated(c)
        assertEquals("Couldn't translate to French. Typed as said.", bar(c))
    }

    @Test fun aTimeoutSaysSo() = runTest {
        core.translateReply = { _, _ -> TranslateOutcome.Failed(TranslateOutcome.TIMEOUT, "AI didn't translate within 25 s") }
        val c = controller()
        speakTranslated(c)
        assertTrue("typed as said", host.inserted.single().contains("demo"))
        assertEquals(dev.murmur.bubble.PreviewContent.TRANSLATION_TIMED_OUT, bar(c))
    }

    @Test fun anyOtherFailureKeepsTheGeneralText() = runTest {
        core.translateReply = { _, _ -> TranslateOutcome.Failed(TranslateOutcome.BACKEND, "Translation failed: connect_failed") }
        val c = controller()
        speakTranslated(c)
        assertEquals("Not translated. Typed as said.", bar(c))
    }

    @Test fun aPlainDictationIsNeverTranslated() = runTest {
        core.translateReply = { _, _ -> TranslateOutcome.Text("translated words") }
        val c = controller()
        c.toggle(hold = false)
        audio.speak(); advanceTimeBy(600)
        c.toggle(hold = false)
        advanceTimeBy(1_000); runCurrent()
        assertTrue(core.translations.isEmpty())
        assertFalse(core.lastConfig!!.rawMode)
    }
}
