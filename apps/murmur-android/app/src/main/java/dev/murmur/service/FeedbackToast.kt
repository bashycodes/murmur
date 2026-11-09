package dev.murmur.service

import dev.murmur.R
import dev.murmur.session.FeedbackEvent

/** The toast a dictation [FeedbackEvent] shows, if any; [long] = Toast.LENGTH_LONG. */
data class FeedbackToast(val res: Int, val long: Boolean = false) {
    companion object {
        /**
         * Only events the bubble does not already show get a toast: TargetChanged has the ✓ / ⧉ / dismiss row,
         * MicLost goes straight on to transcribe what was heard.
         */
        fun of(e: FeedbackEvent): FeedbackToast? = when (e) {
            FeedbackEvent.CapWarning -> FeedbackToast(R.string.toast_cap_warning)
            FeedbackEvent.CapReached -> FeedbackToast(R.string.toast_cap_reached)
            FeedbackEvent.NeedMicPermission -> FeedbackToast(R.string.toast_need_mic)
            FeedbackEvent.PasteRetryFailed -> FeedbackToast(R.string.toast_paste_failed)
            FeedbackEvent.TargetProtected -> FeedbackToast(R.string.toast_target_protected, long = true)
            FeedbackEvent.StartSound, FeedbackEvent.StopSound, FeedbackEvent.CancelHaptic, FeedbackEvent.NoSpeech,
            FeedbackEvent.TargetChanged, FeedbackEvent.MicLost, FeedbackEvent.CommandFailed -> null
        }
    }
}
