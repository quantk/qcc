import { parseDiff } from "../../state/diff";
import { relativePath } from "../../util/format";
import { DiffView, Disclosure, Markdown, StatusMark } from "../../ui/kit";
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
    case "command": {
      const failed = item.exit_code !== undefined && item.exit_code !== null && item.exit_code !== 0;
      return (
        <Disclosure
          className="tool"
          head={
            <>
              <span className="label">Run</span>
              <code className="one-line">{item.command}</code>
              {failed && <span className="tag failed">exit {item.exit_code}</span>}
              <StatusMark status={item.status} />
            </>
          }
        >
          {item.output ? <pre className="output">{item.output}</pre> : null}
        </Disclosure>
      );
    }
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
        <Disclosure
          className="tool"
          head={
            <>
              <span className="label">Read</span>
              <code className="one-line" title={item.path}>
                {relativePath(item.path, thread.cwd)}
              </code>
              <StatusMark status={item.status} />
            </>
          }
        />
      );
    case "search":
      return (
        <Disclosure
          className="tool"
          head={
            <>
              <span className="label">Search</span>
              <code className="one-line">{item.query}</code>
              <StatusMark status={item.status} />
            </>
          }
        />
      );
    case "tool":
      return (
        <Disclosure
          className="tool"
          head={
            <>
              <span className="label">{item.name}</span>
              <code className="one-line">{summarize(item.input)}</code>
              <StatusMark status={item.status} />
            </>
          }
        >
          <pre className="output">{JSON.stringify(item.input, null, 2)}</pre>
          {item.output ? <pre className="output">{item.output}</pre> : null}
        </Disclosure>
      );
    case "extension":
      return null;
  }
}

