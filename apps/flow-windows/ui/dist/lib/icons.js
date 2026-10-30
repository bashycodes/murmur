// Stroke icons from the design (docs/design/app-design: AppSidebar,
// Settings*, Main), as 24x24 path data for .fl-icon <svg>s.
export const ICON = {
    dictation: "M9 6a3 3 0 0 1 6 0v5a3 3 0 0 1-6 0zM5 11a7 7 0 0 0 14 0M12 18v3",
    notetaker: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z",
    insights: "M4 20h16M7 16V10M12 16V5M17 16v-4",
    dictionary: "M6 3h12v18H6zM9.5 8h5M9.5 12h5",
    snippets: "M6 3.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5zM6 15.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5zM8 7.5L20 18M8 16.5L20 6",
    style: "M3 7V5h10v2M8 5v14M6 19h4M14 12v-1h7v1M17.5 11v8M16 19h3",
    settings: "M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M5.3 18.7l2.1-2.1M16.6 7.4l2.1-2.1",
    general: "M4 7h9M17 7h3M4 17h3M11 17h9M15 5v4M9 15v4",
    system: "M3 4h18v12H3zM8 20h8M12 16v4",
    privacy: "M12 3l7 3v6c0 4.5-3 7.5-7 9c-4-1.5-7-4.5-7-9V6zM9 12l2 2 4-4",
    cleanup: "M4 20L15 9M17 3l1 2 2 1-2 1-1 2-1-2-2-1 2-1z",
    server: "M4 4h16v6H4zM4 14h16v6H4zM8 7h.01M8 17h.01",
    cloud: "M7 18a4 4 0 0 1-.5-7.97A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z",
    chip: "M6 6h12v12H6zM9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4",
    developer: "M8 7l-5 5 5 5M16 7l5 5-5 5",
    close: "M6 6l12 12M18 6L6 18",
    copy: "M8 8h12v12H8zM4 16V4h12",
    paste: "M9 4h6v3H9zM7 5.5H5V21h14V5.5h-2",
    undo: "M9 14L4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3",
    trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
    up: "M6 15l6-6 6 6",
    down: "M6 9l6 6 6-6",
    plus: "M12 5v14M5 12h14",
    search: "M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14zM20 20l-4-4",
    retry: "M20 12a8 8 0 1 1-2.34-5.66M20 4v5h-5",
};
/** Markup for a static template. */
export function iconSvg(path, size = 20) {
    return `<svg class="fl-icon" viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true"><path d="${path}"></path></svg>`;
}
/** An icon element for code that builds DOM. */
export function iconEl(path, size = 16) {
    const NS = "http://www.w3.org/2000/svg";
    const s = document.createElementNS(NS, "svg");
    s.setAttribute("class", "fl-icon");
    s.setAttribute("viewBox", "0 0 24 24");
    s.setAttribute("width", String(size));
    s.setAttribute("height", String(size));
    s.setAttribute("aria-hidden", "true");
    const p = document.createElementNS(NS, "path");
    p.setAttribute("d", path);
    s.append(p);
    return s;
}
