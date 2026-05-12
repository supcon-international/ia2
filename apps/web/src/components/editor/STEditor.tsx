import Editor, { type Monaco, type OnMount } from "@monaco-editor/react"
import type { editor } from "monaco-editor"
import { useCallback, useEffect, useRef } from "react"

import { useDarkMode } from "@/lib/dark-mode"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import {
  LANGUAGE_ID,
  builtins,
  editorOptions,
  keywords,
  languageConfiguration,
  monarch,
  typeKeywords,
} from "./iec61131-language"

let languageRegistered = false

type Props = {
  value: string
  onChange: (value: string) => void
  diagnostics: CheckDiagnostic[]
}

export function STEditor({ value, onChange, diagnostics }: Props) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null)
  const monacoRef = useRef<Monaco | null>(null)
  const dark = useDarkMode()
  const theme = dark === "dark" ? "vs-dark" : "light"

  const handleMount: OnMount = useCallback((editorInstance, monaco) => {
    editorRef.current = editorInstance
    monacoRef.current = monaco
    if (languageRegistered) return
    monaco.languages.register({ id: LANGUAGE_ID })
    monaco.languages.setMonarchTokensProvider(LANGUAGE_ID, monarch)
    monaco.languages.setLanguageConfiguration(LANGUAGE_ID, languageConfiguration)

    // Keyword / type / builtin completion. Variable-name completion
    // would need parsing current source — left for when the ironplc LSP
    // advertises a completionProvider capability (today it doesn't).
    const Kind = monaco.languages.CompletionItemKind
    const detailKeyword = "keyword"
    const detailType = "IEC type"
    const detailBuiltin = "stdlib"
    monaco.languages.registerCompletionItemProvider(LANGUAGE_ID, {
      triggerCharacters: [],
      provideCompletionItems(model: editor.ITextModel, position: { lineNumber: number; column: number }) {
        const word = model.getWordUntilPosition(position)
        const range = {
          startLineNumber: position.lineNumber,
          endLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endColumn: word.endColumn,
        }
        const make = (
          label: string,
          kind: number,
          detail: string,
        ) => ({ label, kind, insertText: label, detail, range })
        return {
          suggestions: [
            ...keywords.map((k) => make(k, Kind.Keyword, detailKeyword)),
            ...typeKeywords.map((t) =>
              make(t, Kind.TypeParameter, detailType),
            ),
            ...builtins.map((b) => make(b, Kind.Function, detailBuiltin)),
          ],
        }
      },
    })
    languageRegistered = true
  }, [])

  // Push diagnostics into Monaco's marker pool whenever they change.
  useEffect(() => {
    const editor = editorRef.current
    const monaco = monacoRef.current
    if (!editor || !monaco) return
    const model = editor.getModel()
    if (!model) return
    const markers = diagnostics.map((d) => ({
      severity: monaco.MarkerSeverity.Error,
      message: `${d.code}: ${d.message}`,
      startLineNumber: d.start_line,
      startColumn: d.start_column,
      endLineNumber: d.end_line,
      endColumn: Math.max(d.end_column, d.start_column + 1),
      code: d.code,
      source: "ironplc",
    }))
    monaco.editor.setModelMarkers(model, "iec61131", markers)
  }, [diagnostics])

  return (
    <Editor
      height="100%"
      width="100%"
      language={LANGUAGE_ID}
      value={value}
      theme={theme}
      onChange={(v) => onChange(v ?? "")}
      onMount={handleMount}
      options={editorOptions}
    />
  )
}
