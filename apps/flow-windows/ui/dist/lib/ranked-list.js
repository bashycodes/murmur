// The ranked engine list shared by the speech-to-text order, the meeting
// transcription order (D47) and the cleanup order (D36): rows built with
// `engineRow` (pages/settings/speech.ts), laid out in the saved order,
// numbered among the shown ones, dimmed when switched off, and moved one
// shown row up or down. Each feature owns its data, switches and saving.
/** Put the rows in `order` (DOM order = try order) and rank the shown ones.
 *  Returns the shown keys in order. */
export function rankRows(l, order) {
    const shown = order.filter((k) => l.shown(k));
    for (const k of order) {
        const row = l.row(k);
        l.list.insertBefore(row, l.before ?? null);
        row.hidden = !shown.includes(k);
    }
    shown.forEach((k, i) => {
        const row = l.row(k);
        row.classList.toggle("is-off", !l.on(k));
        row.querySelector(".lw-rank").textContent = String(i + 1);
        row.querySelector("[data-move=up]").disabled = i === 0;
        row.querySelector("[data-move=down]").disabled = i === shown.length - 1;
        row.querySelector(".lw-move").hidden = shown.length < 2;
    });
    return shown;
}
/** `items` with the one whose key is `key` swapped with the next shown item
 *  up (-1) or down (1); null when it is already at that end. */
export function moveShown(items, keyOf, shown, key, dir) {
    const list = [...items];
    const vis = list.filter((t) => shown(keyOf(t)));
    const i = vis.findIndex((t) => keyOf(t) === key);
    const other = vis[i + dir];
    if (i < 0 || !other)
        return null;
    const a = list.indexOf(vis[i]);
    const b = list.indexOf(other);
    [list[a], list[b]] = [list[b], list[a]];
    return list;
}
/** Wire every row's up/down buttons in `list`: `attr` is the data attribute
 *  that holds the row's key (e.g. `route` for `data-route`). */
export function onMove(list, attr, move) {
    for (const b of list.querySelectorAll("[data-move]")) {
        const key = b.closest(`[data-${attr}]`).dataset[attr];
        b.addEventListener("click", () => move(key, b.dataset.move === "up" ? -1 : 1));
    }
}
