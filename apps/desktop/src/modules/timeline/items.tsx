import { FileTextIcon, SearchIcon, WrenchIcon } from "lucide-react";

import { parseDiff } from "../../state/diff";
import { relativePath } from "../../util/format";
import { CommandTrace, DiffView, Disclosure, Markdown, StatusMark, ToolTrace } from "../../ui/kit";
import type { ItemRendererProps } from "../../ui/registry";

export const CORE_ITEM_TYPES = [
  "agentMessage",
  "reasoning",
  "plan",
  "command",
  "fileChange",
  "fileRead",
  "search",
  "tool",
] as const;

function summarize(value: unknown): string {
  if (value === null || value === undefined) return "";
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 120 ? `${text.slice(0, 120)}…` : text;
}

/** Draws every built-in item type. Registered per type, so any one can be replaced. */
export function CoreItem({ item, thread }: ItemRendererProps) {
  switch (item.type) {
    case "agentMessage":
      return (
        <div className="message agent">
          {item.text ? <Markdown text={item.text} /> : <StatusMark status={item.status} />}
          {item.text && item.status !== "pending" && <StatusMark status={item.status} />}
        </div>
      );
    case "reasoning":
      return (
        <Disclosure
          className="reasoning"
          head={
            <>
              <span className="label">Thinking</span>
              <StatusMark status={item.status} />
            </>
          }
        >
          {item.text ? <Markdown text={item.text} /> : null}
        </Disclosure>
      );
    case "plan":
      return (
        <div className="plan">
          <div className="label">Plan</div>
          <Markdown text={item.text} />
        </div>
      );
    case "command":
      return <CommandTrace command={item.command} output={item.output} exitCode={item.exit_code} status={item.status} />;
    case "fileChange": {
      const files = item.diff ? parseDiff(item.diff, item.path) : [];
      const additions = files.reduce((sum, file) => sum + file.additions, 0);
      const deletions = files.reduce((sum, file) => sum + file.deletions, 0);
      const lines = files.reduce((sum, file) => sum + file.lines.length, 0);
      return (
        <Disclosure
          className="tool"
          defaultOpen={lines > 0 && lines <= 30}
          head={
            <>
              <span className="label">Edit</span>
              <code className="one-line" title={item.path}>
                {relativePath(item.path, thread.cwd)}
              </code>
              {files.length > 0 && (
                <span className="diff-stat">
                  <span className="add">+{additions}</span> <span className="del">−{deletions}</span>
                </span>
              )}
              <StatusMark status={item.status} />
            </>
          }
        >
          {files.length > 0 ? <DiffView files={files} bare /> : null}
        </Disclosure>
      );
    }
    case "fileRead":
      return (
        <ToolTrace
          icon={<FileTextIcon />}
          title={`Read ${relativePath(item.path, thread.cwd)}`}
          hint={item.path}
          status={item.status}
        />
      );
    case "search":
      return <ToolTrace icon={<SearchIcon />} title={`Search ${item.query}`} hint={item.query} status={item.status} />;
    case "tool":
      return (
        <ToolTrace
          icon={<WrenchIcon />}
          title={
            <>
              {item.name} <span className="font-mono text-xs opacity-70">{summarize(item.input)}</span>
            </>
          }
          hint={item.name}
          status={item.status}
          input={item.input}
          output={item.output}
        />
      );
    case "extension":
      return null;
  }
}

