// Edits of the daemon's settings, made on a draft that is saved as a whole.

import type { ProviderId, ProviderSettings, Settings } from "../api/protocol.gen";

export function providerSettings(settings: Settings, provider: ProviderId): ProviderSettings {
  return settings.providers?.[provider] ?? {};
}

/** Change one agent's settings; an agent left with nothing set is dropped. */
export function withProvider(settings: Settings, provider: ProviderId, patch: Partial<ProviderSettings>): Settings {
  const next: ProviderSettings = { ...providerSettings(settings, provider), ...patch };
  const providers = { ...settings.providers };
  const empty =
    !next.model &&
    !next.effort &&
    !next.permission_mode &&
    !next.hidden &&
    Object.keys(next.env ?? {}).length === 0;
  if (empty) delete providers[provider];
  else providers[provider] = next;
  return { ...settings, providers };
}

/** Say where a program lives, or (`null`, empty) go back to the PATH. */
export function withProgram(settings: Settings, name: string, path: string | null): Settings {
  const programs = { ...settings.programs };
  if (path?.trim()) programs[name] = path.trim();
  else delete programs[name];
  return { ...settings, programs };
}

/** Environment variables as editable rows, and back. Rows without a name are dropped. */
export type EnvRow = { name: string; value: string };

export function envRows(env: { [name: string]: string } | undefined): EnvRow[] {
  return Object.entries(env ?? {}).map(([name, value]) => ({ name, value }));
}

export function envOf(rows: readonly EnvRow[]): { [name: string]: string } {
  return Object.fromEntries(rows.filter((row) => row.name.trim()).map((row) => [row.name.trim(), row.value]));
}
