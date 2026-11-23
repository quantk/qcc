// The editor command in the settings screen, applied at once like the window's other preferences.

import { isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import { detectEditor } from "../../api/editor";
import { Input } from "@/components/ui/input";
import { setEditor, useEditor } from "./preference";

export function EditorSetting({ Setting }: { Setting: React.ComponentType<{ title: string; hint: string; children: React.ReactNode }> }) {
  const saved = useEditor();
  const [value, setValue] = useState(saved);
  const [detected, setDetected] = useState<string | null | undefined>(undefined);
  useEffect(() => setValue(saved), [saved]);
  useEffect(() => {
    if (!isTauri()) return;
    let current = true;
    detectEditor()
      .then((found) => current && setDetected(found))
      .catch(() => current && setDetected(null));
    return () => {
      current = false;
    };
  }, []);
  const fallback =
    detected === undefined
      ? "Empty: the first editor found."
      : detected
        ? `Empty: ${detected}, found on this machine.`
        : "Empty: no editor was found, so the system's default app opens the directory.";
  return (
    <Setting
      title="Editor"
      hint={`What "Open in editor" runs with a thread's directory, as its last argument or in place of {path}: code, zed, idea --new-window. ${fallback}`}
    >
      <Input
        aria-label="Editor command"
        className="w-52"
        placeholder={detected ? (detected.split("/").pop() ?? "Find one") : "Find one"}
        value={value}
        onChange={(event) => setValue(event.target.value)}
        onBlur={() => setEditor(value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") setEditor(value);
        }}
      />
    </Setting>
  );
}
