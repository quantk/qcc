import { MinusIcon, PlusIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { store, useApp } from "../../state/store";
import { MAX_ZOOM, MIN_ZOOM, formatZoom, zoomIn, zoomOut } from "../../state/zoom";
import { setZoom, useZoom } from "./zoom";

function Setting(props: { title: string; hint: string; children: React.ReactNode }) {
  return (
    <section className="flex items-center justify-between gap-6 border-b py-4">
      <div className="min-w-0">
        <h2 className="text-sm font-semibold">{props.title}</h2>
        <p className="text-[13px] text-muted-foreground">{props.hint}</p>
      </div>
      <div className="flex shrink-0 items-center gap-2">{props.children}</div>
    </section>
  );
}

export function SettingsView() {
  const zoom = useZoom();
  const showExamples = useApp((state) => state.showExamples);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="border-b px-6 pt-3.5 pb-3">
        <h1 className="text-lg">Settings</h1>
      </header>
      <div className="w-full max-w-2xl overflow-auto px-6">
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
      </div>
    </div>
  );
}
