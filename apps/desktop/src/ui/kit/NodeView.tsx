// Draws the UI nodes plugins describe their views with. The set is closed: a node
// of an unknown type, or with fields of the wrong shape, is not drawn at all, so
// nothing a plugin sends can put markup or code of its own on screen.

import { type ReactNode, useState } from "react";

import { Markdown } from "./Markdown";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export type NodeAction = (action: string, value?: string) => void;

type Fields = Record<string, unknown>;

const TONES = new Set(["muted", "success", "warning", "danger"]);
const MAX_DEPTH = 12;

const str = (value: unknown): string | undefined => (typeof value === "string" ? value : undefined);
const tone = (value: unknown): string => (typeof value === "string" && TONES.has(value) ? ` tone-${value}` : "");

function InputNode({ node, act, busy }: { node: Fields; act: NodeAction; busy: boolean }) {
  const [text, setText] = useState("");
  const action = str(node.action);
  if (!action) return null;
  return (
    <form
      className="pn-input"
      onSubmit={(event) => {
        event.preventDefault();
        act(action, text);
        setText("");
      }}
    >
      <Input value={text} placeholder={str(node.placeholder)} onChange={(event) => setText(event.target.value)} />
      <Button type="submit" disabled={busy}>
        {str(node.submit) ?? "Submit"}
      </Button>
    </form>
  );
}

function draw(value: unknown, act: NodeAction, busy: boolean, depth: number, key?: number): ReactNode {
  if (typeof value !== "object" || value === null || Array.isArray(value) || depth > MAX_DEPTH) return null;
  const node = value as Fields;
  const children = () =>
    Array.isArray(node.children) ? node.children.map((child, index) => draw(child, act, busy, depth + 1, index)) : null;

  switch (node.type) {
    case "stack":
      return (
        <div key={key} className="pn-stack">
          {children()}
        </div>
      );
    case "row":
      return (
        <div key={key} className="pn-row">
          {children()}
        </div>
      );
    case "heading":
      return (
        <h3 key={key} className="pn-heading">
          {str(node.text)}
        </h3>
      );
    case "text":
      return (
        <p key={key} className={`pn-text${tone(node.tone)}`}>
          {str(node.text)}
        </p>
      );
    case "markdown":
      return <Markdown key={key} text={str(node.text) ?? ""} />;
    case "code":
      return (
        <pre key={key} className="output pn-code">
          {str(node.text)}
        </pre>
      );
    case "badge":
      return (
        <span key={key} className={`tag${tone(node.tone)}`}>
          {str(node.text)}
        </span>
      );
    case "divider":
      return <hr key={key} className="pn-divider" />;
    case "keyValues":
      return (
        <dl key={key} className="pn-kv">
          {(Array.isArray(node.items) ? node.items : []).map((item: Fields | null, index) => (
            <div key={index}>
              <dt>{str(item?.key)}</dt>
              <dd>{str(item?.value)}</dd>
            </div>
          ))}
        </dl>
      );
    case "table": {
      const columns = Array.isArray(node.columns) ? node.columns : [];
      const rows = Array.isArray(node.rows) ? node.rows : [];
      return (
        <table key={key} className="pn-table">
          <thead>
            <tr>
              {columns.map((column, index) => (
                <th key={index}>{str(column)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, index) => (
              <tr key={index}>
                {(Array.isArray(row) ? row : []).map((cell, cellIndex) => (
                  <td key={cellIndex}>{str(cell)}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      );
    }
    case "button": {
      const action = str(node.action);
      if (!action) return null;
      const variant = node.style === "primary" ? "default" : node.style === "danger" ? "destructive" : "outline";
      return (
        <Button key={key} variant={variant} disabled={busy} onClick={() => act(action, str(node.value))}>
          {str(node.label) ?? action}
        </Button>
      );
    }
    case "input":
      return <InputNode key={key} node={node} act={act} busy={busy} />;
    default:
      return null;
  }
}

const noAction: NodeAction = () => {};

/** A tree of UI nodes. `onAction` receives what buttons and inputs send. */
export function NodeView(props: { tree: unknown; onAction?: NodeAction; busy?: boolean }) {
  return <div className="pn">{draw(props.tree, props.onAction ?? noAction, props.busy ?? false, 0)}</div>;
}
