// The patterns of shadcn AI Elements (Terminal, Tool, Reasoning, …), fed by timeline
// items instead of AI SDK message parts. Each is one line of a trace that unfolds into
// what the agent saw or thought.
import { BracesIcon, BrainIcon, TerminalIcon } from "lucide-react";
import { type MouseEvent, type ReactNode, useMemo, useRef } from "react";

import type { ItemStatus } from "../../api/protocol.gen";
import { previewText } from "../../state/preview";
import { useKeepInPlace } from "./KeepInPlace";
import { Markdown } from "./Markdown";
import { useRemembered } from "./Remembered";
import { Button } from "@/components/ui/button";
import { CodeBlock, CodeBlockContent, CodeBlockFooter, CodeBlockLine } from "@/components/ui/code-block";
import {
  Trace,
  TraceCollapsible,
  TraceCollapsibleContent,
  TraceCollapsibleTrigger,
  TraceDescription,
  TraceIcon,
  TraceLine,
  TraceTitle,
} from "@/components/ui/trace";
import { useHighlightedCode } from "@/hooks/use-highlighted-code";

type LineStatus = "running" | "error" | undefined;

function lineStatus(status: ItemStatus, failed = false): LineStatus {
  if (status === "pending") return "running";
  if (status === "failed" || status === "declined" || failed) return "error";
  return undefined;
}

/** Says how an item ended when the title alone does not. */
function outcome(status: ItemStatus): string | null {
  return status === "declined" || status === "interrupted" ? status : null;
}

function hasContent(children: ReactNode): boolean {
  return children !== undefined && children !== null && children !== false;
}

/**
 * One item of the trace: an icon, a title and how it went, unfolding into `children`.
 * Until the user opens or closes it, it is open exactly while `open` holds, so a running
 * command shows its output and finished thinking folds away.
 */
export function ItemTrace(props: {
  icon: ReactNode;
  title: ReactNode;
  /** Shown on hover, typically the full path, command or query. */
  hint?: string;
  status: ItemStatus;
  /** Marks the item failed even when its status does not, as a non-zero exit does. */
  failed?: boolean;
  /** Small text after the title: a diff stat, a duration. */
  description?: ReactNode;
  /** Whether it is open while the user has not chosen. */
  open?: boolean;
  /** Classes of the title, for a command's monospace. */
  titleClassName?: string;
  children?: ReactNode;
}) {
  // Kept while the item is scrolled out of the timeline.
  const [chosen, setChosen] = useRemembered<boolean | null>("trace-open", null);
  // The line opens downwards, so it stays put on its own, unless the list follows
  // its end; keeping it in place also stops that.
  const keep = useKeepInPlace();
  const trigger = useRef<HTMLButtonElement>(null);
  const end = outcome(props.status);
  const line = (
    <TraceLine status={lineStatus(props.status, props.failed)}>
      <TraceIcon>{props.icon}</TraceIcon>
      <TraceTitle className={props.titleClassName} title={props.hint}>
        {props.title}
      </TraceTitle>
      {props.description && <TraceDescription>{props.description}</TraceDescription>}
      {end && <TraceDescription>{end}</TraceDescription>}
    </TraceLine>
  );
  if (!hasContent(props.children)) return <Trace>{line}</Trace>;
  return (
    <Trace>
      <TraceCollapsible
        open={chosen ?? props.open ?? false}
        onOpenChange={(open) => {
          keep(trigger.current);
          setChosen(open);
        }}
      >
        <TraceCollapsibleTrigger ref={trigger}>{line}</TraceCollapsibleTrigger>
        <TraceCollapsibleContent>{props.children}</TraceCollapsibleContent>
      </TraceCollapsible>
    </Trace>
  );
}

/**
 * How much of an output is shown before the reader asks for all of it: about as tall
 * as the box it replaces. The output has no scrolling of its own: a box that
 * scrolls inside the timeline took the wheel from it, so the timeline stopped under
 * the pointer.
 */
const PREVIEW = { lines: 12, chars: 1600 };

function Code(props: { text: string; language: string; failed?: boolean; name: string }) {
  const [all, setAll] = useRemembered(`code-all:${props.name}`, false);
  const preview = useMemo(() => previewText(props.text, PREVIEW), [props.text]);
  const lines = useHighlightedCode(all || !preview.clipped ? props.text : preview.text, props.language);
  const keep = useKeepInPlace();
  const block = useRef<HTMLDivElement>(null);
  const toggle = (event: MouseEvent<HTMLButtonElement>) => {
    // More grows below what is being read, so the block stays; less takes away what
    // is above the button, so the button stays.
    keep(all ? event.currentTarget : block.current);
    setAll(!all);
  };
  return (
    <CodeBlock ref={block} size="sm" variant={props.failed ? "destructive" : "default"}>
      <CodeBlockContent wrap>
        {lines.map((line, index) => (
          <CodeBlockLine key={index}>{line}</CodeBlockLine>
        ))}
      </CodeBlockContent>
      {preview.clipped && (
        <CodeBlockFooter className="-mt-1.5">
          <Button variant="ghost" size="xs" className="-ml-2 text-muted-foreground" onClick={toggle}>
            {all ? "Show less" : preview.lines > 1 ? `Show all ${preview.lines} lines` : "Show all"}
          </Button>
        </CodeBlockFooter>
      )}
    </CodeBlock>
  );
}

/** A shell command and its output: the Terminal pattern. */
export function CommandTrace(props: { command: string; output?: string | null; exitCode?: number | null; status: ItemStatus }) {
  const failed = props.exitCode !== undefined && props.exitCode !== null && props.exitCode !== 0;
  const verb = props.status === "pending" ? "Running" : "Ran";
  return (
    <ItemTrace
      icon={<TerminalIcon />}
      title={failed ? `${props.command} exited with code ${props.exitCode}` : `${verb} ${props.command}`}
      titleClassName="font-mono text-xs"
      hint={props.command}
      status={props.status}
      failed={failed}
      open={props.status === "pending" && !!props.output}
    >
      {props.output ? <Code name="output" text={props.output} language="ansi" failed={failed} /> : null}
    </ItemTrace>
  );
}

/** Any other tool call, from its input to its output: the Tool pattern. */
export function ToolTrace(props: {
  icon: ReactNode;
  title: ReactNode;
  hint?: string;
  status: ItemStatus;
  input?: unknown;
  output?: string | null;
}) {
  // An empty object says nothing a reader needs to unfold.
  const hasInput =
    props.input !== undefined &&
    props.input !== null &&
    !(typeof props.input === "object" && Object.keys(props.input).length === 0);
  return (
    <ItemTrace icon={props.icon} title={props.title} hint={props.hint} status={props.status}>
      {hasInput || props.output ? (
        <div className="flex flex-col gap-2">
          {hasInput && <Code name="input" text={JSON.stringify(props.input, null, 2)} language="json" />}
          {props.output ? <Code name="output" text={props.output} language="ansi" failed={props.status === "failed"} /> : null}
        </div>
      ) : null}
    </ItemTrace>
  );
}

/** The agent's thinking: open while it streams, folded once it is done. The Reasoning pattern. */
export function ReasoningTrace(props: { text: string; status: ItemStatus }) {
  const thinking = props.status === "pending";
  return (
    <ItemTrace icon={<BrainIcon />} title={thinking ? "Thinking" : "Thought"} status={props.status} open={thinking}>
      {props.text ? <Markdown text={props.text} /> : null}
    </ItemTrace>
  );
}

/** Something with no drawing of its own, shown as the data it is. */
export function DataTrace(props: { title: string; status: ItemStatus; value: unknown; description?: ReactNode }) {
  return (
    <ItemTrace icon={<BracesIcon />} title={props.title} status={props.status} description={props.description}>
      <Code name="value" text={JSON.stringify(props.value, null, 2)} language="json" />
    </ItemTrace>
  );
}
