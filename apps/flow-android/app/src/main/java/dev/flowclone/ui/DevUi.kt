package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.InsetDrawable
import android.graphics.drawable.RippleDrawable
import android.text.Editable
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.EditText
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView

/**
 * Settings look shared by main Settings (SettingsActivity) and Developer settings
 * (DeveloperActivity); platform views only, no UI deps: grouped rounded cards on a plain
 * background, rows with a title + grey value subtitle, switch rows with a one-line description,
 * labelled fields with inline errors, full-width actions with their result right under them.
 * Follows the app's DayNight theme (dark on a dark system), 48 dp+ touch targets, system insets.
 */
internal class DevPalette(
    val bg: Int, val surface: Int, val field: Int, val text: Int, val muted: Int, val divider: Int,
    val accent: Int, val onAccent: Int, val tonal: Int, val danger: Int, val ok: Int, val warnBg: Int, val dangerBg: Int,
) {
    companion object {
        fun of(ctx: Context): DevPalette {
            val night = ctx.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK == Configuration.UI_MODE_NIGHT_YES
            fun c(s: String) = Color.parseColor(s)
            return if (night) DevPalette(
                bg = c("#FF0F0F10"), surface = c("#FF1E1E20"), field = c("#FF2A2A2D"), text = c("#FFE6E1E5"), muted = c("#FF9E9AA3"),
                divider = c("#FF2F2F33"), accent = c("#FFD0BCFF"), onAccent = c("#FF381E72"), tonal = c("#FF2B2738"),
                danger = c("#FFF2B8B5"), ok = c("#FF8BD3A0"), warnBg = c("#FF3A3220"), dangerBg = c("#FF3B1F1E"),
            ) else DevPalette(
                bg = c("#FFF3F2F7"), surface = c("#FFFFFFFF"), field = c("#FFF0EFF4"), text = c("#FF1C1B1F"), muted = c("#FF6B6870"),
                divider = c("#FFE6E4EA"), accent = c("#FF6750A4"), onAccent = c("#FFFFFFFF"), tonal = c("#FFEADDFF"),
                danger = c("#FFB3261E"), ok = c("#FF2E7D32"), warnBg = c("#FFFFF4D6"), dangerBg = c("#FFFCE8E6"),
            )
        }
    }
}

internal enum class Tone { NEUTRAL, OK, BAD, WARN }

/** Builder for one Settings / Developer settings screen. */
internal class DevUi(val activity: Activity) {
    val ctx: Context get() = activity
    val p = DevPalette.of(activity)
    private fun dp(v: Int) = (v * ctx.resources.displayMetrics.density).toInt()
    private fun dpf(v: Int) = v * ctx.resources.displayMetrics.density

    private fun rounded(color: Int, radiusDp: Int = 20) = GradientDrawable().apply { setColor(color); cornerRadius = dpf(radiusDp) }

    private fun ripple(content: android.graphics.drawable.Drawable?, radiusDp: Int = 0): RippleDrawable {
        val mask = if (radiusDp > 0) rounded(Color.WHITE, radiusDp) else ColorDrawable(Color.WHITE)
        return RippleDrawable(ColorStateList.valueOf((p.text and 0x00FFFFFF) or 0x22000000), content, mask)
    }

    /** Scrolling column on the page background; pads itself by the system bars, cutout and keyboard. */
    fun screen(build: LinearLayout.() -> Unit): LinearLayout {
        val col = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            build()
        }
        val scroll = ScrollView(ctx).apply {
            setBackgroundColor(p.bg)
            isFillViewport = true
            clipToPadding = false
            addView(col, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            setOnApplyWindowInsetsListener { v, insets ->
                val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                v.setPadding(i.left + dp(16), i.top + dp(8), i.right + dp(16), i.bottom + dp(24))
                insets
            }
        }
        activity.window.decorView.setBackgroundColor(p.bg)
        activity.setContentView(scroll)
        scroll.requestApplyInsets()
        return col
    }

    // ---- text ----

    fun LinearLayout.sectionLabel(text: String) = TextView(ctx).apply {
        this.text = text; textSize = 13f; setTextColor(p.muted); setTypeface(typeface, Typeface.BOLD)
        letterSpacing = 0.04f
        setPadding(dp(16), dp(20), dp(16), dp(8))
        isAccessibilityHeading = true
    }.also(::addView)

    fun LinearLayout.note(text: CharSequence, color: Int = p.muted, sizeSp: Float = 13f) = TextView(ctx).apply {
        this.text = text; textSize = sizeSp; setTextColor(color)
        setPadding(dp(16), dp(6), dp(16), dp(6))
    }.also(::addView)

    // ---- cards ----

    /** Rounded group; children are separated by inset hairlines. */
    fun LinearLayout.card(build: LinearLayout.() -> Unit): LinearLayout = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(p.surface)
        clipToOutline = true
        dividerDrawable = InsetDrawable(GradientDrawable().apply { setColor(p.divider); setSize(1, maxOf(1, dp(1) / 2 + 1)) }, dp(16), 0, 0, 0)
        showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        build()
    }.also { addView(it, lp(top = 4, bottom = 8)) }

    private fun lp(top: Int = 0, bottom: Int = 0) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
        topMargin = dp(top); bottomMargin = dp(bottom)
    }

    /** Card header: bold title with a grey counter / hint on the right (e.g. "2 of 3"). */
    fun LinearLayout.cardHeader(title: String, trailing: String = ""): TextView {
        val t = TextView(ctx).apply { text = title; textSize = 16f; setTextColor(p.text); setTypeface(typeface, Typeface.BOLD); isAccessibilityHeading = true }
        val r = TextView(ctx).apply { text = trailing; textSize = 13f; setTextColor(p.muted); gravity = Gravity.END }
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(16), dp(14), dp(16), dp(6))
            addView(t, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(r)
        })
        return r
    }

    private fun icon(res: Int, tint: Int, sizeDp: Int = 24) = ImageView(ctx).apply {
        setImageResource(res); imageTintList = ColorStateList.valueOf(tint)
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        layoutParams = LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp))
    }

    private fun titleSub(title: String, sub: String, titleColor: Int = p.text): Pair<LinearLayout, Pair<TextView, TextView>> {
        val t = TextView(ctx).apply { text = title; textSize = 16f; setTextColor(titleColor) }
        val s = TextView(ctx).apply {
            text = sub; textSize = 13f; setTextColor(p.muted); visibility = if (sub.isEmpty()) View.GONE else View.VISIBLE
        }
        val col = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL; addView(t); addView(s) }
        return col to (t to s)
    }

    class NavRow(val view: View, val title: TextView, val subtitle: TextView) {
        fun setSubtitle(text: CharSequence) {
            subtitle.text = text; subtitle.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
            view.contentDescription = listOf(title.text, text).filter { it.isNotEmpty() }.joinToString(", ")
        }
    }

    /** Icon + title + current value + chevron; the whole row is one touch target / a11y node. */
    fun LinearLayout.navRow(iconRes: Int?, title: String, subtitle: String = "", danger: Boolean = false, chevron: Boolean = true, onClick: () -> Unit): NavRow {
        val (texts, ts) = titleSub(title, subtitle, if (danger) p.danger else p.text)
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(64)
            setPadding(dp(16), dp(12), dp(12), dp(12))
            background = ripple(null)
            isClickable = true; isFocusable = true
            setOnClickListener { onClick() }
            if (iconRes != null) addView(icon(iconRes, if (danger) p.danger else p.accent).apply {
                (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(16)
            })
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            if (chevron) addView(icon(dev.flowclone.R.drawable.ic_dev_chevron, p.muted, 20))
        }
        addView(row)
        return NavRow(row, ts.first, ts.second).also { it.setSubtitle(subtitle) }
    }

    /** A read-only "label / value" row (status values; selectable for copy). */
    fun LinearLayout.infoRow(label: String, value: String = ""): TextView {
        val v = TextView(ctx).apply { text = value; textSize = 14f; setTextColor(p.text); setTextIsSelectable(true) }
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(12), dp(16), dp(12))
            addView(TextView(ctx).apply { text = label; textSize = 13f; setTextColor(p.muted) })
            addView(v)
        })
        return v
    }

    // ---- switches ----

    /** Switch row: title + one-line description; the row toggles and reads as one switch to TalkBack. */
    class ToggleRow(val row: LinearLayout, val sw: Switch, val title: TextView, val desc: TextView) {
        var isChecked: Boolean get() = sw.isChecked; set(v) { sw.isChecked = v }
        var isEnabled: Boolean
            get() = sw.isEnabled
            set(v) { sw.isEnabled = v; row.isEnabled = v; row.alpha = if (v) 1f else 0.5f }
        fun setTitle(t: String) { title.text = t }
        fun setDescription(t: String) { desc.text = t; desc.visibility = if (t.isEmpty()) View.GONE else View.VISIBLE }
        fun onChange(f: (Boolean) -> Unit) = sw.setOnCheckedChangeListener { _, on -> f(on) }
    }

    fun LinearLayout.toggleRow(title: String, description: String, checked: Boolean, enabled: Boolean = true): ToggleRow {
        val (texts, ts) = titleSub(title, description)
        val sw = Switch(ctx).apply {
            isChecked = checked; isClickable = false; isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            thumbTintList = ColorStateList(arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()), intArrayOf(p.accent, p.muted))
            trackTintList = ColorStateList(arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()), intArrayOf((p.accent and 0x00FFFFFF) or 0x66000000, p.field))
        }
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            minimumHeight = dp(64)
            setPadding(dp(16), dp(12), dp(16), dp(12))
            background = ripple(null)
            isClickable = true; isFocusable = true
            setOnClickListener { if (sw.isEnabled) sw.toggle() }
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
            addView(sw)
            accessibilityDelegate = object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.className = Switch::class.java.name
                    info.isCheckable = true; info.isChecked = sw.isChecked; info.isEnabled = sw.isEnabled
                    info.text = listOf(ts.first.text, ts.second.text).filter { it.isNotEmpty() }.joinToString(". ")
                }
            }
        }
        sw.setOnCheckedChangeListener(null)
        addView(row)
        return ToggleRow(row, sw, ts.first, ts.second).also { it.setDescription(description); it.isEnabled = enabled }
    }

    // ---- single choice ----

    class Choice<T>(val value: T, val title: String, val description: String = "")

    /** Radio rows; each row is one touch target and reads to TalkBack as one radio button. */
    class ChoiceGroup<T>(private val rows: List<Pair<T, Pair<View, android.widget.RadioButton>>>) {
        var selected: T? = null; private set
        var onPick: ((T) -> Unit)? = null

        /** Shows [v] as selected without calling [onPick] (initial value, a cancelled change). */
        fun select(v: T) {
            selected = v
            for ((value, r) in rows) r.second.isChecked = value == v
        }

        internal fun pick(v: T) {
            if (!isEnabled || v == selected) return
            select(v); onPick?.invoke(v)
        }

        var isEnabled: Boolean = true
            set(on) { field = on; for ((_, r) in rows) { r.first.isEnabled = on; r.second.isEnabled = on; r.first.alpha = if (on) 1f else 0.5f } }
    }

    fun <T> LinearLayout.choiceRows(choices: List<Choice<T>>, selected: T, onPick: (T) -> Unit): ChoiceGroup<T> {
        val built = ArrayList<Pair<T, Pair<View, android.widget.RadioButton>>>()
        lateinit var group: ChoiceGroup<T>
        for (c in choices) {
            val (texts, ts) = titleSub(c.title, c.description)
            val rb = android.widget.RadioButton(ctx).apply {
                isClickable = false; isFocusable = false
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                buttonTintList = ColorStateList(arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()), intArrayOf(p.accent, p.muted))
            }
            val row = LinearLayout(ctx).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                minimumHeight = dp(56)
                setPadding(dp(8), dp(8), dp(16), dp(8))
                background = ripple(null)
                isClickable = true; isFocusable = true
                setOnClickListener { group.pick(c.value) }
                addView(rb, LinearLayout.LayoutParams(dp(48), dp(48)))
                addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(4) })
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        info.className = android.widget.RadioButton::class.java.name
                        info.isCheckable = true; info.isChecked = rb.isChecked; info.isEnabled = host.isEnabled
                        info.text = listOf(ts.first.text, ts.second.text).filter { it.isNotEmpty() }.joinToString(". ")
                    }
                }
            }
            addView(row)
            built += c.value to (row to rb)
        }
        group = ChoiceGroup(built)
        group.select(selected)
        group.onPick = onPick
        return group
    }

    // ---- fields ----

    class FieldRow(val edit: EditText, val error: TextView, val help: TextView) {
        fun setError(e: String?) { error.text = e.orEmpty(); error.visibility = if (e == null) View.GONE else View.VISIBLE }
        var text: String get() = edit.text.toString(); set(v) { edit.setText(v) }
    }

    /** Label above a borderless input; grey help and a red inline error below it. */
    fun LinearLayout.fieldRow(label: String, value: String, inputType: Int, help: String = "", hint: String = "", minLines: Int = 1, onEdit: () -> Unit): FieldRow {
        val box = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(16), dp(12), dp(16), dp(10)) }
        val l = TextView(ctx).apply { text = label; textSize = 13f; setTextColor(p.muted) }
        val e = EditText(ctx).apply {
            id = View.generateViewId()
            this.inputType = inputType; setText(value)
            if (minLines > 1) { isSingleLine = false; this.minLines = minLines; gravity = Gravity.TOP or Gravity.START } else isSingleLine = true
            this.hint = hint; setHintTextColor((p.muted and 0x00FFFFFF) or 0x99000000.toInt())
            textSize = 16f; setTextColor(p.text)
            background = rounded(p.field, 12)
            setPadding(dp(12), dp(10), dp(12), dp(10))
            minHeight = dp(48)
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun afterTextChanged(s: Editable?) = onEdit()
            })
        }
        l.labelFor = e.id
        val h = TextView(ctx).apply { text = help; textSize = 12f; setTextColor(p.muted); setPadding(0, dp(4), 0, 0); visibility = if (help.isEmpty()) View.GONE else View.VISIBLE }
        val err = TextView(ctx).apply {
            textSize = 13f; setTextColor(p.danger); setPadding(0, dp(4), 0, 0); visibility = View.GONE
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        box.addView(l); box.addView(e, lp(top = 6)); box.addView(h); box.addView(err)
        addView(box)
        return FieldRow(e, err, h)
    }

    /**
     * Secret field (server token / API key): masked, never in the a11y tree ([SecretField]),
     * with Show / Paste / Clear. [onClear] replaces the default "empty the field" when given.
     */
    fun LinearLayout.secretRow(
        label: String, value: String, help: String, what: String, onEdit: () -> Unit,
        clearLabel: String = "Clear", onClear: ((FieldRow) -> Unit)? = null,
    ): FieldRow {
        val f = fieldRow(label, value, android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD, help, onEdit = onEdit)
        SecretField.protect(f.edit)
        val show = chip("Show") { b ->
            val on = !SecretField.isShown(f.edit)
            SecretField.setShown(f.edit, on)
            b.text = if (on) "Hide" else "Show"; b.contentDescription = "${b.text} $what"
        }.apply { contentDescription = "Show $what" }
        val paste = chip("Paste") {
            val cm = ctx.getSystemService(android.content.ClipboardManager::class.java)
            val t = cm?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(ctx)?.toString()?.trim()
            if (t.isNullOrEmpty()) android.widget.Toast.makeText(ctx, "Clipboard is empty", android.widget.Toast.LENGTH_SHORT).show() else f.edit.setText(t)
        }.apply { contentDescription = "Paste $what from the clipboard" }
        val clear = chip(clearLabel) { onClear?.invoke(f) ?: f.edit.setText("") }.apply { contentDescription = "$clearLabel $what" }
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(dp(8), 0, dp(8), dp(8))
            for (b in listOf(show, paste, clear)) addView(b, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(4); marginEnd = dp(4) })
        })
        return f
    }

    /** Small tonal button (48 dp tall) for in-card actions. */
    fun chip(text: String, onClick: (Button) -> Unit): Button = Button(ctx, null, 0, android.R.style.Widget_DeviceDefault_Button_Borderless).apply {
        this.text = text; isAllCaps = false; textSize = 14f; setTextColor(p.accent)
        background = ripple(rounded(p.tonal, 12), 12)
        minHeight = dp(48); minimumHeight = dp(48)
        stateListAnimator = null
        setOnClickListener { onClick(this) }
    }

    /** Row of chips inside a card. */
    fun LinearLayout.chipRow(vararg chips: Button) = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL
        setPadding(dp(12), dp(4), dp(12), dp(12))
        for (b in chips) addView(b, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(4); marginEnd = dp(4) })
    }.also(::addView)

    /** 48 dp square icon button (route order ↑ / ↓). */
    fun iconButton(res: Int, description: String, onClick: () -> Unit) = ImageButton(ctx).apply {
        setImageResource(res); imageTintList = ColorStateList.valueOf(p.accent)
        background = ripple(null, 24)
        contentDescription = description
        minimumWidth = dp(48); minimumHeight = dp(48)
        layoutParams = LinearLayout.LayoutParams(dp(48), dp(48))
        setOnClickListener { onClick() }
    }

    // ---- actions + results ----

    enum class Style { PRIMARY, TONAL, DANGER }

    /** Full-width page action ("Test connection"); put a [result] right after it. */
    fun LinearLayout.action(text: String, style: Style = Style.TONAL, onClick: () -> Unit): Button {
        val (bg, fg) = when (style) {
            Style.PRIMARY -> p.accent to p.onAccent
            Style.TONAL -> p.surface to p.accent
            Style.DANGER -> p.surface to p.danger
        }
        val b = Button(ctx, null, 0, android.R.style.Widget_DeviceDefault_Button_Borderless).apply {
            this.text = text; isAllCaps = false; textSize = 16f; setTextColor(fg); setTypeface(typeface, Typeface.BOLD)
            background = ripple(rounded(bg, 20), 20)
            minHeight = dp(56); minimumHeight = dp(56)
            stateListAnimator = null
            setOnClickListener { onClick() }
        }
        addView(b, lp(top = 8, bottom = 4))
        return b
    }

    class Result(val view: TextView, private val p: DevPalette) {
        fun show(text: CharSequence, tone: Tone = Tone.NEUTRAL) {
            view.text = text
            view.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
            view.setTextColor(when (tone) { Tone.OK -> p.ok; Tone.BAD -> p.danger; else -> p.text })
            (view.background as? GradientDrawable)?.setColor(when (tone) { Tone.BAD -> p.dangerBg; Tone.WARN -> p.warnBg; else -> p.surface })
        }
        val text: CharSequence get() = view.text
    }

    /** Result of the action above it (announced by TalkBack when it changes). */
    fun LinearLayout.result(): Result {
        val v = TextView(ctx).apply {
            textSize = 14f; setTextColor(p.text); setTextIsSelectable(true)
            background = rounded(p.surface, 16)
            setPadding(dp(16), dp(12), dp(16), dp(12))
            visibility = View.GONE
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        addView(v, lp(top = 4, bottom = 8))
        return Result(v, p)
    }

    /** Warning / error banner at the top of a page; hidden while empty. */
    fun LinearLayout.banner(): Result = result()
}
