package dev.murmur.bubble

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** A screen of ours made for dictation (a note editor): the bubble stays available over it. */
interface DictationSurface

/**
 * Is one of our own screens in front? Fed by the app's activity lifecycle (started / stopped),
 * so it holds whatever the bubble is doing and whatever the accessibility snapshot says. While
 * [inFront] the bubble stays off our screens ([BubblePresence.visible]), except over a
 * [DictationSurface]. A screen started in split screen is not counted: the other half is an app
 * the user may be dictating into.
 *
 * Started rather than resumed: going from one of our screens to the next starts the new one
 * before the old one stops, so [inFront] does not blink off between pages.
 */
class OwnScreens {
    private val screens = HashMap<Any, Boolean>() // screen → is a dictation surface
    private val _inFront = MutableStateFlow(false)
    val inFront: StateFlow<Boolean> = _inFront

    fun started(screen: Any, surface: Boolean, splitScreen: Boolean) {
        if (splitScreen) return
        screens[screen] = surface
        publish()
    }

    fun stopped(screen: Any) {
        if (screens.remove(screen) != null) publish()
    }

    private fun publish() {
        _inFront.value = screens.isNotEmpty() && screens.values.none { it }
    }
}
