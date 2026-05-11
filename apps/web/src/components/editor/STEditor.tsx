import Editor, { type OnMount } from "@monaco-editor/react"
import type { editor, Monaco } from "@monaco-editor/react"
import { useCallback, useEffect, useRef } from "react"

import { useDarkMode } from "@/lib/dark-mode"
import type { CheckDiagnostic } from "@/types/generated/CheckDiagnostic"
import {
  LANGUAGE_ID,
  editorOptions,
  languageConfiguration,
  monarch,
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
