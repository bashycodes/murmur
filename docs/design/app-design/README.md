# Murmur app design (source of truth for the UI refactor)

Exported from the "Murmur App Design" canvas (claude.ai design artifact),
version 1794116499-a1a1 (v53), 2026-11-08.

The visual look (`flow-theme.css`, the design tokens, the fonts) is kept as it is by owner decision 2026-11-09.

- `canvas.json`: artboard index (titles, sizes, grouping).
- `*.dc.html`: one artboard each. Markup is plain HTML + inline styles; `<sc-for>`/`<sc-if>`
  are repeat/branch templates, `{{x}}` holes are filled from the `renderVals()` script at the bottom.
  `<dc-import name="X">` embeds artboard `X.dc.html`. `<x-import component="Flow.Y">` mounts a
  design-system component from `ds/flow/components/bundle.js` (React; read it for markup).
- `flow-theme.css`: light + `.lw-dark` dark tokens, Android tokens (`.lw-android`), Murmur helper classes.
- `ds/flow/components/bundle.css` + `bundle.js`: the Flow design-system component styles/markup.
- `ds/flow/tokens.json`: full token set.

Groups: App pages (desktop, 1280x800), Settings (1040 wide), Dark mode, First-run setup,
Android (390x844).
