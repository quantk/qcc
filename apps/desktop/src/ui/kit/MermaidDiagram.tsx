import { CodeIcon, NetworkIcon } from "lucide-react";
import { useEffect, useId, useState } from "react";

import { errorMessage } from "../../util/format";
import { Button } from "@/components/ui/button";

type Rendered = { svg: string } | { error: string } | null;

/** Text keeps arriving while an agent streams; wait for it to settle before drawing. */
const SETTLE_MS = 400;

let loading: Promise<typeof import("mermaid").default> | null = null;

/** Mermaid is large; it is loaded the first time a diagram is on screen. */
function mermaid() {
  loading ??= import("mermaid").then(({ default: mermaid }) => {
    const dark = globalThis.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
    mermaid.initialize({
      startOnLoad: false,
      // Agent output is untrusted: no HTML in labels, no scripts, no click handlers.
      securityLevel: "strict",
      htmlLabels: false,
      flowchart: { htmlLabels: false },
      theme: dark ? "dark" : "default",
      fontFamily: "inherit",
    });
    return mermaid;
  });
  return loading;
}

/** A ```mermaid block of an agent's message, drawn as the diagram it describes. */
export function MermaidDiagram({ source }: { source: string }) {
  const id = `mermaid-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const [rendered, setRendered] = useState<Rendered>(null);
  const [showSource, setShowSource] = useState(false);

  useEffect(() => {
    let stale = false;
    const timer = setTimeout(() => {
      mermaid()
        .then((mermaid) => mermaid.render(id, source))
        .then(({ svg }) => !stale && setRendered({ svg }))
        .catch((error: unknown) => {
          if (!stale) setRendered({ error: errorMessage(error) });
          // A failed render leaves its scratch element behind.
          document.getElementById(`d${id}`)?.remove();
        });
    }, SETTLE_MS);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [id, source]);

  const drawn = rendered && "svg" in rendered ? rendered.svg : null;
  return (
    <figure className="my-2 overflow-hidden rounded-lg border">
      <div className="flex items-center justify-between border-b px-3 py-1 text-xs text-muted-foreground">
        <span>Mermaid diagram</span>
        {drawn && (
          <Button variant="ghost" size="xs" onClick={() => setShowSource(!showSource)}>
            {showSource ? <NetworkIcon /> : <CodeIcon />} {showSource ? "Diagram" : "Source"}
          </Button>
        )}
      </div>
      {drawn && !showSource ? (
        // Rendered by mermaid in strict mode, which sanitises the SVG it returns.
        <div className="overflow-x-auto bg-card p-3 [&_svg]:mx-auto [&_svg]:h-auto [&_svg]:max-w-full" dangerouslySetInnerHTML={{ __html: drawn }} />
      ) : (
        <>
          {rendered && "error" in rendered && (
            <p className="px-3 pt-2 text-xs text-destructive">Cannot draw the diagram: {rendered.error}</p>
          )}
          <pre className="m-0 overflow-x-auto bg-transparent p-3 text-xs">
            <code>{source}</code>
          </pre>
        </>
      )}
    </figure>
  );
}
