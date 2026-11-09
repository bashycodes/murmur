package dev.flowclone.service

import dev.flowclone.R
import dev.flowclone.session.FeedbackEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FeedbackToastTest {
    /** The bubble already says these another way (✓ / ⧉ / dismiss row; transcription goes on), so no toast. */
    @Test fun eventsTheBubbleAlreadyShowsHaveNoToast() {
        assertNull(FeedbackToast.of(FeedbackEvent.TargetChanged))
        assertNull(FeedbackToast.of(FeedbackEvent.MicLost))
    }

    @Test fun soundsAndHapticsHaveNoToast() {
        listOf(FeedbackEvent.StartSound, FeedbackEvent.StopSound, FeedbackEvent.CancelHaptic, FeedbackEvent.NoSpeech)
            .forEach { assertNull(it.name, FeedbackToast.of(it)) }
    }

    @Test fun errorsKeepTheirToast() {
        assertEquals(FeedbackToast(R.string.toast_cap_warning), FeedbackToast.of(FeedbackEvent.CapWarning))
        assertEquals(FeedbackToast(R.string.toast_cap_reached), FeedbackToast.of(FeedbackEvent.CapReached))
        assertEquals(FeedbackToast(R.string.toast_need_mic), FeedbackToast.of(FeedbackEvent.NeedMicPermission))
        assertEquals(FeedbackToast(R.string.toast_paste_failed), FeedbackToast.of(FeedbackEvent.PasteRetryFailed))
        assertEquals(FeedbackToast(R.string.toast_target_protected, long = true), FeedbackToast.of(FeedbackEvent.TargetProtected))
    }
}
