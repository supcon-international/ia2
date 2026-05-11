import Editor, { type OnMount } from "@monaco-editor/react"
import { useCallback, useEffect, useState } from "react"

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
}

export function STEditor({ value, onChange }: Props) {
  // Re-render when matchMedia changes so the theme tracks system pref.
  const [theme, setTheme] = useState(() =>
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "vs-dark"
      : "light"
  )

  useEffect(() => {
    if (typeof window === "undefined") return
    const mq = window.matchMedia("(prefers-color-scheme: dark)")
    const handler = (e: MediaQueryListEvent) =>
      setTheme(e.matches ? "vs-dark" : "light")
    mq.addEventListener("change", handler)
    return () => mq.removeEventListener("change", handler)
  }, [])

  const handleMount: OnMount = useCallback((_editor, monaco) => {
    if (languageRegistered) return
    monaco.languages.register({ id: LANGUAGE_ID })
    monaco.languages.setMonarchTokensProvider(LANGUAGE_ID, monarch)
    monaco.languages.setLanguageConfiguration(LANGUAGE_ID, languageConfiguration)
    languageRegistered = true
  }, [])

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
