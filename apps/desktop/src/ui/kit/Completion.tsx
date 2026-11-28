// Completion over a text field: a list that opens while a word starting with a
// trigger (`/`, `@`) is typed, filled by the sources modules register
// (`UiRegistry.composerCompletion`). The keys stay in the field: ↑/↓ move, Enter or
// Tab take an entry, Escape closes the list for the word.

import { type KeyboardEvent, type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { type CompletionToken, applyCompletion, completionToken } from "../../state/completion";
import type { CompletionContext, CompletionItem, ComposerCompletion } from "../registry";
import { Command, CommandGroup, CommandItem, CommandList } from "@/components/ui/command";
import { cn } from "@/lib/utils";

export type CompletionSection = { source: ComposerCompletion; items: readonly CompletionItem[] };

export type Completion = {
  open: boolean;
  sections: readonly CompletionSection[];
  /** The entry Enter takes, as `entryValue` names it. */
  active: string | null;
  setActive: (value: string) => void;
  choose: (item: CompletionItem, descend?: boolean) => void;
  /** Call first in the field's key handler; true: the key was the list's. */
  onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => boolean;
  /** Call when the caret may have moved: on change, select and focus. */
  onCaret: () => void;
  /** Call when the field loses focus. */
  onBlur: () => void;
};

/** The value cmdk knows an entry by. */
export function entryValue(source: ComposerCompletion, item: CompletionItem): string {
  return `${source.id}\u0000${item.key}`;
}

/** Wires completion to a controlled textarea. */
export function useCompletion(options: {
  text: string;
  setText: (text: string) => void;
  input: RefObject<HTMLTextAreaElement | null>;
  sources: readonly ComposerCompletion[];
  context: CompletionContext;
}): Completion {
  const { text, setText, input, context } = options;
  const sources = useMemo(
    () => options.sources.filter((source) => source.when?.(context) ?? true),
    // By thread and agent, as the lists are asked for below.
    [options.sources, context.thread.id, context.provider?.id],
  );
  const triggers = useMemo(() => [...new Set(sources.map((source) => source.trigger))], [sources]);
  const [caret, setCaret] = useState<number | null>(null);
  const token = useMemo(() => (caret === null ? null : completionToken(text, caret, triggers)), [text, caret, triggers]);
  // Escape closes the list for the word it was open for, until another one is typed.
  const [dismissed, setDismissed] = useState<number | null>(null);
  const [sections, setSections] = useState<readonly CompletionSection[]>([]);
  const [active, setActive] = useState<string | null>(null);

  const trigger = token?.trigger;
  const query = token?.query;
  // A thread changes as its agent works; the list is asked again only for another
  // thread or agent, not for each of those changes.
  const latest = useRef(context);
  latest.current = context;
  const threadId = context.thread.id;
  const providerId = context.provider?.id;
  const asked = useRef(0);
  useEffect(() => {
    const context = latest.current;
    const question = ++asked.current;
    if (trigger === undefined || query === undefined) {
      setSections([]);
      return;
    }
    const mine = sources.filter((source) => source.trigger === trigger);
    void Promise.all(
      mine.map(async (source) => {
        try {
          return { source, items: await source.complete(query, context) };
        } catch {
          return { source, items: [] };
        }
      }),
    ).then((answered) => {
      if (question !== asked.current) return;
      const found = answered.filter((section) => section.items.length > 0);
      setSections(found);
      const first = found[0];
      const item = first?.items[0];
      setActive(first && item ? entryValue(first.source, item) : null);
    });
  }, [trigger, query, sources, threadId, providerId]);

  useEffect(() => {
    if (token === null || token.start !== dismissed) setDismissed(null);
  }, [token, dismissed]);

  const open = token !== null && dismissed === null && sections.length > 0;
  const entries = useMemo(
    () => sections.flatMap((section) => section.items.map((item) => ({ source: section.source, item }))),
    [sections],
  );

  // Where the caret goes once the text that was just put in is rendered.
  const pendingCaret = useRef<number | null>(null);
  useLayoutEffect(() => {
    const element = input.current;
    const at = pendingCaret.current;
    if (at === null || !element) return;
    pendingCaret.current = null;
    element.focus();
    element.setSelectionRange(at, at);
    setCaret(at);
  }, [text, input]);

  const choose = useCallback(
    (item: CompletionItem, descend = false) => {
      if (!token) return;
      const insert = descend && item.descend !== undefined ? item.descend : item.insert;
      const next = applyCompletion(text, token as CompletionToken, insert);
      pendingCaret.current = next.caret;
      setText(next.text);
    },
    [text, token, setText],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!open || event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return false;
    const index = entries.findIndex(({ source, item }) => entryValue(source, item) === active);
    const move = (by: number) => {
      const next = entries[(Math.max(index, 0) + by + entries.length) % entries.length];
      if (next) setActive(entryValue(next.source, next.item));
    };
    switch (event.key) {
      case "ArrowDown":
        move(index < 0 ? 0 : 1);
        break;
      case "ArrowUp":
        move(-1);
        break;
      case "Enter":
      case "Tab": {
        if (event.shiftKey) return false;
        const chosen = entries[Math.max(index, 0)];
        if (!chosen) return false;
        choose(chosen.item, event.key === "Tab");
        break;
      }
      case "Escape":
        setDismissed(token?.start ?? null);
        break;
      default:
        return false;
    }
    event.preventDefault();
    event.stopPropagation();
    return true;
  };

  const onCaret = () => {
    const element = input.current;
    if (!element) return;
    setCaret(element.selectionStart === element.selectionEnd ? element.selectionStart : null);
  };

  return { open, sections, active, setActive, choose, onKeyDown, onCaret, onBlur: () => setCaret(null) };
}

/** The open list, meant to sit over the field it completes. */
export function CompletionList({ completion, id, className }: { completion: Completion; id: string; className?: string }) {
  const list = useRef<HTMLDivElement>(null);
  // Only the list scrolls to the chosen entry: `scrollIntoView` would move the
  // timeline behind it as well.
  useEffect(() => {
    const box = list.current;
    // By value: cmdk marks the entry selected only after this runs.
    const entry = [...(box?.querySelectorAll<HTMLElement>("[cmdk-item]") ?? [])].find(
      (element) => element.dataset.value === completion.active,
    );
    if (!box || !entry) return;
    // The first entry of a group brings its heading into view with it.
    const group = entry.closest<HTMLElement>("[cmdk-group]");
    const first = group?.querySelector("[cmdk-item]") === entry;
    const top = (first && group ? group : entry).getBoundingClientRect().top - box.getBoundingClientRect().top + box.scrollTop;
    if (top < box.scrollTop) box.scrollTop = top;
    else if (top + entry.offsetHeight > box.scrollTop + box.clientHeight) {
      box.scrollTop = top + entry.offsetHeight - box.clientHeight;
    }
  }, [completion.active, completion.open]);
  if (!completion.open) return null;
  return (
    <Command
      shouldFilter={false}
      loop
      value={completion.active ?? ""}
      onValueChange={completion.setActive}
      // The field keeps the focus: a click on an entry must not take it away.
      onMouseDown={(event) => event.preventDefault()}
      className={cn("h-auto rounded-lg! border shadow-md", className)}
    >
      <CommandList ref={list} id={id} role="listbox" className="max-h-[min(18rem,40vh)]">
        {completion.sections.map(({ source, items }) => (
          <CommandGroup key={source.id} heading={source.title}>
            {items.map((item) => {
              const Icon = item.icon;
              return (
                <CommandItem
                  key={item.key}
                  value={entryValue(source, item)}
                  onSelect={() => completion.choose(item)}
                  className="gap-2"
                >
                  {Icon && <Icon className="size-4 text-muted-foreground" />}
                  <span className="shrink-0 font-mono text-[13px]">
                    <Highlighted text={item.label} positions={item.positions} />
                  </span>
                  {item.detail && (
                    <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground" title={item.detail}>
                      {item.detail}
                    </span>
                  )}
                  {!item.detail && <span className="flex-1" />}
                  {item.badge && (
                    <span className="shrink-0 rounded border px-1 text-[11px] text-muted-foreground">{item.badge}</span>
                  )}
                </CommandItem>
              );
            })}
          </CommandGroup>
        ))}
      </CommandList>
    </Command>
  );
}

/** `text` with the characters at `positions` stressed. */
function Highlighted({ text, positions }: { text: string; positions?: readonly number[] }) {
  if (!positions || positions.length === 0) return <>{text}</>;
  const marked = new Set(positions);
  const parts: { text: string; marked: boolean }[] = [];
  // Positions count characters, as the daemon does, not UTF-16 units.
  [...text].forEach((char, i) => {
    const on = marked.has(i);
    const last = parts[parts.length - 1];
    if (last && last.marked === on) last.text += char;
    else parts.push({ text: char, marked: on });
  });
  return (
    <>
      {parts.map((part, index) =>
        part.marked ? (
          <mark key={index} className="bg-transparent font-semibold text-primary">
            {part.text}
          </mark>
        ) : (
          <span key={index} className="text-foreground/80">
            {part.text}
          </span>
        ),
      )}
    </>
  );
}
