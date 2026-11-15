// The Terminal and Tool patterns of shadcn AI Elements, fed by timeline items instead of
// AI SDK message parts. Each is one line of a trace that unfolds into what the agent saw.
import { TerminalIcon } from "lucide-react";
import { type ReactNode, useState } from "react";

import type { ItemStatus } from "../../api/protocol.gen";
import { CodeBlock, CodeBlockContent, CodeBlockLine } from "@/components/ui/code-block";
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

/**
 * A line that unfolds into `children`. Until the user picks a side it stays open while
 * `autoOpen` holds, so a running command shows its output as it arrives.
 */
function TraceRow(props: { line: ReactNode; autoOpen?: boolean; children?: ReactNode }) {
  const [chosen, setChosen] = useState<boolean | null>(null);
  if (props.children === undefined || props.children === null || props.children === false) {
    return <Trace>{props.line}</Trace>;
  }
  return (
    <Trace>
      <TraceCollapsible open={chosen ?? props.autoOpen ?? false} onOpenChange={(open) => setChosen(open)}>
        <TraceCollapsibleTrigger>{props.line}</TraceCollapsibleTrigger>
        <TraceCollapsibleContent>{props.children}</TraceCollapsibleContent>
      </TraceCollapsible>
    </Trace>
  );
}

function Code({ text, language, failed }: { text: string; language: string; failed?: boolean }) {
  const lines = useHighlightedCode(text, language);
  return (
    <CodeBlock size="sm" variant={failed ? "destructive" : "default"}>
      <CodeBlockContent wrap className="max-h-64">
        {lines.map((line, index) => (
          <CodeBlockLine key={index}>{line}</CodeBlockLine>
        ))}
      </CodeBlockContent>
    </CodeBlock>
  );
}

/** A shell command and its output: the Terminal pattern. */
export function CommandTrace(props: { command: string; output?: string | null; exitCode?: number | null; status: ItemStatus }) {
  const failed = props.exitCode !== undefined && props.exitCode !== null && props.exitCode !== 0;
  const status = lineStatus(props.status, failed);
  const verb = props.status === "pending" ? "Running" : "Ran";
  const title = failed ? `${props.command} exited with code ${props.exitCode}` : `${verb} ${props.command}`;
  const end = outcome(props.status);
  return (
    <TraceRow
      autoOpen={props.status === "pending" && !!props.output}
      line={
        <TraceLine status={status}>
          <TraceIcon>
            <TerminalIcon />
          </TraceIcon>
          <TraceTitle className="font-mono text-xs" title={props.command}>
            {title}
          </TraceTitle>
          {end && <TraceDescription>{end}</TraceDescription>}
        </TraceLine>
      }
    >
      {props.output ? <Code text={props.output} language="ansi" failed={failed} /> : null}
    </TraceRow>
  );
}

/** Any other tool call, from its input to its output: the Tool pattern. */
export function ToolTrace(props: {
  icon: ReactNode;
  title: ReactNode;
  /** Shown on hover, typically the full path or query. */
  hint?: string;
  status: ItemStatus;
  input?: unknown;
  output?: string | null;
}) {
  const end = outcome(props.status);
  const failed = props.status === "failed";
  // An empty object says nothing a reader needs to unfold.
  const hasInput =
    props.input !== undefined &&
    props.input !== null &&
    !(typeof props.input === "object" && Object.keys(props.input).length === 0);
  const body =
    hasInput || props.output ? (
      <div className="flex flex-col gap-2">
        {hasInput && <Code text={JSON.stringify(props.input, null, 2)} language="json" />}
        {props.output ? <Code text={props.output} language="ansi" failed={failed} /> : null}
      </div>
    ) : null;
  return (
    <TraceRow
      line={
        <TraceLine status={lineStatus(props.status)}>
          <TraceIcon>{props.icon}</TraceIcon>
          <TraceTitle title={props.hint}>{props.title}</TraceTitle>
          {end && <TraceDescription>{end}</TraceDescription>}
        </TraceLine>
      }
    >
      {body}
    </TraceRow>
  );
}
