// The window's own memory. Storage may be missing or refuse writes (private
// windows, a full quota); then nothing is remembered past this window, and the app
// still works until it closes.

/** The text saved under `key`, or null when there is none or it cannot be read. */
export function readText(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

/** The value saved under `key` as JSON, or null when there is none or it does not parse. */
export function readJson(key: string): unknown {
  const text = readText(key);
  if (!text) return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

export function saveText(key: string, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // Not remembered past this window.
  }
}

export function saveJson(key: string, value: unknown): void {
  saveText(key, JSON.stringify(value));
}
