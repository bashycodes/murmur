/* @ds-bundle: {"format":4,"namespace":"Murmur","components":[{"name":"Button"},{"name":"IconButton"},{"name":"Badge"},{"name":"Toggle"},{"name":"Kbd"},{"name":"TextInput"},{"name":"Select"},{"name":"Menu"},{"name":"Tabs"},{"name":"NavItem"},{"name":"Sidebar"},{"name":"SettingsGroup"},{"name":"SettingsRow"},{"name":"HistoryList"},{"name":"StatsRail"},{"name":"InsightCard"},{"name":"UsageBars"},{"name":"Heatmap"},{"name":"Banner"},{"name":"Callout"},{"name":"Checklist"},{"name":"EmptyState"},{"name":"OptionCard"},{"name":"Modal"},{"name":"Popover"},{"name":"Tooltip"},{"name":"Avatar"},{"name":"Icon"}]} */
(function () {
  var React = window.React;
  var h = React.createElement;
  var useState = React.useState;
  function cx() { return Array.prototype.filter.call(arguments, Boolean).join(" "); }

  /* ---------- Icon: 24px grid, 1.75 stroke, round caps (substitute for the app's own line set) ---------- */
  var P = {
    mic: ["M9 6a3 3 0 0 1 6 0v5a3 3 0 0 1-6 0z", "M5 11a7 7 0 0 0 14 0", "M12 18v3"],
    record: ["M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z", "M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z"],
    chart: ["M4 20h16", "M7 16V10", "M12 16V5", "M17 16v-4"],
    book: ["M6 3h12v18H6z", "M9.5 8h5", "M9.5 12h5"],
    scissors: ["M6 3.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z", "M6 15.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z", "M8 7.5L20 18", "M8 16.5L20 6"],
    type: ["M3 7V5h10v2", "M8 5v14", "M6 19h4", "M14 12v-1h7v1", "M17.5 11v8", "M16 19h3"],
    wand: ["M4 20L15 9", "M17 3l1 2 2 1-2 1-1 2-1-2-2-1 2-1z"],
    note: ["M5 4h14v16H5z", "M8.5 9h7", "M8.5 13h4"],
    gear: ["M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z", "M12 2.5v3", "M12 18.5v3", "M2.5 12h3", "M18.5 12h3", "M5.3 5.3l2.1 2.1", "M16.6 16.6l2.1 2.1", "M5.3 18.7l2.1-2.1", "M16.6 7.4l2.1-2.1"],
    search: ["M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14z", "M20 20l-4-4"],
    x: ["M6 6l12 12", "M18 6L6 18"],
    chevron: ["M6 9l6 6 6-6"],
    check: ["M5 12.5l4.5 4.5L19 7"],
    sun: ["M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8z", "M12 2v2", "M12 20v2", "M2 12h2", "M20 12h2", "M4.9 4.9l1.4 1.4", "M17.7 17.7l1.4 1.4", "M4.9 19.1l1.4-1.4", "M17.7 6.3l1.4-1.4"],
    moon: ["M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"],
    bell: ["M6 16v-5a6 6 0 0 1 12 0v5l2 2H4z", "M10 21h4"],
    info: ["M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z", "M12 11v5", "M12 8h.01"],
    plus: ["M12 5v14", "M5 12h14"],
    sliders: ["M4 7h9", "M17 7h3", "M4 17h3", "M11 17h9", "M15 5v4", "M9 15v4"],
    monitor: ["M3 4h18v12H3z", "M8 20h8", "M12 16v4"],
    user: ["M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z", "M12 7a3 3 0 1 0 0 6a3 3 0 1 0 0-6z", "M6.5 18a6 6 0 0 1 11 0"],
    users: ["M9 6a3 3 0 1 0 0 6a3 3 0 1 0 0-6z", "M3.5 19a5.5 5.5 0 0 1 11 0", "M16 6a3 3 0 0 1 0 6", "M17 14.5a5 5 0 0 1 4 4.5"],
    calendar: ["M4 5h16v16H4z", "M4 10h16", "M9 3v4", "M15 3v4"],
    file: ["M6 3h8l4 4v14H6z", "M14 3v4h4"],
    arrow: ["M5 12h14", "M13 6l6 6-6 6"],
    refresh: ["M20 12a8 8 0 1 1-2.3-5.7", "M20 4v4h-4"],
    sort: ["M8 4v16", "M4 8l4-4 4 4", "M16 20V4", "M12 16l4 4 4-4"],
    panel: ["M3 4h18v16H3z", "M9 4v16"],
    gift: ["M4 9h16v4H4z", "M5 13h14v8H5z", "M12 9v12", "M12 9C10 5 7 6 8 8", "M12 9c2-4 5-3 4-1"],
    help: ["M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z", "M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.3-1 .8-1 1.5v.7", "M12 17h.01"],
    hash: ["M5 9h14", "M5 15h14", "M10 4L8 20", "M16 4l-2 16"],
    shield: ["M12 3l7 3v6c0 4.5-3 7.5-7 9c-4-1.5-7-4.5-7-9V6z", "M9 12l2 2 4-4"],
    card: ["M3 6h18v13H3z", "M3 10h18"],
    share: ["M12 15V4", "M8 8l4-4 4 4", "M5 13v7h14v-7"]
  };
  function Icon(p) {
    var d = P[p.name] || [];
    return h("svg", { className: cx("fl-icon", p.className), viewBox: "0 0 24 24", width: p.size || 20, height: p.size || 20, "aria-hidden": "true", style: p.style },
      d.map(function (s, i) { return h("path", { key: i, d: s }); }));
  }

  /* ---------- Actions ---------- */
  function Button(p) {
    var variant = p.variant || "primary", size = p.size || "md";
    var rest = Object.assign({}, p); ["variant", "size", "icon", "iconRight", "block", "rounded", "className", "children"].forEach(function (k) { delete rest[k]; });
    return h("button", Object.assign({ type: "button" }, rest, {
      className: cx("fl-btn", "fl-btn-" + variant, size !== "md" && "fl-btn-" + size, p.block && "fl-btn-block", p.rounded && "fl-btn-rounded", p.className)
    }), p.icon && h(Icon, { name: p.icon, size: 16 }), p.children, p.iconRight && h(Icon, { name: p.iconRight, size: 16 }));
  }
  function IconButton(p) {
    return h("button", { type: "button", className: cx("fl-iconbtn", p.variant && "is-" + p.variant, p.active && "is-active", p.className), "aria-label": p.label, title: p.label, onClick: p.onClick },
      h(Icon, { name: p.icon, size: p.size || 20 }));
  }
  function Badge(p) { return h("span", { className: cx("fl-badge", "fl-badge-" + (p.tone || "dark"), p.className) }, p.icon && h(Icon, { name: p.icon, size: 14 }), p.children); }

  /* ---------- Inputs ---------- */
  function Toggle(p) {
    var s = useState(!!p.defaultChecked), on = p.checked != null ? p.checked : s[0];
    return h("button", {
      type: "button", role: "switch", "aria-checked": on ? "true" : "false", "aria-label": p.label, disabled: p.disabled, className: cx("fl-toggle", p.size === "small" && "is-small"),
      onClick: function () { if (p.checked == null) s[1](!on); if (p.onChange) p.onChange(!on); }
    });
  }
  function Kbd(p) {
    return h("span", { className: "fl-kbd-group" },
      (p.keys || []).map(function (k, i) { return h("kbd", { key: i, className: "fl-kbd" }, k); }),
      p.children && h("span", { className: "fl-kbd-text" }, p.children));
  }
  function TextInput(p) {
    var rest = Object.assign({}, p); delete rest.className;
    return h("input", Object.assign({ type: "text" }, rest, { className: cx("fl-input", p.className) }));
  }
  function Menu(p) {
    return h("div", { className: "fl-menu", role: "menu" }, (p.items || []).map(function (it) {
      var sel = it.value === p.value;
      return h("button", { key: it.value, type: "button", role: "menuitemradio", "aria-checked": sel, className: cx("fl-menu-item", sel && "is-selected", !it.icon && "no-icon"), onClick: function () { if (p.onSelect) p.onSelect(it.value); } },
        it.icon && h(Icon, { name: it.icon, size: 16 }), h("span", null, it.label), sel ? h(Icon, { name: "check", size: 16 }) : h("span"));
    }));
  }
  function Select(p) {
    var o = useState(!!p.defaultOpen), open = o[0];
    var v = useState(p.value != null ? p.value : p.defaultValue), val = p.value != null ? p.value : v[0];
    var items = p.options || [];
    var cur = items.filter(function (i) { return i.value === val; })[0] || {};
    return h("span", { className: "fl-select-wrap", style: { width: p.width || 200 } },
      h("button", { type: "button", disabled: p.disabled, className: cx("fl-select", open && "is-open"), style: { width: "100%" }, onClick: function () { o[1](!open); } },
        h("span", { className: "fl-select-value" }, cur.icon && h(Icon, { name: cur.icon, size: 16 }), cur.label || p.placeholder),
        h(Icon, { name: "chevron", size: 16, className: "fl-select-chev", style: open ? { transform: "rotate(180deg)" } : null })),
      open && h(Menu, { items: items, value: val, onSelect: function (x) { v[1](x); o[1](false); if (p.onChange) p.onChange(x); } }));
  }

  /* ---------- Navigation ---------- */
  function Tabs(p) {
    var s = useState(p.defaultValue || (p.tabs && p.tabs[0] && (p.tabs[0].value || p.tabs[0]))), val = p.value != null ? p.value : s[0];
    return h("div", { className: "fl-tabs" }, h("div", { className: "fl-tabs-list", role: "tablist" },
      (p.tabs || []).map(function (t) {
        var v = t.value || t, label = t.label || t, active = v === val;
        return h("button", { key: v, role: "tab", "aria-selected": active, className: cx("fl-tab", active && "is-active"), onClick: function () { s[1](v); if (p.onChange) p.onChange(v); } },
          label, t.badge && h(Badge, { tone: "brandLight" }, t.badge));
      })),
      p.actions && h("div", { className: "fl-tabs-actions" }, p.actions));
  }
  function NavItem(p) {
    return h("button", { type: "button", className: cx("fl-nav", p.active && "is-active"), "aria-current": p.active ? "page" : undefined, onClick: p.onClick },
      p.icon && h(Icon, { name: p.icon }), h("span", { className: "fl-nav-label" }, p.label), p.badge && h(Badge, { tone: "glow" }, p.badge));
  }
  function Sidebar(p) {
    return h("nav", { className: "fl-sidebar" },
      h("div", { className: "fl-logo" }, p.brand || "Murmur"),
      p.children);
  }

  /* ---------- Settings ---------- */
  function SettingsGroup(p) {
    return h(React.Fragment, null, p.title && h("div", { className: "fl-group-title" }, p.title), h("div", { className: "fl-group" }, p.children));
  }
  function SettingsRow(p) {
    return h("div", { className: "fl-row" },
      h("div", { className: "fl-row-text" }, h("div", { className: "fl-row-title" }, p.title), p.description && h("div", { className: "fl-row-desc" }, p.description)),
      p.control && h("div", { className: "fl-row-control" }, p.control));
  }

  /* ---------- Data ---------- */
  function HistoryList(p) {
    return h("div", null, (p.groups || []).map(function (g, gi) {
      return h(React.Fragment, { key: gi },
        h("div", { className: "fl-list-date" }, h("span", { className: "fl-overline" }, g.date), gi === 0 && p.onSearch !== false && h("span", { style: { marginLeft: "auto", color: "var(--foreground-tertiary)" } }, h(Icon, { name: "search", size: 16 }))),
        h("div", { className: "fl-list" }, g.items.map(function (it, i) {
          return h("div", { key: i, className: "fl-list-row" }, it.time && h("span", { className: "fl-list-time" }, it.time), h("span", null, it.text));
        })));
    }));
  }
  function StatsRail(p) {
    return h("aside", { className: "fl-stats" },
      h("div", { className: "fl-stats-sec" }, (p.stats || []).map(function (s, i) {
        return h("div", { key: i, className: "fl-stat" }, h("span", { className: "fl-stat-value" }, s.value), h("span", { className: "fl-stat-unit" }, s.unit));
      })),
      p.children);
  }
  function InsightCard(p) {
    return h("div", { className: "fl-card", style: p.style },
      h("div", { style: { display: "flex", alignItems: "flex-start" } },
        h("div", { style: { flex: 1 } }, h("div", { className: "fl-metric" }, p.value), h("div", { className: "fl-metric-cap" }, p.caption)),
        p.trend && h("span", { className: "fl-pill" }, p.trend)),
      p.children);
  }
  function UsageBars(p) {
    var max = Math.max.apply(null, (p.rows || []).map(function (r) { return r.pct; }).concat([1]));
    return h("div", { className: "fl-usage" }, (p.rows || []).map(function (r, i) {
      return h(React.Fragment, { key: i },
        h("span", { style: { color: "var(--foreground)" } }, h(Icon, { name: r.icon || "file" })),
        h("span", { className: "fl-usage-track" },
          h("span", { className: cx("fl-usage-bar", i === 0 ? "is-top" : "is-mid"), style: { width: Math.max(30, Math.round((r.pct / max) * (p.maxWidth || 200))) } }, r.pct + "%"),
          h("span", { className: "fl-usage-label" }, r.label)));
    }));
  }
  function Heatmap(p) {
    var days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"], weeks = p.weeks || [];
    var cells = [];
    days.forEach(function (d, di) {
      cells.push(h("span", { key: "l" + di }, d));
      weeks.forEach(function (w, wi) { var v = w[di]; cells.push(h("i", { key: di + "-" + wi, className: v == null ? "" : "h" + v })); });
    });
    return h("div", null,
      h("div", { className: "fl-heat", style: { "--cols": weeks.length } }, cells),
      h("div", { className: "fl-heat-legend" }, "More", h("i", { className: "h4", style: sw("--fathom-900") }), h("i", { style: sw("--fathom-500") }), h("i", { style: sw("--fathom-400") }), h("i", { style: sw("--fathom-100") }), "Less"));
  }
  function sw(v) { return { display: "inline-block", width: 13, height: 13, borderRadius: 2, background: "var(" + v + ")" }; }

  /* ---------- Surfaces ---------- */
  function Banner(p) {
    return h("div", { className: cx("fl-banner", "fl-banner-" + (p.tone || "night")), style: p.style },
      p.onClose && h("button", { className: "fl-banner-close", "aria-label": "Dismiss", onClick: p.onClose }, h(Icon, { name: "x", size: 14 })),
      h("h3", { className: "fl-banner-title" }, p.title),
      p.body && h("p", { className: "fl-banner-body" }, p.body),
      p.actions && h("div", { className: "fl-banner-actions" }, p.actions));
  }
  function Callout(p) {
    return h("div", { className: cx("fl-callout", "fl-callout-" + (p.tone || "peach")) },
      p.leading, h("div", { className: "fl-grow" }, h("div", { className: "fl-callout-title" }, p.title), p.body && h("div", { className: "fl-callout-body" }, p.body)), p.actions);
  }
  function Checklist(p) {
    var items = p.items || [], done = items.filter(function (i) { return i.done; }).length;
    return h("div", { className: "fl-check-card" },
      h("div", { className: "fl-check-title" }, p.title),
      h("div", { className: "fl-progress" }, h("span", { style: { width: (items.length ? (done / items.length) * 100 : 0) + "%" } })),
      items.map(function (it, i) { return h("div", { key: i, className: cx("fl-check-item", it.done && "is-done") }, h("i"), h("span", null, it.label)); }));
  }
  function EmptyState(p) {
    return h("div", { className: "fl-empty" }, p.icon && h(Icon, { name: p.icon, size: 20 }), h("div", { className: "fl-empty-title" }, p.title), p.body && h("div", { className: "fl-empty-body" }, p.body), p.action);
  }
  function OptionCard(p) {
    return h("button", { type: "button", className: cx("fl-option", p.selected && "is-selected"), "aria-pressed": !!p.selected, onClick: p.onClick },
      h("span", { className: "fl-option-title" }, p.title), h("span", { className: "fl-option-sub" }, p.subtitle),
      p.sample && h("span", { className: "fl-bubble-wrap" }, h("span", { className: "fl-bubble" }, p.sample), h(Avatar, { initial: p.initial || "J", tone: p.avatarTone || "lilac", size: "lg" })));
  }
  function Modal(p) {
    return h("div", { className: "fl-scrim", style: p.style },
      h("div", { className: "fl-modal", role: "dialog", "aria-label": p.title },
        p.nav && h("div", { className: "fl-modal-nav" }, h("div", { className: "fl-modal-nav-top" }, p.nav), p.footer && h("div", { className: "fl-modal-foot" }, p.footer)),
        h("div", { className: "fl-modal-body" }, h("h2", { className: "fl-modal-title" }, p.title), p.children)));
  }
  function Popover(p) { return h("div", { className: "fl-popover", style: p.style }, p.children); }
  function Tooltip(p) { return h("span", { className: cx("fl-tooltip", p.hint && "is-hint"), role: "tooltip" }, p.label); }
  function Avatar(p) { return h("span", { className: cx("fl-avatar", p.size === "lg" && "fl-avatar-lg", p.tone && p.tone !== "purple" && "fl-avatar-" + p.tone) }, p.initial); }

  window.Murmur = Object.assign(window.Murmur || {}, {
    Icon: Icon, Button: Button, IconButton: IconButton, Badge: Badge, Toggle: Toggle, Kbd: Kbd, TextInput: TextInput, Select: Select, Menu: Menu,
    Tabs: Tabs, NavItem: NavItem, Sidebar: Sidebar, SettingsGroup: SettingsGroup, SettingsRow: SettingsRow, HistoryList: HistoryList,
    StatsRail: StatsRail, InsightCard: InsightCard, UsageBars: UsageBars, Heatmap: Heatmap, Banner: Banner, Callout: Callout,
    Checklist: Checklist, EmptyState: EmptyState, OptionCard: OptionCard, Modal: Modal, Popover: Popover, Tooltip: Tooltip, Avatar: Avatar
  });
})();
