import type { Monaco } from "@monaco-editor/react"

/**
 * Monaco themes for the acid-green tune.
 *
 * Monaco's stock `light` / `vs-dark` fight the design on two axes: they
 * paint a pure-white (#FFFFFF) / blue-black (#1E1E1E) canvas into the
 * middle of our warm neutrals, and their syntax palette is the VS Code
 * blue/orange family. Both read as "an editor someone embedded", which
 * is exactly the seam the design removes — the code surface is supposed
 * to be the same material as the panes around it.
 *
 * Colours below are sampled from the Figma frames, not invented:
 * identifiers sit near the foreground, keywords are violet, IEC types
 * teal, numerics olive. The pairs are luminance-mirrored across themes
 * so code has the same "shape" (which tokens pop) in light and dark.
 */

const LIGHT = {
  bg: "#FAFAF8",
  fg: "#2C2B27",
  comment: "#8C8980",
  keyword: "#6E3C9C",
  type: "#1E7A66",
  number: "#4E7A2C",
  operator: "#6A6963",
  string: "#4E7A2C",
  lineNr: "#ABAAA2",
  lineNrActive: "#575651",
  selection: "#DDE9C9",
  lineHighlight: "#F2F2F0",
} as const

const DARK = {
  bg: "#1C1C1B",
  fg: "#D6D4CF",
  comment: "#7C7A73",
  keyword: "#C49AD8",
  type: "#6FC1A8",
  number: "#A9CE80",
  operator: "#9A988F",
  string: "#A9CE80",
  lineNr: "#5F5E57",
  lineNrActive: "#A3A199",
  selection: "#3A4426",
  lineHighlight: "#232322",
} as const

export const ACID_LIGHT = "ia2-acid-light"
export const ACID_DARK = "ia2-acid-dark"

function rules(p: typeof LIGHT | typeof DARK) {
  // Monaco strips the leading '#' in token rules but not in `colors`.
  const h = (c: string) => c.slice(1)
  return [
    { token: "comment", foreground: h(p.comment), fontStyle: "italic" },
    { token: "keyword", foreground: h(p.keyword) },
    // IEC types (INT / DINT / BOOL / TIME…) — our monarch emits
    // `type.identifier` for these.
    { token: "type.identifier", foreground: h(p.type) },
    { token: "number", foreground: h(p.number) },
    { token: "number.hex", foreground: h(p.number) },
    { token: "number.time", foreground: h(p.number) },
    { token: "string", foreground: h(p.string) },
    { token: "string.escape", foreground: h(p.string) },
    { token: "string.quote", foreground: h(p.string) },
    { token: "operator", foreground: h(p.operator) },
    { token: "delimiter", foreground: h(p.operator) },
    { token: "identifier", foreground: h(p.fg) },
    // Standard-library calls (TON, CTU, …) share the type hue: both
    // are "things the standard gives you", vs. your own identifiers.
    { token: "support.function", foreground: h(p.type) },
  ]
}

function colors(p: typeof LIGHT | typeof DARK) {
  return {
    "editor.background": p.bg,
    "editor.foreground": p.fg,
    "editorLineNumber.foreground": p.lineNr,
    "editorLineNumber.activeForeground": p.lineNrActive,
    "editor.selectionBackground": p.selection,
    "editor.inactiveSelectionBackground": `${p.selection}80`,
    "editor.lineHighlightBackground": p.lineHighlight,
    "editor.lineHighlightBorder": "#00000000",
    "editorCursor.foreground": p.fg,
    "editorWidget.background": p.lineHighlight,
    "editorWidget.border": p.lineNr,
    "editorSuggestWidget.background": p.lineHighlight,
    "editorSuggestWidget.selectedBackground": p.selection,
    "editorGutter.background": p.bg,
    "editorIndentGuide.background1": p.lineHighlight,
    "scrollbarSlider.background": `${p.lineNr}40`,
    "scrollbarSlider.hoverBackground": `${p.lineNr}70`,
    "scrollbarSlider.activeBackground": `${p.lineNr}90`,
  }
}

/** Idempotent — Monaco tolerates re-defining a theme by the same name. */
export function defineAcidThemes(monaco: Monaco) {
  monaco.editor.defineTheme(ACID_LIGHT, {
    base: "vs",
    inherit: true,
    rules: rules(LIGHT),
    colors: colors(LIGHT),
  })
  monaco.editor.defineTheme(ACID_DARK, {
    base: "vs-dark",
    inherit: true,
    rules: rules(DARK),
    colors: colors(DARK),
  })
}
