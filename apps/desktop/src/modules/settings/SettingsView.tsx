import { open } from "@tauri-apps/plugin-dialog";
import { CircleAlertIcon, CircleCheckIcon, FolderOpenIcon, MinusIcon, PlusIcon, RefreshCwIcon, XIcon } from "lucide-react";
import { useMemo, useState } from "react";

import type {
  PermissionMode,
  PluginInfo,
  ProgramStatus,
  ProviderInfo,
  SettingField,
  Settings,
} from "../../api/protocol.gen";
import { defaultLabel, effectiveSetting, pluginSetting, withPluginSetting } from "../../state/pluginSettings";
import { modeLabel, providersOf, visiblePlugins } from "../../state/selectors";
import { envOf, envRows, providerSettings, withProgram, withProvider } from "../../state/settings";
import { store, useApp } from "../../state/store";
import { MAX_ZOOM, MIN_ZOOM, formatZoom, zoomIn, zoomOut } from "../../state/zoom";
import { Choice, EffortPicker, EnvEditor, ModelPicker } from "../../ui/kit";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Field, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { LAYOUT_SECTION, LayoutSettings } from "./LayoutSettings";
import { setZoom, useZoom } from "./zoom";

function Setting(props: { title: string; hint: string; children: React.ReactNode }) {
  return (
    <section className="flex items-center justify-between gap-6 border-b py-4 last:border-b-0">
      <div className="min-w-0">
        <h3 className="text-sm font-semibold">{props.title}</h3>
        <p className="text-[13px] text-muted-foreground">{props.hint}</p>
      </div>
      <div className="flex shrink-0 items-center gap-2">{props.children}</div>
    </section>
  );
}

function Section(props: { title: string; hint?: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2 pt-6">
      <div>
        <h2 className="text-base font-semibold">{props.title}</h2>
        {props.hint && <p className="text-[13px] text-muted-foreground">{props.hint}</p>}
      </div>
      {props.children}
    </section>
  );
}

/** Daemon settings are edited as a draft and saved together; window preferences apply at once. */
export function SettingsView() {
  const loaded = useApp((state) => state.settings);
  const allPlugins = useApp((state) => state.allPlugins);
  const showExamples = useApp((state) => state.showExamples);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const settings = draft ?? loaded?.settings ?? {};
  const dirty =
    draft !== null &&
    JSON.stringify({ ...draft, mcp_servers: undefined, skills: undefined }) !==
      JSON.stringify({ ...(loaded?.settings ?? {}), mcp_servers: undefined, skills: undefined });
  const plugins = useMemo(() => visiblePlugins(allPlugins, showExamples), [allPlugins, showExamples]);
  const providers = useMemo(() => providersOf(plugins), [plugins]);

  const save = async () => {
    if (!draft) return;
    setSaving(true);
    // MCP servers and skills have screens of their own and are saved from there at once.
    const latest = store().getState().settings?.settings;
    if (await store().saveSettings({ ...draft, mcp_servers: latest?.mcp_servers, skills: latest?.skills })) {
      setDraft(null);
    }
    setSaving(false);
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b px-6 pt-3.5 pb-3">
        <h1 className="text-lg">Settings</h1>
        <span className="flex-1" />
        {dirty && (
          <>
            <span className="text-[13px] text-muted-foreground">Unsaved changes</span>
            <Button variant="outline" size="sm" onClick={() => setDraft(null)}>
              Discard
            </Button>
            <Button size="sm" disabled={saving} onClick={() => void save()}>
              {saving ? "Saving…" : "Save"}
            </Button>
          </>
        )}
      </header>
      <div className="w-full max-w-3xl overflow-auto px-6 pb-10">
        <Section title="General">
          <Setting title="Default agent" hint="What a new thread starts with, here and in the CLI.">
            <Choice
              aria-label="Default agent"
              className="w-52"
              value={settings.default_provider ?? null}
              options={[
                { value: null, label: "Last used" },
                ...providers.map((provider) => ({ value: provider.id, label: provider.name })),
              ]}
              onChange={(value) => setDraft({ ...settings, default_provider: value })}
            />
          </Setting>
          <ExamplesSetting />
          <ZoomSetting />
        </Section>
        <div id={LAYOUT_SECTION}>
          <Section title="Layout" hint="Which blocks the window shows and where: screens, sidebar, a thread's tabs and its side panel.">
            <LayoutSettings />
          </Section>
        </div>
        <Section
          title="Agents"
          hint="Where each agent's program is, and what its threads start with. Saved in the daemon, so the CLI uses them too."
        >
          {providers.map((provider) => (
            <AgentSettings
              key={provider.id}
              provider={provider}
              plugin={plugins.find((plugin) => plugin.providers.some((own) => own.id === provider.id))}
              programs={loaded?.programs ?? []}
              settings={settings}
              change={setDraft}
            />
          ))}
        </Section>
        <Section title="Plugins" hint="What is loaded, what each one adds and what it is allowed to do.">
          {allPlugins.map((plugin) => (
            <PluginCard key={plugin.id} plugin={plugin} settings={settings} providers={providers} change={setDraft} />
          ))}
        </Section>
      </div>
    </div>
  );
}

function ExamplesSetting() {
  const showExamples = useApp((state) => state.showExamples);
  return (
    <Setting
      title="Show example plugins"
      hint="Echo is the test agent that repeats what it is sent, and the example of the plugin API (its Echo canvas screen and tabs). It costs nothing and is not a real agent."
    >
      <Switch
        aria-label="Show example plugins"
        checked={showExamples}
        onCheckedChange={(checked) => store().setShowExamples(checked)}
      />
    </Setting>
  );
}

function ZoomSetting() {
  const zoom = useZoom();
  return (
    <Setting title="Interface scale" hint="Ctrl+Plus and Ctrl+Minus change it from anywhere, as in a browser.">
      <Button variant="outline" size="icon" aria-label="Zoom out" disabled={zoom <= MIN_ZOOM} onClick={() => setZoom(zoomOut(zoom))}>
        <MinusIcon />
      </Button>
      <output className="w-12 text-center text-sm tabular-nums" aria-live="polite">
        {formatZoom(zoom)}
      </output>
      <Button variant="outline" size="icon" aria-label="Zoom in" disabled={zoom >= MAX_ZOOM} onClick={() => setZoom(zoomIn(zoom))}>
        <PlusIcon />
      </Button>
      <Button variant="ghost" disabled={zoom === 1} onClick={() => setZoom(1)}>
        Reset
      </Button>
    </Setting>
  );
}

function AgentSettings(props: {
  provider: ProviderInfo;
  plugin: PluginInfo | undefined;
  programs: readonly ProgramStatus[];
  settings: Settings;
  change: (settings: Settings) => void;
}) {
  const { provider, settings, change } = props;
  const own = providerSettings(settings, provider.id);
  const programs = props.programs.filter((program) => props.plugin && program.plugins.includes(props.plugin.id));
  const modes = provider.capabilities.permission_modes ?? [];
  const update = (patch: Parameters<typeof withProvider>[2]) => change(withProvider(settings, provider.id, patch));

  return (
    <article className="flex flex-col gap-3 rounded-lg border p-4">
      <header className="flex items-center gap-2">
        <h3 className="font-semibold">{provider.name}</h3>
        {provider.id !== provider.name && (
          <Badge variant="outline" className="font-mono">
            {provider.id}
          </Badge>
        )}
        <span className="flex-1" />
        <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
          Offer in pickers
          <Switch
            aria-label={`Offer ${provider.name} in pickers`}
            checked={!own.hidden}
            onCheckedChange={(checked) => update({ hidden: !checked })}
          />
        </label>
      </header>
      {programs.map((program) => (
        <ProgramRow
          key={program.name}
          program={program}
          value={settings.programs?.[program.name] ?? ""}
          change={(path) => change(withProgram(settings, program.name, path))}
        />
      ))}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Field>
          <FieldLabel>Model of new threads</FieldLabel>
          <ModelPicker provider={provider.id} value={own.model ?? null} onChange={(model) => update({ model })} />
        </Field>
        <Field>
          <FieldLabel>Effort of new threads</FieldLabel>
          <EffortPicker
            provider={provider.id}
            model={own.model ?? null}
            value={own.effort ?? null}
            onChange={(effort) => update({ effort })}
          />
        </Field>
        <Field>
          <FieldLabel>Permissions</FieldLabel>
          <Choice
          aria-label="Default permissions"
          className="w-full"
          value={own.permission_mode ?? null}
          disabled={modes.length === 0}
          options={[
            { value: null, label: "Agent's own configuration" },
            ...modes.map((mode) => ({ value: mode, label: modeLabel[mode] })),
          ]}
          onChange={(mode) => update({ permission_mode: mode as PermissionMode | null })}
          />
        </Field>
      </div>
      <Field>
        <FieldLabel>Environment of its sessions</FieldLabel>
        <EnvEditor
          rows={envRows(own.env)}
          change={(rows) => update({ env: envOf(rows) })}
          title="A variable every session of this agent sees: an API key, a config directory"
        />
      </Field>
    </article>
  );
}

function ProgramRow(props: { program: ProgramStatus; value: string; change: (path: string | null) => void }) {
  const { program, value, change } = props;
  const browse = async () => {
    const picked = await open({ directory: false, multiple: false, title: `The ${program.name} program` });
    if (typeof picked === "string") change(picked);
  };
  return (
    <Field>
      <FieldLabel>
        Program <code>{program.name}</code>
      </FieldLabel>
      <div className="flex items-center gap-2">
        <Input
          aria-label={`Path of ${program.name}`}
          value={value}
          onChange={(event) => change(event.target.value)}
          placeholder={program.path ? "From PATH" : "Not found on PATH: give the full path"}
          spellCheck={false}
          className="font-mono text-xs"
        />
        <Button variant="outline" size="icon" aria-label={`Choose the ${program.name} file`} onClick={() => void browse()}>
          <FolderOpenIcon />
        </Button>
        {value && (
          <Button variant="ghost" size="icon" aria-label="Look it up on PATH again" title="Look it up on PATH again" onClick={() => change(null)}>
            <XIcon />
          </Button>
        )}
      </div>
      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
        {program.path ? (
          <>
            <CircleCheckIcon className="size-3.5 text-(--green)" /> Runs {program.path}
          </>
        ) : (
          <>
            <CircleAlertIcon className="size-3.5 text-destructive" /> {program.error ?? "Not found"}
          </>
        )}
        <Button variant="ghost" size="icon-xs" aria-label="Check again" title="Check again" onClick={() => void store().reloadSettings()}>
          <RefreshCwIcon />
        </Button>
      </p>
    </Field>
  );
}

function PluginCard(props: {
  plugin: PluginInfo;
  settings: Settings;
  providers: readonly ProviderInfo[];
  change: (settings: Settings) => void;
}) {
  const { plugin } = props;
  const { permissions } = plugin;
  const allowed = [
    permissions.deltas && "timeline",
    permissions.kv && "storage",
    permissions.timers && "timers",
    permissions.threads && "threads",
    permissions.skills && "skills",
    permissions.process.length > 0 && `runs ${permissions.process.join(", ")}`,
  ].filter(Boolean);
  const adds = [
    ...plugin.providers.map((provider) => `agent ${provider.name}`),
    ...plugin.environments.map((environment) => `workspace ${environment.name}`),
    ...plugin.commands.map((command) => `command ${command.title}`),
    ...plugin.skills.map((skill) => `skill ${skill}`),
  ];
  return (
    <article className="flex flex-col gap-1 rounded-lg border px-4 py-3 text-sm">
      <header className="flex items-center gap-2">
        <h3 className="font-semibold">{plugin.name ?? plugin.id}</h3>
        <span className="font-mono text-xs text-muted-foreground">
          {plugin.id} {plugin.version}
        </span>
        <span className="flex-1" />
        {plugin.bundled && <Badge variant="secondary">bundled</Badge>}
        {plugin.example && <Badge variant="outline">example</Badge>}
      </header>
      {adds.length > 0 && <p className="text-muted-foreground">Adds: {adds.join(" · ")}</p>}
      <p className="text-muted-foreground">May: {allowed.length > 0 ? allowed.join(" · ") : "nothing beyond logging"}</p>
      {(plugin.settings ?? []).length > 0 && (
        <div className="mt-2 grid grid-cols-1 gap-3 sm:grid-cols-2">
          {(plugin.settings ?? []).map((field) => (
            <PluginSettingField
              key={field.id}
              plugin={plugin}
              field={field}
              settings={props.settings}
              providers={props.providers}
              change={(value) => props.change(withPluginSetting(props.settings, plugin.id, field.id, value))}
            />
          ))}
        </div>
      )}
    </article>
  );
}

/** One of a plugin's own settings, asked for the way its kind says. */
function PluginSettingField(props: {
  plugin: PluginInfo;
  field: SettingField;
  settings: Settings;
  providers: readonly ProviderInfo[];
  change: (value: string | null) => void;
}) {
  const { plugin, field, settings, change } = props;
  const value = pluginSetting(settings, plugin.id, field.id);
  const id = `plugin-${plugin.id}-${field.id}`;
  const other = (name: string) => effectiveSetting(settings, plugin, name);
  const { kind } = field;
  let control: React.ReactNode;
  switch (kind.type) {
    case "text":
      control = (
        <Input
          id={id}
          value={value ?? ""}
          placeholder={field.default ?? ""}
          spellCheck={false}
          onChange={(event) => change(event.target.value)}
        />
      );
      break;
    case "flag":
      control = (
        <Switch
          id={id}
          aria-label={field.label}
          checked={(value ?? field.default) === "true"}
          onCheckedChange={(checked) => change(checked ? "true" : "false")}
        />
      );
      break;
    case "choice":
      control = (
        <Choice
          id={id}
          aria-label={field.label}
          className="w-full"
          value={value}
          options={[{ value: null, label: defaultLabel(field) }, ...kind.options.map((option) => ({ value: option, label: option }))]}
          onChange={change}
        />
      );
      break;
    case "provider":
      control = (
        <Choice
          id={id}
          aria-label={field.label}
          className="w-full"
          value={value}
          options={[
            { value: null, label: defaultLabel(field) },
            ...props.providers.map((provider) => ({ value: provider.id, label: provider.name })),
          ]}
          onChange={change}
        />
      );
      break;
    case "model":
      control = <ModelPicker id={id} provider={other(kind.provider) ?? undefined} value={value} onChange={change} />;
      break;
    case "effort":
      control = (
        <EffortPicker
          id={id}
          provider={other(kind.provider) ?? undefined}
          model={other(kind.model)}
          value={value}
          onChange={change}
        />
      );
      break;
    case "permission-mode":
      control = (
        <Choice
          id={id}
          aria-label={field.label}
          className="w-full"
          value={value}
          options={[
            { value: null, label: defaultLabel(field) },
            ...Object.entries(modeLabel).map(([mode, label]) => ({ value: mode, label })),
          ]}
          onChange={change}
        />
      );
      break;
  }
  return (
    <Field>
      <FieldLabel htmlFor={id}>{field.label}</FieldLabel>
      {control}
      {field.description && <p className="text-xs text-muted-foreground">{field.description}</p>}
    </Field>
  );
}
