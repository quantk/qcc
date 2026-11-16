import { ChevronRightIcon, MessageSquarePlusIcon } from "lucide-react";
import { Fragment, type ReactNode, memo, useEffect, useMemo, useState } from "react";

import { type DiffFile, highlightRuns } from "../../state/diff";
import { languageOfPath } from "../../state/languages";
import { loadHighlighter } from "@/lib/highlighter";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

/** Files longer than this start folded: a huge diff must not freeze the window. */
const FOLD_ABOVE = 400;

/** Past this many lines a file is shown plain: highlighting it would stall the window. */
const HIGHLIGHT_UP_TO = 3000;

/**
 * What a view adds to a diff of its own: comments under lines and a way to start
 * one. The diff itself knows nothing about comments.
 */
export type DiffAddon = {
  /** Drawn above the file's lines. */
  head?(file: DiffFile): ReactNode;
  /** Drawn under the line at `index` of `file.lines`. */
  below?(file: DiffFile, index: number): ReactNode;
  /** Offered on the lines `canPick` allows: the user picked the line at `index`. */
  pick?(file: DiffFile, index: number): void;
  canPick?(file: DiffFile, index: number): boolean;
};

const statusLabel = { added: "new", deleted: "deleted", modified: "" } as const;

/**
 * Bring the diff of `path` into view inside `within`. The same diff may be drawn
 * twice at once (a thread tab and the side panel), so files are found by a data
 * attribute in their own container rather than by a document-wide id.
 */
export function scrollToFile(within: HTMLElement | null, path: string): void {
  const sections = within?.querySelectorAll<HTMLElement>("[data-diff-path]") ?? [];
  const target = Array.from(sections).find((section) => section.dataset.diffPath === path);
  target?.scrollIntoView({ block: "start", behavior: "smooth" });
}

/** Lines added and removed, as a diff counts them. */
export function DiffStat({ additions, deletions }: { additions: number; deletions: number }) {
  return (
    <span className="diff-stat">
      <span className="add">+{additions}</span> <span className="del">−{deletions}</span>
    </span>
  );
}

type Highlighter = NonNullable<Awaited<ReturnType<typeof loadHighlighter>>>;

/**
 * The lines of `file` highlighted in the language its name suggests, by index in
 * `file.lines`; null until the highlighter is loaded, and for a file in a language
 * it does not know. Only an open file is highlighted, so folded ones cost nothing.
 */
function useHighlightedLines(file: DiffFile, open: boolean): (ReactNode[] | undefined)[] | null {
  const language = file.binary || file.lines.length > HIGHLIGHT_UP_TO ? null : languageOfPath(file.path);
  const [loaded, setLoaded] = useState<{ language: string; highlighter: Highlighter } | null>(null);
  const wanted = open ? language : null;

  useEffect(() => {
    if (!wanted) return;
    let stale = false;
    loadHighlighter(wanted)
      .then((highlighter) => {
        if (!stale && highlighter) setLoaded({ language: wanted, highlighter });
      })
      .catch(() => {});
    return () => {
      stale = true;
    };
  }, [wanted]);

  const highlighter = loaded && loaded.language === wanted ? loaded.highlighter : null;
  return useMemo(() => {
    if (!highlighter) return null;
    const lines: (ReactNode[] | undefined)[] = new Array(file.lines.length);
    for (const run of highlightRuns(file)) {
      const highlighted = highlighter.highlight(run.text).lines;
      run.indices.forEach((index, at) => {
        lines[index] = highlighted[at];
      });
    }
    return lines;
  }, [file, highlighter]);
}

/** A row across the whole diff that stays in view when the code scrolls sideways. */
function HeadRow({ children }: { children: ReactNode }) {
  return children ? <AddonRow>{children}</AddonRow> : null;
}

function AddonRow({ children }: { children: ReactNode }) {
  return (
    <tr className="diff-addon">
      <td colSpan={3} className="border-y bg-background p-0 font-sans whitespace-normal">
        <div className="sticky left-0 max-w-2xl p-2">{children}</div>
      </td>
    </tr>
  );
}

const DiffFileView = memo(function DiffFileView({
  file,
  bare,
  addon,
}: {
  file: DiffFile;
  bare: boolean;
  addon?: DiffAddon;
}) {
  const [open, setOpen] = useState(file.lines.length <= FOLD_ABOVE);
  const highlighted = useHighlightedLines(file, open || bare);
  return (
    <Collapsible
      render={<section />}
      className="diff-file"
      data-diff-path={bare ? undefined : file.path}
      open={open || bare}
      onOpenChange={setOpen}
    >
      {!bare && (
        <CollapsibleTrigger className="diff-file-head group cursor-pointer">
          <ChevronRightIcon className="size-3.5 flex-none text-muted-foreground transition-transform group-data-[panel-open]:rotate-90" />
          <span className="diff-path">{file.path}</span>
          {statusLabel[file.status] && <span className="tag">{statusLabel[file.status]}</span>}
          <DiffStat additions={file.additions} deletions={file.deletions} />
        </CollapsibleTrigger>
      )}
      <CollapsibleContent>
        <table className="diff-lines">
          <tbody>
            {addon?.head && <HeadRow>{addon.head(file)}</HeadRow>}
            {file.lines.map((line, index) => {
              const below = addon?.below?.(file, index);
              const pickable = addon?.pick && (addon.canPick?.(file, index) ?? true);
              return (
                <Fragment key={index}>
                  {line.kind === "hunk" || line.kind === "note" ? (
                    <tr className={line.kind}>
                      <td className="no" colSpan={2} />
                      <td className="code">{line.text}</td>
                    </tr>
                  ) : (
                    <tr className={`${line.kind} group/line`}>
                      <td className="no relative">
                        {pickable && (
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            className="absolute top-0 left-0.5 size-[1.45em] bg-muted opacity-0 group-hover/line:opacity-100 focus-visible:opacity-100"
                            title="Comment on this line"
                            aria-label="Comment on this line"
                            onClick={() => addon.pick!(file, index)}
                          >
                            <MessageSquarePlusIcon />
                          </Button>
                        )}
                        {line.oldNo}
                      </td>
                      <td className="no">{line.newNo}</td>
                      <td className="code">
                        <span className="sign">{line.kind === "add" ? "+" : line.kind === "del" ? "−" : " "}</span>
                        {highlighted?.[index] ?? line.text}
                      </td>
                    </tr>
                  )}
                  {below ? <AddonRow>{below}</AddonRow> : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </CollapsibleContent>
    </Collapsible>
  );
});

/** `bare` drops the per-file headers: for the diff of one file shown under its name. */
export function DiffView({ files, bare = false, addon }: { files: DiffFile[]; bare?: boolean; addon?: DiffAddon }) {
  return (
    <div className="diff">
      {files.map((file, index) => (
        <DiffFileView key={`${index}:${file.path}`} file={file} bare={bare} addon={addon} />
      ))}
    </div>
  );
}
