import type { editor, languages } from "monaco-editor"

export const LANGUAGE_ID = "iec61131"

export const keywords = [
  "PROGRAM", "END_PROGRAM",
  "FUNCTION", "END_FUNCTION",
  "FUNCTION_BLOCK", "END_FUNCTION_BLOCK",
  "TYPE", "END_TYPE",
  "STRUCT", "END_STRUCT",
  "VAR", "VAR_INPUT", "VAR_OUTPUT", "VAR_IN_OUT", "VAR_GLOBAL",
  "VAR_TEMP", "VAR_EXTERNAL", "VAR_ACCESS", "END_VAR",
  "CONSTANT", "RETAIN", "NON_RETAIN", "PERSISTENT", "AT",
  "CONFIGURATION", "END_CONFIGURATION",
  "RESOURCE", "ON", "END_RESOURCE",
  "TASK", "INTERVAL", "PRIORITY", "SINGLE", "WITH",
  "IF", "THEN", "ELSIF", "ELSE", "END_IF",
  "CASE", "OF", "END_CASE",
  "FOR", "TO", "BY", "DO", "END_FOR",
  "WHILE", "END_WHILE",
  "REPEAT", "UNTIL", "END_REPEAT",
  "EXIT", "CONTINUE", "RETURN",
  "TRUE", "FALSE",
  "NOT", "AND", "OR", "XOR", "MOD", "DIV",
  "REF_TO", "REF", "NULL",
]

export const typeKeywords = [
  "BOOL", "BYTE", "WORD", "DWORD", "LWORD",
  "SINT", "INT", "DINT", "LINT",
  "USINT", "UINT", "UDINT", "ULINT",
  "REAL", "LREAL",
  "TIME", "LTIME", "DATE", "LDATE",
  "TIME_OF_DAY", "TOD", "LTOD",
  "DATE_AND_TIME", "DT", "LDT",
  "STRING", "WSTRING", "CHAR", "WCHAR",
  "ARRAY", "ANY", "ANY_INT", "ANY_NUM", "ANY_REAL", "ANY_BIT", "ANY_STRING",
]

export const builtins = [
  "ABS", "SQRT", "LN", "LOG", "EXP",
  "SIN", "COS", "TAN", "ASIN", "ACOS", "ATAN",
  "MIN", "MAX", "LIMIT", "MUX", "SEL",
  "LEN", "LEFT", "RIGHT", "MID", "CONCAT", "INSERT", "DELETE", "REPLACE", "FIND",
  "TON", "TOF", "TP", "R_TRIG", "F_TRIG", "CTU", "CTD", "CTUD",
  "SHL", "SHR", "ROL", "ROR",
]

export const monarch: languages.IMonarchLanguage = {
  defaultToken: "",
  ignoreCase: true,
  keywords,
  typeKeywords,
  builtins,
  operators: [
    ":=", "=>", "..",
    "=", "<>", "<", ">", "<=", ">=",
    "+", "-", "*", "/", "**",
  ],
  symbols: /[=><!~?:&|+\-*/^%]+/,
  tokenizer: {
    root: [
      // time literals: T#100ms, LTIME#1h2m3s, TIME#...
      [/(?:L?TIME|L?T)#[\d._smhd]+/i, "number.time"],
      // typed literals: BOOL#TRUE, INT#42, 16#FF, 2#0101
      [/(?:[A-Z_]+#)?[0-9]+(?:_[0-9]+)*(?:\.[0-9]+)?(?:e[+-]?[0-9]+)?/i, "number"],
      [/[0-9]+#[0-9A-F_]+/i, "number.hex"],

      // identifiers + keywords
      [
        /[a-zA-Z_][\w]*/,
        {
          cases: {
            "@keywords": "keyword",
            "@typeKeywords": "type.identifier",
            "@builtins": "support.function",
            "@default": "identifier",
          },
        },
      ],

      // whitespace
      { include: "@whitespace" },

      // strings
      [/'/, { token: "string.quote", bracket: "@open", next: "@stringSingle" }],
      [/"/, { token: "string.quote", bracket: "@open", next: "@stringDouble" }],

      // punctuation
      [/[{}()[\]]/, "@brackets"],
      [/[<>](?!@symbols)/, "@brackets"],
      [
        /@symbols/,
        {
          cases: {
            "@operators": "operator",
            "@default": "",
          },
        },
      ],
      [/[;,.]/, "delimiter"],
    ],

    whitespace: [
      [/[ \t\r\n]+/, ""],
      [/\(\*/, "comment", "@commentBlock"],
      [/\/\/.*$/, "comment"],
    ],
    commentBlock: [
      [/[^*(]+/, "comment"],
      [/\*\)/, "comment", "@pop"],
      [/[*(]/, "comment"],
    ],
    stringSingle: [
      [/[^\\']+/, "string"],
      [/\\./, "string.escape"],
      [/'/, { token: "string.quote", bracket: "@close", next: "@pop" }],
    ],
    stringDouble: [
      [/[^\\"]+/, "string"],
      [/\\./, "string.escape"],
      [/"/, { token: "string.quote", bracket: "@close", next: "@pop" }],
    ],
  },
}

export const languageConfiguration: languages.LanguageConfiguration = {
  comments: {
    blockComment: ["(*", "*)"],
    lineComment: "//",
  },
  brackets: [
    ["{", "}"],
    ["[", "]"],
    ["(", ")"],
  ],
  autoClosingPairs: [
    { open: "(*", close: "*)" },
    { open: "(", close: ")" },
    { open: "[", close: "]" },
    { open: "{", close: "}" },
    { open: "'", close: "'", notIn: ["string", "comment"] },
    { open: '"', close: '"', notIn: ["string", "comment"] },
  ],
  surroundingPairs: [
    { open: "(", close: ")" },
    { open: "[", close: "]" },
    { open: "{", close: "}" },
    { open: "'", close: "'" },
    { open: '"', close: '"' },
  ],
}

export const editorOptions: editor.IStandaloneEditorConstructionOptions = {
  // Same stack as the rest of the app (--font-mono in styles.css). If
  // Monaco kept its own fallback list, the editor's glyphs would drift
  // from the tree / monitor mono and the seam would be visible.
  fontFamily:
    '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, monospace',
  fontSize: 13,
  lineNumbers: "on",
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  smoothScrolling: true,
  renderWhitespace: "selection",
  tabSize: 2,
  insertSpaces: true,
  automaticLayout: true,
  padding: { top: 8, bottom: 8 },
  // The design's code block is airier than Monaco's default 1.35×.
  lineHeight: 21,
  renderLineHighlight: "line",
  guides: { indentation: false },
}
