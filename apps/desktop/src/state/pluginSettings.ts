// A plugin's own settings: the fields it declares (`PluginInfo.settings`) and the
// values the user set, kept in the daemon's settings by plugin and field id.

import type { PluginInfo, SettingField, Settings } from "../api/protocol.gen";

/** What the user set for one of the plugin's settings, or null if they did not. */
export function pluginSetting(settings: Settings, plugin: string, field: string): string | null {
  return settings.plugins?.[plugin]?.[field] ?? null;
}

/** What the plugin is told: what the user set, or else the default. */
export function effectiveSetting(settings: Settings, plugin: PluginInfo, field: string): string | null {
  const declared = plugin.settings?.find((candidate) => candidate.id === field);
  return pluginSetting(settings, plugin.id, field) ?? declared?.default ?? null;
}

/** Set one of a plugin's settings, or (`null`, empty text) go back to its default. */
export function withPluginSetting(settings: Settings, plugin: string, field: string, value: string | null): Settings {
  const values = { ...settings.plugins?.[plugin] };
  if (value === null || value === "") delete values[field];
  else values[field] = value;
  const plugins = { ...settings.plugins };
  if (Object.keys(values).length === 0) delete plugins[plugin];
  else plugins[plugin] = values;
  return { ...settings, plugins };
}

/** The label of a setting's default, for a field that shows it when nothing is set. */
export function defaultLabel(field: SettingField): string {
  return field.default ? `Default (${field.default})` : "Not set";
}
