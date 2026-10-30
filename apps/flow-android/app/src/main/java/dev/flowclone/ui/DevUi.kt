package dev.flowclone.ui

import android.app.Activity
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.InsetDrawable
import android.graphics.drawable.RippleDrawable
import android.text.Editable
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.TextWatcher
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Switch
import android.widget.TextView
import dev.flowclone.R

/**
 * Colour roles every screen uses, mapped onto the design tokens ([Tokens], from
 * docs/design/app-design/flow-theme.css). Light / dark follows the system (uiMode).
 * The design is monochrome: the "accent" role is the ink colour.
 */
internal class DevPalette(
    val bg: Int, val surface: Int, val field: Int, val text: Int, val muted: Int, val divider: Int,
    val accent: Int, val onAccent: Int, val tonal: Int, val danger: Int, val ok: Int, val warnBg: Int, val dangerBg: Int,
    /** The full token set (switch, badge, route, FAB ... colours). */
    val t: Tokens,
) {
    companion object {
        fun of(ctx: Context): DevPalette = from(Tokens.of(ctx))

        fun from(t: Tokens) = DevPalette(
            bg = t.bgSettings, surface = t.card, field = t.field, text = t.ink, muted = t.ink2, divider = t.divider,
            accent = t.ink, onAccent = t.card, tonal = t.selected, danger = t.danger, ok = t.ok, warnBg = t.warnBg, dangerBg = t.dangerBg,
            t = t,
        )
    }
}

internal enum class Tone { NEUTRAL, OK, BAD, WARN }

/**
 * Builder for one screen in the Murmur Android look (docs/design/app-design, Android*.dc.html):
 * platform views only, no UI deps. Figtree text, grouped 24 dp cards with hairline dividers on a
 * cool-grey page, uppercase section labels, rows with a title + grey subtitle, the design's pill
 * switch, outlined fields and buttons. Also the parts the main tabs use (top bar, FAB, badges,
 * route tags, segmented tabs, progress, callout, stat row). 48 dp+ touch targets, system insets.
 */
internal class DevUi(val activity: Activity) {
    val ctx: Context get() = activity
    val p = DevPalette.of(activity)
    /** Design tokens (same object as [DevPalette.t]). */
    val t: Tokens get() = p.t
    private val density get() = ctx.resources.displayMetrics.density
    private fun dp(v: Int) = (v * density).toInt()
    private fun dpf(v: Float) = v * density

    private fun rounded(color: Int, radiusDp: Int = 24) = GradientDrawable().apply { setColor(color); cornerRadius = dpf(radiusDp.toFloat()) }
    private fun outlined(fill: Int, radiusDp: Int, stroke: Int = t.outline) = GradientDrawable().apply {
        setColor(fill); cornerRadius = dpf(radiusDp.toFloat()); setStroke(dpf(1.5f).toInt().coerceAtLeast(1), stroke)
    }

    private fun ripple(content: Drawable?, radiusDp: Int = 0): RippleDrawable {
        val mask = if (radiusDp > 0) rounded(Color.WHITE, radiusDp) else GradientDrawable().apply { setColor(Color.WHITE) }
        return RippleDrawable(ColorStateList.valueOf((p.text and 0x00FFFFFF) or 0x22000000), content, mask)
    }

    /** Figtree at [weight] (system sans if the font cannot load). */
    fun TextView.sans(weight: Int = TypeScale.W_REGULAR): TextView = apply { typeface = Fonts.sans(ctx, weight) }

    /** EB Garamond at [weight] (system serif if the font cannot load). */
    fun TextView.serif(weight: Int = TypeScale.W_REGULAR): TextView = apply { typeface = Fonts.serif(ctx, weight) }

    /** A Figtree TextView in the design's type scale. */
    fun tv(text: CharSequence, sizeSp: Float = TypeScale.ROW, color: Int = p.text, weight: Int = TypeScale.W_REGULAR) = TextView(ctx).apply {
        this.text = text; textSize = sizeSp; setTextColor(color); sans(weight)
    }

    /**
     * Scrolling column on the page background ([background]: settings grey by default, `t.bg`
     * for the main tabs); pads itself by the system bars, cutout and keyboard.
     */
    fun screen(background: Int = p.bg, build: LinearLayout.() -> Unit): LinearLayout {
        val col = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            build()
        }
        val scroll = ScrollView(ctx).apply {
            setBackgroundColor(background)
            isFillViewport = true
            clipToPadding = false
            addView(col, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            setOnApplyWindowInsetsListener { v, insets ->
                val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                v.setPadding(i.left + dp(16), i.top + dp(8), i.right + dp(16), i.bottom + dp(24))
                insets
            }
        }
        activity.window.decorView.setBackgroundColor(background)
        activity.setContentView(scroll)
        scroll.requestApplyInsets()
        return col
    }

    // ---- text ----

    /** `.a-section`: 13 sp semibold uppercase, .06 em tracking, grey. */
    fun LinearLayout.sectionLabel(text: String) = tv(text, TypeScale.SECTION, p.muted, TypeScale.W_SEMIBOLD).apply {
        isAllCaps = true
        letterSpacing = TypeScale.SECTION_TRACKING
        setPadding(dp(4), dp(20), dp(4), dp(10))
        isAccessibilityHeading = true
    }.also(::addView)

    /** Day / group heading on a main tab ("Today"): 20 sp bold. */
    fun LinearLayout.heading(text: String) = tv(text, TypeScale.HEADING, p.text, TypeScale.W_BOLD).apply {
        setPadding(dp(4), dp(10), dp(4), dp(4))
        isAccessibilityHeading = true
    }.also(::addView)

    fun LinearLayout.note(text: CharSequence, color: Int = p.muted, sizeSp: Float = 15f) = tv(text, sizeSp, color).apply {
        setPadding(dp(4), dp(6), dp(4), dp(6))
    }.also(::addView)

    // ---- cards ----

    /** `.a-card.a-list`: white 24 dp rounded group; children are separated by inset hairlines. */
    fun LinearLayout.card(build: LinearLayout.() -> Unit): LinearLayout = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(p.surface, 24)
        clipToOutline = true
        dividerDrawable = InsetDrawable(GradientDrawable().apply { setColor(p.divider); setSize(1, dp(1).coerceAtLeast(1)) }, dp(20), 0, dp(20), 0)
        showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        build()
    }.also { addView(it, lp(top = 4, bottom = 8)) }

    private fun lp(top: Int = 0, bottom: Int = 0) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
        topMargin = dp(top); bottomMargin = dp(bottom)
    }

    /** Card header: bold title with a grey counter / hint on the right (e.g. "2 of 3"). */
    fun LinearLayout.cardHeader(title: String, trailing: String = ""): TextView {
        val tt = tv(title, TypeScale.ROW, p.text, TypeScale.W_SEMIBOLD).apply { isAccessibilityHeading = true }
        val r = tv(trailing, TypeScale.META, p.muted).apply { gravity = Gravity.END }
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(20), dp(16), dp(20), dp(6))
            addView(tt, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(r)
        })
        return r
    }

    private fun icon(res: Int, tint: Int, sizeDp: Int = 24) = ImageView(ctx).apply {
        setImageResource(res); imageTintList = ColorStateList.valueOf(tint)
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        layoutParams = LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp))
    }

    /** Row texts: 17 sp medium title, 15 sp grey subtitle (hidden while empty). */
    private fun titleSub(title: String, sub: String, titleColor: Int = p.text): Pair<LinearLayout, Pair<TextView, TextView>> {
        val tt = tv(title, TypeScale.ROW, titleColor, TypeScale.W_MEDIUM)
        val s = tv(sub, TypeScale.SUB, p.muted).apply { visibility = if (sub.isEmpty()) View.GONE else View.VISIBLE; setPadding(0, dp(2), 0, 0) }
        val col = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL; addView(tt); addView(s) }
        return col to (tt to s)
    }

    private fun LinearLayout.rowBox(clickable: Boolean, endPad: Int = 20) {
        orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
        minimumHeight = dp(ROW_MIN_DP)
        setPadding(dp(20), dp(14), dp(endPad), dp(14))
        if (clickable) { background = ripple(null); isClickable = true; isFocusable = true }
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
            rowBox(true, endPad = 16)
            setOnClickListener { onClick() }
            if (iconRes != null) addView(icon(iconRes, if (danger) p.danger else p.text).apply {
                (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(16)
            })
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
            if (chevron) addView(icon(R.drawable.ic_lw_chevron, p.text, 22))
        }
        addView(row)
        return NavRow(row, ts.first, ts.second).also { it.setSubtitle(subtitle) }
    }

    /** A read-only "label / value" row (status values; selectable for copy). */
    fun LinearLayout.infoRow(label: String, value: String = ""): TextView {
        val v = tv(value, TypeScale.SUB, p.text).apply { setTextIsSelectable(true) }
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(12), dp(20), dp(12))
            addView(tv(label, TypeScale.META, p.muted))
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

    /** `.a-switch`: 52x32 dp pill (off = light track, small dot; on = ink track, bigger dot). Not focusable on its own. */
    fun designSwitch(checked: Boolean = false): Switch = Switch(ctx).apply {
        showText = false
        thumbTintList = null; trackTintList = null
        trackDrawable = LwDrawables.switchTrack(t, density)
        thumbDrawable = LwDrawables.switchThumb(t, density)
        switchMinWidth = dp(52)
        background = null
        isChecked = checked
    }

    fun LinearLayout.toggleRow(title: String, description: String, checked: Boolean, enabled: Boolean = true): ToggleRow {
        val (texts, ts) = titleSub(title, description)
        val sw = designSwitch(checked).apply {
            isClickable = false; isFocusable = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val row = LinearLayout(ctx).apply {
            rowBox(true)
            setOnClickListener { if (sw.isEnabled) sw.toggle() }
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(16) })
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

    // ---- slider ----

    /** Slider row: title with the current value on the right, a full-width SeekBar below. */
    class SliderRow(val row: LinearLayout, val bar: SeekBar, val value: TextView, private val step: Int, private val label: (Int) -> String) {
        /** In the row's units (a multiple of the step). */
        val current: Int get() = bar.progress * step
        private var listener: ((Int) -> Unit)? = null

        init {
            bar.setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(b: SeekBar, progress: Int, fromUser: Boolean) {
                    show(progress * step)
                    if (fromUser) listener?.invoke(progress * step)
                }
                override fun onStartTrackingTouch(b: SeekBar) = Unit
                override fun onStopTrackingTouch(b: SeekBar) = Unit
            })
            show(current)
        }

        private fun show(v: Int) { value.text = label(v); bar.stateDescription = label(v) }

        /** [f] gets each value the user picks, while dragging too. */
        fun onChange(f: (Int) -> Unit) { listener = f }
    }

    /**
     * [min]..[max] in [step]s, showing [label] of the current value; TalkBack reads the title
     * and that label (state description) and steps it with its scroll actions.
     */
    fun LinearLayout.sliderRow(title: String, description: String, min: Int, max: Int, step: Int, value: Int, label: (Int) -> String): SliderRow {
        val (texts, _) = titleSub(title, description)
        val v = tv("", TypeScale.ROW, p.text, TypeScale.W_SEMIBOLD).apply {
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val bar = SeekBar(ctx).apply {
            this.max = max / step; this.min = min / step; progress = value / step
            thumbTintList = ColorStateList.valueOf(p.text)
            progressTintList = ColorStateList.valueOf(p.text)
            progressBackgroundTintList = ColorStateList.valueOf(t.outline)
            minimumHeight = dp(48)
            contentDescription = title
        }
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(14), dp(20), dp(8))
            addView(LinearLayout(ctx).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
                addView(v)
            })
            addView(bar, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(48)).apply { topMargin = dp(4) })
        }
        addView(row)
        return SliderRow(row, bar, v, step, label)
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
                buttonTintList = ColorStateList(arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()), intArrayOf(p.text, p.muted))
            }
            val row = LinearLayout(ctx).apply {
                rowBox(true)
                setPadding(dp(10), dp(8), dp(20), dp(8))
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

    /** Label above an outlined input (10 dp radius, 1.5 dp outline); grey help and a red inline error below it. */
    fun LinearLayout.fieldRow(label: String, value: String, inputType: Int, help: String = "", hint: String = "", minLines: Int = 1, onEdit: () -> Unit): FieldRow {
        val box = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(14), dp(20), dp(12)) }
        val l = tv(label, TypeScale.META, p.muted, TypeScale.W_MEDIUM)
        val e = EditText(ctx).apply {
            id = View.generateViewId()
            this.inputType = inputType; setText(value)
            if (minLines > 1) { isSingleLine = false; this.minLines = minLines; gravity = Gravity.TOP or Gravity.START } else isSingleLine = true
            this.hint = hint; setHintTextColor(p.muted)
            textSize = TypeScale.ROW; setTextColor(p.text); sans()
            background = outlined(p.field, 10)
            setPadding(dp(14), dp(12), dp(14), dp(12))
            minHeight = dp(52); minimumHeight = dp(52)
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun afterTextChanged(s: Editable?) = onEdit()
            })
        }
        l.labelFor = e.id
        val h = tv(help, TypeScale.META, p.muted).apply { setPadding(0, dp(6), 0, 0); visibility = if (help.isEmpty()) View.GONE else View.VISIBLE }
        val err = tv("", TypeScale.META, p.danger).apply {
            setPadding(0, dp(6), 0, 0); visibility = View.GONE
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
            val tx = cm?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(ctx)?.toString()?.trim()
            if (tx.isNullOrEmpty()) android.widget.Toast.makeText(ctx, "Clipboard is empty", android.widget.Toast.LENGTH_SHORT).show() else f.edit.setText(tx)
        }.apply { contentDescription = "Paste $what from the clipboard" }
        val clear = chip(clearLabel) { onClear?.invoke(f) ?: f.edit.setText("") }.apply { contentDescription = "$clearLabel $what" }
        addView(LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(dp(16), 0, dp(16), dp(12))
            for (b in listOf(show, paste, clear)) addView(b, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(4); marginEnd = dp(4) })
        })
        return f
    }

    private fun baseButton(text: String) = Button(ctx, null, 0, android.R.style.Widget_DeviceDefault_Button_Borderless).apply {
        this.text = text; isAllCaps = false; sans(TypeScale.W_MEDIUM)
        stateListAnimator = null
        minHeight = dp(48); minimumHeight = dp(48)
    }

    /** Small outlined pill button (48 dp tall) for in-card actions (Show / Paste / Grant). */
    fun chip(text: String, onClick: (Button) -> Unit): Button = baseButton(text).apply {
        textSize = 15f; setTextColor(p.text)
        background = ripple(outlined(Color.TRANSPARENT, 24), 24)
        setPadding(dp(16), 0, dp(16), 0)
        setOnClickListener { onClick(this) }
    }

    /** Row of chips inside a card. */
    fun LinearLayout.chipRow(vararg chips: Button) = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL
        setPadding(dp(16), dp(4), dp(16), dp(14))
        for (b in chips) addView(b, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(4); marginEnd = dp(4) })
    }.also(::addView)

    /** 48 dp square icon button in ink (route order, top-bar actions). */
    fun iconButton(res: Int, description: String, tint: Int = p.text, onClick: () -> Unit) = ImageButton(ctx).apply {
        setImageResource(res); imageTintList = ColorStateList.valueOf(tint)
        background = ripple(null, 24)
        contentDescription = description
        minimumWidth = dp(48); minimumHeight = dp(48)
        layoutParams = LinearLayout.LayoutParams(dp(48), dp(48))
        setOnClickListener { onClick() }
    }

    /** Filled dark button (`--a-button`, 10 dp radius, 17 sp): the one main action of a card or page. */
    fun filledButton(text: String, onClick: () -> Unit): Button = baseButton(text).apply {
        textSize = TypeScale.ROW; setTextColor(t.onButton)
        background = ripple(rounded(t.button, 10), 10)
        setOnClickListener { onClick() }
    }

    /** Outlined button (1.5 dp `--a-outline`, 10 dp radius, or a pill with [pill]). */
    fun outlinedButton(text: String, pill: Boolean = false, color: Int = p.text, onClick: () -> Unit): Button = baseButton(text).apply {
        textSize = TypeScale.BUTTON; setTextColor(color)
        val r = if (pill) 24 else 10
        background = ripple(outlined(Color.TRANSPARENT, r), r)
        if (pill) setPadding(dp(16), 0, dp(16), 0)
        setOnClickListener { onClick() }
    }

    // ---- actions + results ----

    enum class Style { PRIMARY, TONAL, DANGER }

    /** Full-width page action ("Test connection"): PRIMARY filled dark, TONAL / DANGER outlined. Put a [result] right after it. */
    fun LinearLayout.action(text: String, style: Style = Style.TONAL, onClick: () -> Unit): Button {
        val b = baseButton(text).apply {
            textSize = TypeScale.BUTTON
            minHeight = dp(52); minimumHeight = dp(52)
            when (style) {
                Style.PRIMARY -> { setTextColor(t.onButton); background = ripple(rounded(t.button, 10), 10) }
                Style.TONAL -> { setTextColor(p.text); background = ripple(outlined(p.surface, 10), 10) }
                Style.DANGER -> { setTextColor(p.danger); background = ripple(outlined(p.surface, 10), 10) }
            }
            setOnClickListener { onClick() }
        }
        addView(b, lp(top = 8, bottom = 4))
        return b
    }

    class Result(val view: TextView, private val p: DevPalette) {
        fun show(text: CharSequence, tone: Tone = Tone.NEUTRAL) {
            view.text = text
            view.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
            view.setTextColor(when (tone) { Tone.OK -> p.ok; Tone.BAD -> p.danger; Tone.WARN -> p.t.onPeach; else -> p.text })
            (view.background as? GradientDrawable)?.setColor(when (tone) { Tone.BAD -> p.dangerBg; Tone.WARN -> p.t.peach; else -> p.surface })
        }
        val text: CharSequence get() = view.text
    }

    /** Result of the action above it (announced by TalkBack when it changes). */
    fun LinearLayout.result(): Result {
        val v = tv("", TypeScale.SUB, p.text).apply {
            setTextIsSelectable(true)
            background = rounded(p.surface, 16)
            setPadding(dp(16), dp(14), dp(16), dp(14))
            visibility = View.GONE
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        addView(v, lp(top = 4, bottom = 8))
        return Result(v, p)
    }

    /** Warning / error banner at the top of a page; hidden while empty. */
    fun LinearLayout.banner(): Result = result()

    // =====================================================================================
    // Screens beyond Settings (main screen, Dictionary & snippets, History, model import)
    // =====================================================================================

    /**
     * [screen] for a page that is rebuilt while open (main screen polling, model import progress):
     * keeps the scroll position of the page it replaces.
     */
    fun rebuild(background: Int = p.bg, build: LinearLayout.() -> Unit): LinearLayout {
        val old = (activity.findViewById<ViewGroup>(android.R.id.content)?.getChildAt(0) as? ScrollView)?.scrollY ?: 0
        val col = screen(background, build)
        if (old > 0) (col.parent as ScrollView).let { s -> s.post { s.scrollTo(0, old) } }
        return col
    }

    /** TalkBack announcement for a change made elsewhere (a permission granted, an item deleted). */
    fun announce(text: CharSequence) {
        if (text.isNotEmpty()) activity.window.decorView.announceForAccessibility(text)
    }

    /** Cancel / [positive] prompt; [onNo] also runs when the dialog is dismissed. */
    fun confirm(title: String, message: String, positive: String, onYes: () -> Unit, onNo: (() -> Unit)? = null) =
        android.app.AlertDialog.Builder(activity)
            .setTitle(title).setMessage(message)
            .setNegativeButton("Cancel") { _, _ -> onNo?.invoke() }
            .setOnCancelListener { onNo?.invoke() }
            .setPositiveButton(positive) { _, _ -> onYes() }
            .show()

    /** Status row: icon + title + explanation, and on the right its state badge ("Granted") or the fixing action ("Grant"). */
    class StateRow(val view: LinearLayout, val title: TextView, val subtitle: TextView, val state: TextView)

    fun LinearLayout.stateRow(iconRes: Int?, title: String, subtitle: String, state: String, tone: Tone, action: String? = null, onClick: (() -> Unit)? = null): StateRow {
        val (texts, ts) = titleSub(title, subtitle)
        val actionable = action != null && onClick != null
        val st: TextView = if (actionable) {
            tv(action!!, 15f, p.text, TypeScale.W_MEDIUM).apply {
                gravity = Gravity.CENTER
                background = outlined(Color.TRANSPARENT, 24)
                setPadding(dp(16), dp(9), dp(16), dp(9))
            }
        } else when (tone) {
            Tone.OK -> badge(state, BadgeTone.SUCCESS)
            Tone.WARN -> badge(state, BadgeTone.WARNING)
            Tone.BAD -> badge(state, BadgeTone.DANGER)
            Tone.NEUTRAL -> tv(state, 14f, p.muted, TypeScale.W_MEDIUM).apply { gravity = Gravity.CENTER }
        }
        st.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        val row = LinearLayout(ctx).apply {
            rowBox(false)
            if (iconRes != null) addView(icon(iconRes, when (tone) { Tone.OK -> p.ok; Tone.BAD -> p.danger; else -> p.text }).apply {
                (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(16)
            })
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
            addView(st)
            texts.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            // One TalkBack stop: "Microphone, Not granted. Needed to hear you…, Grant".
            contentDescription = listOf("$title, $state", subtitle, if (actionable) "double-tap to $action" else "").filter { it.isNotEmpty() }.joinToString(". ")
            isFocusable = true
            if (actionable) {
                background = ripple(null)
                isClickable = true
                setOnClickListener { onClick!!() }
            }
        }
        addView(row)
        return StateRow(row, ts.first, ts.second, st)
    }

    /** Collapsible row (e.g. phone-specific "Keep it running" steps); reads as expandable to TalkBack. */
    class Expander(val header: View, val body: LinearLayout, private val arrow: ImageView) {
        var expanded: Boolean = false
            set(v) {
                field = v
                body.visibility = if (v) View.VISIBLE else View.GONE
                arrow.setImageResource(if (v) R.drawable.ic_dev_up else R.drawable.ic_dev_down)
                header.stateDescription = if (v) "Expanded" else "Collapsed"
            }
    }

    fun LinearLayout.expandRow(iconRes: Int?, title: String, subtitle: String, expanded: Boolean = false, build: LinearLayout.() -> Unit): Expander {
        val (texts, _) = titleSub(title, subtitle)
        val arrow = icon(R.drawable.ic_dev_down, p.text, 22)
        lateinit var ex: Expander
        val header = LinearLayout(ctx).apply {
            rowBox(true, endPad = 16)
            if (iconRes != null) addView(icon(iconRes, p.text).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(16) })
            addView(texts, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(12) })
            addView(arrow)
            contentDescription = listOf(title, subtitle).filter { it.isNotEmpty() }.joinToString(", ")
            setOnClickListener { ex.expanded = !ex.expanded }
            accessibilityDelegate = object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.addAction(if (ex.expanded) AccessibilityNodeInfo.AccessibilityAction.ACTION_COLLAPSE else AccessibilityNodeInfo.AccessibilityAction.ACTION_EXPAND)
                }
                override fun performAccessibilityAction(host: View, action: Int, args: android.os.Bundle?): Boolean = when (action) {
                    AccessibilityNodeInfo.ACTION_EXPAND -> { ex.expanded = true; true }
                    AccessibilityNodeInfo.ACTION_COLLAPSE -> { ex.expanded = false; true }
                    else -> super.performAccessibilityAction(host, action, args)
                }
            }
        }
        val body = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL; setPadding(0, 0, 0, dp(8)); build() }
        addView(header); addView(body)
        ex = Expander(header, body, arrow)
        ex.expanded = expanded
        return ex
    }

    /** `.a-search`: 52 dp outlined search box above a long list; [onQuery] runs on every change. */
    fun LinearLayout.searchField(hint: String, query: String = "", onQuery: (String) -> Unit): EditText {
        val e = EditText(ctx).apply {
            this.hint = hint; setText(query); isSingleLine = true
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_FILTER
            setHintTextColor(p.muted)
            textSize = TypeScale.ROW; setTextColor(p.text); sans()
            background = outlined(p.surface, 10)
            // runCatching: JVM unit tests run without packaged resources.
            val glass = runCatching { ctx.getDrawable(R.drawable.ic_lw_search)?.mutate()?.apply { setTint(t.outline) } }.getOrNull()
            setCompoundDrawablesRelativeWithIntrinsicBounds(glass, null, null, null)
            compoundDrawablePadding = dp(14)
            setPadding(dp(16), dp(12), dp(16), dp(12))
            minHeight = dp(52); minimumHeight = dp(52)
            contentDescription = hint
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun onTextChanged(s: CharSequence?, a: Int, b: Int, c: Int) = Unit
                override fun afterTextChanged(s: Editable?) = onQuery(s?.toString().orEmpty())
            })
        }
        addView(e, lp(top = 4, bottom = 12))
        return e
    }

    /** Empty list placeholder inside a card: a title and a one-line hint. */
    fun LinearLayout.emptyState(title: String, hint: String): View = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER_HORIZONTAL
        setPadding(dp(24), dp(28), dp(24), dp(28))
        addView(tv(title, TypeScale.ROW, p.text, TypeScale.W_SEMIBOLD).apply { gravity = Gravity.CENTER })
        addView(tv(hint, TypeScale.SUB, p.muted).apply { gravity = Gravity.CENTER; setPadding(0, dp(6), 0, 0) })
        isFocusable = true
        contentDescription = "$title. $hint"
    }.also(::addView)

    /**
     * List entry (dictionary word, dictation): title (2 lines max), grey meta line, an optional
     * small badge, an optional coloured flag line, an optional trailing button. The row is one
     * touch target; [actions] are offered to TalkBack as custom actions (and long-press runs the first).
     */
    class ListRow(val view: LinearLayout, val title: TextView, val meta: TextView, val badge: TextView, val flag: TextView)

    fun LinearLayout.listRow(
        title: String, meta: String, badge: String = "", flag: String? = null, flagTone: Tone = Tone.WARN,
        trailing: View? = null, actions: List<Pair<String, () -> Unit>> = emptyList(), onClick: () -> Unit,
    ): ListRow {
        val tt = tv(title, TypeScale.ROW, p.text).apply { maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END }
        val m = tv(meta, 14f, p.muted).apply { maxLines = 2; ellipsize = android.text.TextUtils.TruncateAt.END }
        val b = badge(badge, BadgeTone.NEUTRAL).apply { visibility = if (badge.isEmpty()) View.GONE else View.VISIBLE }
        val metaLine = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            addView(m, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(8) })
            addView(b)
        }
        val f = tv(flag.orEmpty(), TypeScale.META, if (flagTone == Tone.BAD) p.danger else if (flagTone == Tone.OK) p.ok else t.badgeWarnFg, TypeScale.W_SEMIBOLD).apply {
            visibility = if (flag.isNullOrEmpty()) View.GONE else View.VISIBLE
            setPadding(0, dp(2), 0, 0)
        }
        val col = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            addView(tt); addView(metaLine, lp(top = 2)); addView(f)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
        }
        val row = LinearLayout(ctx).apply {
            rowBox(true, endPad = if (trailing != null) 4 else 20)
            setPadding(paddingLeft, dp(12), paddingRight, dp(12))
            setOnClickListener { onClick() }
            addView(col, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            if (trailing != null) addView(trailing)
            contentDescription = listOf(title, meta, badge, flag.orEmpty()).filter { it.isNotEmpty() }.joinToString(", ")
            if (actions.isNotEmpty()) {
                setOnLongClickListener { actions.first().second(); true }
                val ids = actions.map { View.generateViewId() }
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        actions.forEachIndexed { i, a -> info.addAction(AccessibilityNodeInfo.AccessibilityAction(ids[i], a.first)) }
                    }
                    override fun performAccessibilityAction(host: View, action: Int, args: android.os.Bundle?): Boolean {
                        val i = ids.indexOf(action)
                        if (i >= 0) { actions[i].second(); return true }
                        return super.performAccessibilityAction(host, action, args)
                    }
                }
            }
        }
        addView(row)
        return ListRow(row, tt, m, b, f)
    }

    /** Selectable body text inside a card (full transcript, a download link). */
    fun LinearLayout.selectableText(text: CharSequence, sizeSp: Float = TypeScale.ROW, color: Int = p.text): TextView = tv(text, sizeSp, color).apply {
        setTextIsSelectable(true)
        setPadding(dp(20), dp(14), dp(20), dp(16))
    }.also(::addView)

    // =====================================================================================
    // Design-system parts for the app shell and main tabs (Android*.dc.html)
    // =====================================================================================

    enum class TopNav { NONE, MENU, BACK }

    /** A top-bar action button (icon + TalkBack label). */
    class TopAction(val iconRes: Int, val description: String, val onClick: () -> Unit)

    class TopBar(val view: FrameLayout, val title: TextView, val nav: ImageButton?, val actions: List<ImageButton>)

    /**
     * 64 dp top bar: menu or back button (48 dp) at the start, centred title, trailing icon
     * actions. [mainTab] picks the tab title style (700 22 sp, tight tracking) over the settings
     * page style (550 21 sp). Not attached; [topBar] adds one to a column.
     */
    fun buildTopBar(
        title: String, nav: TopNav = TopNav.NONE, onNav: (() -> Unit)? = null,
        actions: List<TopAction> = emptyList(), mainTab: Boolean = nav == TopNav.MENU,
    ): TopBar {
        val titleView = tv(title, if (mainTab) TypeScale.TITLE else TypeScale.PAGE_TITLE, p.text, if (mainTab) TypeScale.W_BOLD else TypeScale.W_MEDIUM).apply {
            if (mainTab) letterSpacing = TypeScale.TITLE_TRACKING
            gravity = Gravity.CENTER
            maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END
            isAccessibilityHeading = true
        }
        val navBtn = when (nav) {
            TopNav.NONE -> null
            TopNav.MENU -> iconButton(R.drawable.ic_lw_menu, "Open menu") { onNav?.invoke() }
            TopNav.BACK -> iconButton(R.drawable.ic_lw_back, "Back") { onNav?.invoke() }
        }
        val acts = actions.map { a -> iconButton(a.iconRes, a.description) { a.onClick() } }
        val side = dp(56) * maxOf(1, acts.size)
        val bar = FrameLayout(ctx).apply {
            minimumHeight = dp(64)
            setPadding(dp(4), 0, dp(4), 0)
            addView(titleView, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(64)).apply { marginStart = side; marginEnd = side })
            if (navBtn != null) addView(navBtn, FrameLayout.LayoutParams(dp(48), dp(48), Gravity.START or Gravity.CENTER_VERTICAL).apply { marginStart = dp(4) })
            if (acts.isNotEmpty()) addView(LinearLayout(ctx).apply {
                orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
                for (b in acts) addView(b)
            }, FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.END or Gravity.CENTER_VERTICAL).apply { marginEnd = dp(4) })
        }
        return TopBar(bar, titleView, navBtn, acts)
    }

    /** [buildTopBar], added to this column (full width: undoes the screen's 16 dp side padding). */
    fun LinearLayout.topBar(
        title: String, nav: TopNav = TopNav.NONE, onNav: (() -> Unit)? = null,
        actions: List<TopAction> = emptyList(), mainTab: Boolean = nav == TopNav.MENU,
    ): TopBar = buildTopBar(title, nav, onNav, actions, mainTab).also {
        addView(it.view, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(64)).apply { marginStart = -dp(12); marginEnd = -dp(12) })
    }

    /** `.a-fab`: 56 dp ink circle with a white icon and elevation; not attached (float it in a FrameLayout). */
    fun fab(iconRes: Int, description: String, onClick: () -> Unit): ImageButton = ImageButton(ctx).apply {
        setImageResource(iconRes); imageTintList = ColorStateList.valueOf(t.onFab)
        background = ripple(LwDrawables.circle(t.fab), 28)
        contentDescription = description
        elevation = dpf(6f)
        minimumWidth = dp(56); minimumHeight = dp(56)
        layoutParams = FrameLayout.LayoutParams(dp(56), dp(56), Gravity.END or Gravity.BOTTOM).apply { marginEnd = dp(16); bottomMargin = dp(16) }
        setOnClickListener { onClick() }
    }

    enum class BadgeTone { SUCCESS, WARNING, DANGER, NEUTRAL }

    /** `.fl-badge`: 22 dp tall, 6 dp radius, 12 sp medium ("Granted", "Off", "1 to fix"). Not attached. */
    fun badge(text: String, tone: BadgeTone = BadgeTone.NEUTRAL): TextView {
        val (bg, fg) = when (tone) {
            BadgeTone.SUCCESS -> t.badgeOkBg to t.badgeOkFg
            BadgeTone.WARNING -> t.badgeWarnBg to t.badgeWarnFg
            BadgeTone.DANGER -> p.dangerBg to p.danger
            BadgeTone.NEUTRAL -> t.badgeNeutralBg to t.badgeNeutralFg
        }
        return tv(text, TypeScale.BADGE, fg, TypeScale.W_MEDIUM).apply {
            background = rounded(bg, 6)
            gravity = Gravity.CENTER
            minHeight = dp(22)
            setPadding(dp(8), dp(1), dp(8), dp(1))
            maxLines = 1
        }
    }

    /** Where a dictation ran (`.lw-route`). */
    enum class Route(val label: String) { PHONE("This phone"), SERVER("Your server"), CLOUD("Cloud") }

    /** `.lw-route` tag: 20 dp, 4 dp radius, 11 sp medium, teal / lilac / peach per [route]. Not attached. */
    fun routeTag(route: Route, label: String = route.label): TextView {
        val (bg, fg) = when (route) {
            Route.PHONE -> t.routePhoneBg to t.routePhoneFg
            Route.SERVER -> t.routeServerBg to t.routeServerFg
            Route.CLOUD -> t.routeCloudBg to t.routeCloudFg
        }
        return tv(label, TypeScale.ROUTE, fg, TypeScale.W_MEDIUM).apply {
            background = rounded(bg, 4)
            gravity = Gravity.CENTER
            minHeight = dp(20)
            setPadding(dp(6), 0, dp(6), 0)
            maxLines = 1
        }
    }

    /** `.lw-rank`: 24 dp square, 6 dp radius, the position number of an ordered engine. Not attached. */
    fun rankSquare(n: Int): TextView = tv(n.toString(), 12f, t.rankFg, TypeScale.W_SEMIBOLD).apply {
        background = rounded(t.rankBg, 6)
        gravity = Gravity.CENTER
        layoutParams = LinearLayout.LayoutParams(dp(24), dp(24))
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    /** Segmented tabs; [select] changes the selection without calling the pick callback. */
    class Segmented(val view: LinearLayout, val buttons: List<TextView>, private val restyle: (Int) -> Unit) {
        var selected: Int = -1; private set
        internal var onPick: ((Int) -> Unit)? = null
        fun select(i: Int) { selected = i; restyle(i) }
        internal fun pick(i: Int) { if (i != selected) { select(i); onPick?.invoke(i) } }
    }

    /** `.lw-seg`: equal-width segments on a sand track, the chosen one raised; each segment is a 48 dp tab. */
    fun LinearLayout.segmented(options: List<String>, selected: Int, onPick: (Int) -> Unit): Segmented {
        lateinit var seg: Segmented
        val btns = options.mapIndexed { i, label ->
            tv(label, 14f, t.segText, TypeScale.W_MEDIUM).apply {
                gravity = Gravity.CENTER
                minHeight = dp(48); minimumHeight = dp(48)
                maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END
                isClickable = true; isFocusable = true
                setOnClickListener { seg.pick(i) }
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        info.className = android.widget.RadioButton::class.java.name
                        info.isCheckable = true; info.isChecked = seg.selected == i
                        info.isSelected = seg.selected == i
                    }
                }
            }
        }
        val track = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            background = rounded(t.segBg, 10)
            setPadding(dp(3), dp(3), dp(3), dp(3))
            for (b in btns) addView(b, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginStart = dp(1); marginEnd = dp(1) })
        }
        seg = Segmented(track, btns) { sel ->
            btns.forEachIndexed { i, b ->
                val on = i == sel
                b.isSelected = on
                b.setTextColor(if (on) t.segOnText else t.segText)
                b.background = if (on) rounded(t.segOn, 8).also { if (!t.night) b.elevation = dpf(1f) } else ripple(null, 8)
                if (!on) b.elevation = 0f
            }
        }
        seg.select(selected)
        seg.onPick = onPick
        addView(track, lp(top = 4, bottom = 8))
        return seg
    }

    /** Progress bar with an optional caption above ("3 of 5 done"). */
    class Progress(val view: LinearLayout, val caption: TextView, val bar: View, private val fill: View) {
        /** [fraction] in 0..1; [label] is the caption and what TalkBack reads. */
        fun set(fraction: Float, label: String) {
            val f = fraction.coerceIn(0f, 1f)
            (fill.layoutParams as LinearLayout.LayoutParams).weight = f
            fill.requestLayout()
            caption.text = label; caption.visibility = if (label.isEmpty()) View.GONE else View.VISIBLE
            bar.stateDescription = label
            (bar as LinearLayout).weightSum = 1f
        }
    }

    /** 6 dp ink-on-grey bar (Permissions "3 of 5 done"). */
    fun LinearLayout.progress(fraction: Float, label: String = ""): Progress {
        val cap = tv(label, TypeScale.SUB, p.muted).apply { importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO }
        val fill = View(ctx).apply { setBackgroundColor(p.text) }
        val bar = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            background = rounded(t.selected, 3)
            clipToOutline = true
            weightSum = 1f
            addView(fill, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 0f))
            isFocusable = true
            accessibilityDelegate = object : View.AccessibilityDelegate() {
                override fun onInitializeAccessibilityNodeInfo(host: View, info: AccessibilityNodeInfo) {
                    super.onInitializeAccessibilityNodeInfo(host, info)
                    info.className = android.widget.ProgressBar::class.java.name
                    val w = (fill.layoutParams as LinearLayout.LayoutParams).weight
                    info.rangeInfo = AccessibilityNodeInfo.RangeInfo.obtain(AccessibilityNodeInfo.RangeInfo.RANGE_TYPE_PERCENT, 0f, 100f, w * 100f)
                }
            }
        }
        val box = LinearLayout(ctx).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(4), dp(4), dp(4), dp(6))
            addView(cap)
            addView(bar, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(6)).apply { topMargin = dp(8) })
        }
        addView(box)
        return Progress(box, cap, bar, fill).also { it.set(fraction, label) }
    }

    /** Peach callout (16 dp radius): an optional semibold [lead] sentence, then [text]. */
    fun LinearLayout.callout(text: CharSequence, lead: String? = null): TextView {
        val body: CharSequence = if (lead.isNullOrEmpty()) text else SpannableStringBuilder().apply {
            append(lead)
            setSpan(android.text.style.StyleSpan(android.graphics.Typeface.BOLD), 0, lead.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            append(" "); append(text)
        }
        return tv(body, TypeScale.SUB, t.onPeach).apply {
            background = rounded(t.peach, 16)
            setPadding(dp(16), dp(14), dp(16), dp(14))
        }.also { addView(it, lp(top = 4, bottom = 8)) }
    }

    /** Three-number stat card: serif numbers (EB Garamond 26 sp) over 13 sp grey units; one TalkBack stop per stat. */
    class StatRow(val view: LinearLayout, val numbers: List<TextView>, val units: List<TextView>) {
        fun set(i: Int, number: String, unit: String = units[i].text.toString()) {
            numbers[i].text = number; units[i].text = unit
            (numbers[i].parent as View).contentDescription = "$number $unit"
        }
    }

    fun LinearLayout.statRow(stats: List<Pair<String, String>>): StatRow {
        val nums = ArrayList<TextView>(); val units = ArrayList<TextView>()
        val row = LinearLayout(ctx).apply {
            orientation = LinearLayout.HORIZONTAL
            background = rounded(p.surface, 24)
            setPadding(dp(20), dp(16), dp(20), dp(16))
            for ((n, u) in stats) {
                val num = tv(n, TypeScale.STAT, p.text).apply { serif(); includeFontPadding = false }
                val unit = tv(u, TypeScale.META, p.muted)
                nums += num; units += unit
                addView(LinearLayout(ctx).apply {
                    orientation = LinearLayout.VERTICAL
                    addView(num); addView(unit)
                    isFocusable = true
                    contentDescription = "$n $u"
                    num.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                    unit.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                }, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { marginEnd = dp(8) })
            }
        }
        addView(row, lp(top = 4, bottom = 8))
        return StatRow(row, nums, units)
    }

    /** `.lw-dot`: 8 dp status dot. Not attached. */
    fun dot(color: Int): View = View(ctx).apply {
        background = LwDrawables.circle(color)
        layoutParams = LinearLayout.LayoutParams(dp(8), dp(8))
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }

    companion object {
        /** Minimum height of a list / settings row (design: 56-58 dp; one-line rows). */
        const val ROW_MIN_DP = 58
    }
}
