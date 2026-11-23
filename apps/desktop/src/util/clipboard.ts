// Writing text to the clipboard. The webview's own clipboard API works in Tauri's
// windows (the app's other copy actions use it too), so no clipboard plugin is
// needed; a refusal is reported rather than thrown.

/** Copy text; whether the clipboard took it. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
