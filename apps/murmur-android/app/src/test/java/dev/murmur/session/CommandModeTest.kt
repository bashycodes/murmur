package dev.murmur.session

import dev.murmur.command.CommandRecord
import dev.murmur.command.CommandTarget
import dev.murmur.command.SelectionSnapshot
import dev.murmur.core.CommandOutcome
import dev.murmur.core.CommitResult
import dev.murmur.core.FakeMurmurCore
import dev.murmur.core.FieldContext
import dev.murmur.core.SessionConfig
import dev.murmur.debug.DictationTrace
import dev.murmur.debug.InsertOutcome
import dev.murmur.session.DictationState.ErrorKind
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.currentTime
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Command Mode in the controller: instruction → AI → selection replaced, or nothing touched. */
@OptIn(ExperimentalCoroutinesApi::class)
class CommandModeTest {
    private class FakeAudio : AudioSource {
        var sink: ((ShortArray, Int) -> Unit)? = null
        override fun start(onFrame: (ShortArray, Int) -> Unit): Boolean { sink = onFrame; return true }
        override fun stop() { sink = null }
        fun speak(ms: Int = 1000) { repeat(ms / 100) { sink?.invoke(ShortArray(1600) { 3000 }, 1600) } }
    }

    private class Host : DictationController.Host {
        val events = mutableListOf<FeedbackEvent>()
        val inserted = mutableListOf<String>()
        val history = mutableListOf<CommitResult>()
        var contextCaptures = 0
        override fun feedback(e: FeedbackEvent) { events += e }
        override suspend fun insert(text: String): Boolean { inserted += text; return true }
        override fun recordHistory(result: CommitResult, inserted: Boolean, appId: String?, outcome: String?) { history += result }
        override fun hasMicPermission() = true
        override fun captureContext(): FieldContext { contextCaptures++; return FieldContext(appId = "com.example.notes") }
        override fun sessionConfig() = SessionConfig(languages = listOf("en"))
        override fun onRecordingActive(active: Boolean) = Unit
        override fun endpointSilenceMs(): Int? = 2000
        var target = InsertTarget.OK
        override fun insertTarget(explicit: Boolean) = target

        var command: CommandTarget = CommandTarget.Field("com.example.notes", SelectionSnapshot.of("Hi team. A very long sentence. Bye", 9, 30))
        override fun captureCommand() = command
        var applyOk = true
        val applied = mutableListOf<Pair<CommandTarget.Field, String>>()
        override suspend fun applyCommand(target: CommandTarget.Field, text: String): Boolean { applied += target to text; return applyOk }
        val records = mutableListOf<CommandRecord>()
        override fun recordCommand(record: CommandRecord) { records += record }
        val notInserted = mutableListOf<Pair<String, String>>()
        override fun onNotInserted(text: String, outcome: String) { notInserted += text to outcome }
        val traces = mutableListOf<DictationTrace>()
        override fun onDictationTrace(trace: DictationTrace) { traces += trace }
    }

    private val audio = FakeAudio()
    private val host = Host()
    private val core = FakeMurmurCore(transcript = "make it shorter")

    private fun TestScope.settle() { advanceTimeBy(1_000); runCurrent() }

    private fun TestScope.controller() = DictationController(
        scope = backgroundScope, audio = audio, host = host, core = { core },
        commitDispatcher = StandardTestDispatcher(testScheduler), now = { currentTime },
    ).also { it.onFieldFocusChanged(true) }

    private fun TestScope.speakCommand(c: DictationController) {
        c.toggleCommand()
        audio.speak()
        advanceTimeBy(600)
        c.toggleCommand()
        settle()
    }

    @Test fun anInstructionReplacesTheSelectionWithTheRewrite() = runTest {
        core.commandReply = { _, sel -> CommandOutcome.Text("Short.") }
        val c = controller()
        c.toggleCommand()
        val rec = c.state.value as DictationState.Recording
        assertTrue("a command is hands-free", rec.command && !rec.hold)
        assertTrue("instruction is transcribed raw", core.lastConfig!!.rawMode)
        assertNull("the field's text is not dictation context", core.lastContext)
        assertEquals(0, host.contextCaptures)
        audio.speak(); advanceTimeBy(600)
        c.toggleCommand()
        assertTrue((c.state.value as DictationState.Processing).command)
        settle()
        assertEquals(listOf("make it shorter" to "A very long sentence."), core.commands)
        assertEquals("Short.", host.applied.single().second)
        assertEquals(DictationState.Ready, c.state.value)
        assertEquals(listOf<String>(), host.inserted)
        assertEquals("commands are not dictation history", 0, host.history.size)
        assertEquals(
            CommandRecord("make it shorter", "A very long sentence.", "Short.", "com.example.notes", InsertOutcome.INSERTED),
            host.records.single(),
        )
        assertEquals("command", host.traces.single().mode)
    }

    @Test fun withoutASelectionTheInstructionGeneratesAtTheCursor() = runTest {
        host.command = CommandTarget.Field("com.example.notes", SelectionSnapshot.of("Hi ", 3, 3))
        core.commandReply = { _, _ -> CommandOutcome.Text("Dear team,") }
        val c = controller()
        speakCommand(c)
        assertEquals(listOf("make it shorter" to null), core.commands)
        assertEquals("Dear team,", host.applied.single().second)
        assertNull(host.records.single().original)
    }

    @Test fun aProtectedFieldIsRefusedBeforeRecording() = runTest {
        host.command = CommandTarget.Refused(CommandTarget.PROTECTED_MESSAGE)
        val c = controller()
        c.toggleCommand()
        assertEquals(DictationState.Error(ErrorKind.CommandFailed(CommandTarget.PROTECTED_MESSAGE)), c.state.value)
        assertEquals(listOf(FeedbackEvent.CommandFailed), host.events)
        assertNull("no session, no audio", audio.sink)
        assertNull(core.lastConfig)
        assertTrue(core.commands.isEmpty())
    }

    @Test fun noAiLeavesTheTextUntouchedWithAClearMessage() = runTest {
        val c = controller() // FakeMurmurCore without a reply = no LLM
        speakCommand(c)
        assertEquals(DictationState.Error(ErrorKind.CommandFailed(CommandOutcome.NO_LLM_MESSAGE)), c.state.value)
        assertTrue(FeedbackEvent.CommandFailed in host.events)
        assertTrue("nothing written", host.applied.isEmpty() && host.inserted.isEmpty())
        val r = host.records.single()
        assertEquals(InsertOutcome.COMMAND_FAILED, r.outcome)
        assertNull(r.result)
        assertEquals(CommandOutcome.NO_LLM_MESSAGE, r.error)
        assertEquals(InsertOutcome.COMMAND_FAILED, host.traces.single().insert.outcome)
    }

    @Test fun aHungAiTimesOutAndTouchesNothing() = runTest {
        val gate = java.util.concurrent.CountDownLatch(1)
        core.commandReply = { _, _ -> gate.await(); CommandOutcome.Text("late") }
        val c = DictationController(
            scope = backgroundScope, audio = audio, host = host, core = { core },
            commitDispatcher = kotlinx.coroutines.Dispatchers.IO, now = { currentTime },
        ).also { it.onFieldFocusChanged(true) }
        c.toggleCommand()
        audio.speak(); advanceTimeBy(600)
        c.toggleCommand()
        // Real threads for the blocking calls; virtual time for the timeout.
        repeat(50) { if (core.commands.isEmpty()) Thread.sleep(20); runCurrent() }
        advanceTimeBy(DictationController.COMMAND_TIMEOUT_MS + 1); runCurrent()
        gate.countDown()
        val s = c.state.value as DictationState.Error
        assertTrue(s.toString(), (s.kind as ErrorKind.CommandFailed).message.startsWith("AI didn't answer"))
        assertTrue(host.applied.isEmpty())
    }

    @Test fun anEmptyInstructionIsNoSpeechAndAsksNothing() = runTest {
        core.transcript = ""
        val c = controller()
        speakCommand(c)
        assertTrue(core.commands.isEmpty())
        assertTrue(FeedbackEvent.NoSpeech in host.events)
        assertEquals(DictationState.Ready, c.state.value)
        assertTrue(host.history.isEmpty() && host.records.isEmpty())
    }

    @Test fun focusMovedDuringTheCommandWritesNothing() = runTest {
        core.commandReply = { _, _ -> CommandOutcome.Text("Short.") }
        val c = controller()
        c.toggleCommand()
        audio.speak(); advanceTimeBy(600)
        host.target = InsertTarget.CHANGED
        c.toggleCommand()
        settle()
        assertTrue(host.applied.isEmpty())
        assertEquals(DictationState.Error(ErrorKind.FailedToPaste("Short.")), c.state.value)
        assertEquals(listOf("Short." to InsertOutcome.TARGET_CHANGED), host.notInserted)
    }

    @Test fun aQuestionAboutTheSelectionShowsTheAnswerAndTouchesNothing() = runTest {
        core.transcript = "what does this mean"
        core.commandReply = { _, _ -> CommandOutcome.Answer("It describes a long sentence.") }
        val c = controller()
        speakCommand(c)
        assertEquals(listOf("what does this mean" to "A very long sentence."), core.commands)
        assertEquals(DictationState.Error(ErrorKind.CommandAnswer("It describes a long sentence.")), c.state.value)
        assertTrue("nothing written", host.applied.isEmpty() && host.inserted.isEmpty())
        assertTrue("not a failure: no error tone", FeedbackEvent.CommandFailed !in host.events)
        assertTrue("no not-inserted notification", host.notInserted.isEmpty())
        assertEquals(
            CommandRecord("what does this mean", "A very long sentence.", "It describes a long sentence.", "com.example.notes", InsertOutcome.ANSWERED),
            host.records.single(),
        )
        assertEquals(InsertOutcome.ANSWERED, host.traces.single().insert.outcome)
        c.dismissError()
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun aStaleSelectionKeepsTheAnswerRecoverable() = runTest {
        core.commandReply = { _, _ -> CommandOutcome.Text("Short.") }
        host.applyOk = false
        val c = controller()
        speakCommand(c)
        assertEquals(DictationState.Error(ErrorKind.FailedToPaste("Short.")), c.state.value)
        assertTrue(FeedbackEvent.CommandFailed in host.events)
        assertEquals(InsertOutcome.PASTE_FAILED, host.records.single().outcome)
        assertEquals(listOf("Short." to InsertOutcome.PASTE_FAILED), host.notInserted)
    }

    @Test fun cancellingWhileTheAiWorksDropsTheAnswer() = runTest {
        core.commandReply = { _, _ -> CommandOutcome.Text("Short.") }
        val c = controller()
        c.toggleCommand()
        audio.speak(); advanceTimeBy(600)
        c.toggleCommand()
        assertTrue(c.state.value is DictationState.Processing)
        c.cancel()
        settle()
        assertTrue(host.applied.isEmpty())
        assertEquals(DictationState.Ready, c.state.value)
    }

    @Test fun aNormalDictationAfterACommandIsNotACommand() = runTest {
        core.commandReply = { _, _ -> CommandOutcome.Text("Short.") }
        val c = controller()
        speakCommand(c)
        advanceTimeBy(600)
        c.toggle(hold = false)
        assertTrue(!(c.state.value as DictationState.Recording).command)
        assertTrue(!core.lastConfig!!.rawMode)
        audio.speak(); advanceTimeBy(600)
        c.toggle(hold = false)
        settle()
        assertEquals(listOf("make it shorter"), host.inserted)
        assertEquals(1, host.applied.size)
    }
}
