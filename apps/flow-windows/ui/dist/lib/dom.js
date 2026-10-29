// Small DOM helpers shared by every page. User text always goes in with
// textContent; `html()` is only for the pages' own static templates.
export const $ = (id) => document.getElementById(id);
export function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls)
        e.className = cls;
    if (text !== undefined)
        e.textContent = text;
    return e;
}
export function button(label, onClick, cls = "small") {
    const b = el("button", cls, label);
    b.type = "button";
    b.addEventListener("click", onClick);
    return b;
}
export function setStatus(id, text, ok) {
    const s = $(id);
    s.textContent = text;
    s.className = ok ? "ok" : "err";
}
/** A page's static markup (never user text) as nodes. */
export function html(markup) {
    const t = document.createElement("template");
    t.innerHTML = markup.trim();
    return t.content;
}
const SVG_NS = "http://www.w3.org/2000/svg";
export function svg(tag, attrs, text) {
    const e = document.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(attrs))
        e.setAttribute(k, String(v));
    if (text !== undefined)
        e.textContent = text;
    return e;
}
/** Show a <dialog> with an OK and a Cancel button; resolves to the choice. */
export function askDialog(id, okId, cancelId) {
    const d = $(id);
    return new Promise((resolve) => {
        const done = (ok) => {
            $(okId).onclick = null;
            $(cancelId).onclick = null;
            d.oncancel = null;
            d.close();
            resolve(ok);
        };
        $(okId).onclick = () => done(true);
        $(cancelId).onclick = () => done(false);
        d.oncancel = (e) => {
            e.preventDefault();
            done(false);
        };
        d.showModal();
    });
}
/** Time today, else date + time. */
export function when(ts) {
    const d = new Date(ts);
    const today = new Date();
    return d.toDateString() === today.toDateString()
        ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : d.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}
/** Set a field unless the user is typing in it. */
export function setValue(id, v) {
    const i = $(id);
    if (document.activeElement !== i)
        i.value = v;
}
