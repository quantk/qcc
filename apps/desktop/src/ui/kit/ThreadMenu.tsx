import { Fragment, type ReactElement, type ReactNode } from "react";

import type { Thread } from "../../api/protocol.gen";
import { useUi } from "../registry";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";

/**
 * A thread's right-click menu around `render`, the element that shows the thread.
 * Its entries are contributed (`ui.threadMenuItem`); the shell only draws them.
 */
export function ThreadMenu(props: { thread: Thread; render: ReactElement; children: ReactNode }) {
  const { thread } = props;
  const all = useUi((snapshot) => snapshot.menu);
  const items = all.filter((item) => item.when?.(thread) ?? true);
  return (
    <ContextMenu>
      <ContextMenuTrigger render={props.render}>{props.children}</ContextMenuTrigger>
      {items.length > 0 && (
        <ContextMenuContent className="min-w-52">
          {items.map((item, index) => (
            <Fragment key={item.id}>
              {index > 0 && items[index - 1]?.group !== item.group && <ContextMenuSeparator />}
              <ContextMenuItem variant={item.destructive ? "destructive" : "default"} onClick={() => item.run(thread)}>
                {typeof item.title === "function" ? item.title(thread) : item.title}
              </ContextMenuItem>
            </Fragment>
          ))}
        </ContextMenuContent>
      )}
    </ContextMenu>
  );
}
