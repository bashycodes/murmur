package dev.murmur.bubble

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import java.util.WeakHashMap

/** A screen of ours made for dictation (a note editor): the bubble works over it like over any app. */
interface DictationScreen

/**
 * Which of our own screens are in front, fed by the app's activity lifecycle. The one source for
 * "is Murmur in front": [Front.hidesBubble] keeps the bubble off our screens
 * ([BubblePresence.visible]); [Front.dictationScreen] lets the accessibility service treat our
 * dictation screen's field like any other app's ([dev.murmur.policy.OwnAppGate]).
 *
 * - Started to stopped, not resumed to paused: the next page of ours starts before the last one
 *   stops, so moving between pages never blinks the bubble.
 * - A screen in split screen or a free-form window does not hide the bubble (the other window
 *   may be the app being dictated into). Its mode is re-read on every [refresh] (resume, pause,
 *   configuration change), so entering or leaving split screen in place takes effect.
 * - A screen stopped to be recreated (rotation) hands its place to the next screen that shows,
 *   so the gap between the old and the new instance doesn't flash the bubble.
 * - Screens are held weakly: a stop that never comes cannot keep an Activity alive.
 */
class OwnScreens {
    /** One screen of ours: made for dictation, in split screen / a free-form window. */
    data class Screen(val dictation: Boolean, val split: Boolean)

    data class Front(val hidesBubble: Boolean = false, val dictationScreen: Boolean = false)

    private val screens = WeakHashMap<Any, Screen>()
    private var handover: Screen? = null
    private val _front = MutableStateFlow(Front())
    val front: StateFlow<Front> = _front

    /** [key] started (or is being re-read): it counts as [screen] from now on. */
    fun started(key: Any, screen: Screen) {
        screens[key] = screen
        handover = null
        publish()
    }

    /** Re-read a started screen (split screen may have changed); ignored once it stopped. */
    fun refresh(key: Any, screen: Screen) {
        if (key in screens) started(key, screen)
    }

    /** [key] stopped; [recreating] = it stops only to come back as a new instance (rotation). */
    fun stopped(key: Any, recreating: Boolean) {
        val s = screens.remove(key) ?: return
        if (recreating) handover = s
        publish()
    }

    private fun publish() {
        val all = screens.values + listOfNotNull(handover)
        val full = all.filter { !it.split }
        _front.value = Front(
            hidesBubble = full.isNotEmpty() && full.none { it.dictation },
            dictationScreen = all.any { it.dictation },
        )
    }
}
