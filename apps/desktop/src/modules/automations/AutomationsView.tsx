import { CalendarClockIcon, ChevronRightIcon, PencilIcon, PlayIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { AutomationEntry, AutomationRun } from "../../api/protocol.gen";
import { clockTime, describeSchedule, outcomeLabel, runSummary, triggerLabel, untilTime } from "../../state/automations";
import { attentionLabel, attentionOf, providerName, threadTitle } from "../../state/selectors";
import { store, useApp } from "../../state/store";
import { relativeTime, tailPath } from "../../util/format";
import { useNow } from "../../util/useNow";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { openAutomation, takeFocus } from "./open";

/** Every automation: when it runs, how its latest runs went, and the threads they started. */
export function AutomationsView() {
  const automations = useApp((state) => state.automations);
  const connected = useApp((state) => state.daemon.status === "connected");
  const now = useNow(30_000);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b px-6 pt-3.5 pb-3">
        <h1 className="text-lg">Automations</h1>
        <span className="flex-1" />
        <Button size="sm" disabled={!connected} onClick={() => openAutomation()}>
          <PlusIcon /> New automation
        </Button>
      </header>
      <div className="flex w-full max-w-3xl flex-col gap-3 overflow-auto px-6 pt-6 pb-10">
        <p className="text-[13px] text-muted-foreground">
          Prompts qcc sends an agent on a schedule while the daemon runs. Each run is a thread like any other, marked
          with the automation that started it.
        </p>
        {automations.length === 0 ? (
          <Empty className="border border-dashed">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <CalendarClockIcon />
              </EmptyMedia>
              <EmptyTitle>No automations yet</EmptyTitle>
              <EmptyDescription>
                “Every morning, look through the new issues”: make one here or with <code>qcc automation add</code>.
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        ) : (
          automations.map((entry) => <AutomationCard key={entry.automation.id} entry={entry} now={now} />)
        )}
      </div>
    </div>
  );
}

/** How long ago, in words that read after "Last:". */
function ago(ts: number, now: number): string {
  const time = relativeTime(ts, now);
  if (time === "now") return "just now";
  return /^\d+[mhd]$/.test(time) ? `${time} ago` : `on ${time}`;
}

function AutomationCard({ entry, now }: { entry: AutomationEntry; now: number }) {
  const { automation, next_run_at: next, last_run: last } = entry;
  const agent = useApp((state) => providerName(state.allPlugins, automation.provider));
  const [open, setOpen] = useState(() => takeFocus(automation.id));
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [running, setRunning] = useState(false);
  const card = useRef<HTMLElement>(null);
  useEffect(() => {
    if (open) card.current?.scrollIntoView({ block: "nearest" });
    // Only the first draw: a card opened for the user is brought into view once.
  }, []);
  const described = describeSchedule(automation.schedule);

  const runNow = async () => {
    setRunning(true);
    const run = await store().runAutomation(automation.id);
    setRunning(false);
    if (run?.outcome === "skipped") store().notify(`${automation.name}: ${runSummary(run)}`);
    if (run?.outcome === "started" && run.thread_id) store().select(run.thread_id);
  };

  return (
    <article ref={card} className={cn("flex flex-col rounded-lg border", !automation.enabled && "opacity-80")}>
      <header className="flex items-center gap-2 px-4 pt-3">
        <h3 className="truncate text-sm font-semibold">{automation.name}</h3>
        <Badge variant="secondary" className="font-mono" title={described ?? undefined}>
          {automation.schedule}
        </Badge>
        {!automation.enabled && <Badge variant="outline">off</Badge>}
        <span className="flex-1" />
        <Button variant="ghost" size="icon-sm" aria-label={`Run ${automation.name} now`} title="Run now" disabled={running} onClick={() => void runNow()}>
          <PlayIcon />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label={`Edit ${automation.name}`} title="Edit" onClick={() => openAutomation(automation)}>
          <PencilIcon />
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={`Delete ${automation.name}`}
          title="Delete"
          onClick={() => setConfirmDelete(true)}
        >
          <Trash2Icon />
        </Button>
        <Switch
          aria-label={`Run ${automation.name} on its schedule`}
          title={automation.enabled ? "Runs on its schedule" : "Runs only when you run it"}
          checked={automation.enabled}
          onCheckedChange={(enabled) => void store().setAutomationEnabled(automation.id, enabled)}
        />
      </header>
      <p className="line-clamp-2 px-4 pt-1 text-[13px]" title={automation.prompt}>
        {automation.prompt}
      </p>
      <p className="truncate px-4 pt-1 text-xs text-muted-foreground" title={automation.cwd}>
        {agent}
        {automation.model ? ` · ${automation.model}` : ""} · {tailPath(automation.cwd, 48)}
        {automation.environment ? ` · ${automation.environment}` : ""}
        {automation.thread_mode === "continue" ? " · continues one thread" : ""}
      </p>
      <div className="flex flex-wrap gap-x-4 gap-y-1 px-4 pt-1 text-xs text-muted-foreground">
        <span title={next ? clockTime(next) : undefined}>
          Next: {next ? `${untilTime(next, now)} (${clockTime(next)})` : automation.enabled ? "never" : "off"}
        </span>
        {last && (
          <span
            className={cn("max-w-full truncate", last.outcome === "failed" && "text-destructive")}
            title={last.reason ?? undefined}
          >
            Last: {ago(last.at, now)}, {outcomeLabel[last.outcome].toLowerCase()}
            {last.outcome !== "started" && last.reason ? ` — ${last.reason}` : ""}
          </span>
        )}
      </div>
      <Collapsible open={open} onOpenChange={setOpen} className="mt-2 border-t">
        <CollapsibleTrigger className="group flex w-full items-center gap-2 px-4 py-2 text-left text-[13px] text-muted-foreground">
          <ChevronRightIcon className="size-4 transition-transform group-data-[panel-open]:rotate-90" />
          Recent runs
        </CollapsibleTrigger>
        <CollapsibleContent>{open && <RunList id={automation.id} lastAt={last?.at} />}</CollapsibleContent>
      </Collapsible>
      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {automation.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              It will not run again, and its history goes with it. The threads it started stay.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep it</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void store().deleteAutomation(automation.id)}>
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </article>
  );
}

/** The latest runs of one automation, read again whenever it runs. */
function RunList({ id, lastAt }: { id: string; lastAt: number | undefined }) {
  const [runs, setRuns] = useState<AutomationRun[] | null>(null);
  useEffect(() => {
    let current = true;
    void store()
      .automationRuns(id, 20)
      .then((read) => current && setRuns(read ?? []));
    return () => {
      current = false;
    };
  }, [id, lastAt]);

  if (runs === null) return <p className="px-4 pb-3 text-xs text-muted-foreground">Reading…</p>;
  if (runs.length === 0) return <p className="px-4 pb-3 text-xs text-muted-foreground">It has not come due yet.</p>;
  return (
    <ul className="flex flex-col divide-y border-t">
      {runs.map((run, index) => (
        <RunRow key={`${run.at}:${index}`} run={run} />
      ))}
    </ul>
  );
}

function RunRow({ run }: { run: AutomationRun }) {
  const thread = useApp((state) => (run.thread_id ? state.threads[run.thread_id] : undefined));
  const attention = thread ? attentionOf(thread) : null;
  return (
    <li className="flex items-center gap-3 px-4 py-1.5 text-xs">
      <span className="w-28 shrink-0 tabular-nums text-muted-foreground" title={run.scheduled_for ? `Scheduled for ${clockTime(run.scheduled_for)}` : undefined}>
        {clockTime(run.at)}
      </span>
      <span className="w-20 shrink-0 text-muted-foreground">{triggerLabel[run.trigger]}</span>
      <span
        className={cn("min-w-0 flex-1 truncate", run.outcome === "failed" && "text-destructive")}
        title={run.reason ?? undefined}
      >
        {outcomeLabel[run.outcome]}
        {run.reason ? ` — ${run.reason}` : ""}
      </span>
      {run.thread_id && (
        <Button
          variant="link"
          size="xs"
          className="max-w-56 shrink-0 justify-end px-0"
          title={thread ? `Open ${threadTitle(thread)}` : "The thread is no longer known"}
          disabled={!thread}
          onClick={() => run.thread_id && store().select(run.thread_id)}
        >
          {attention && <span className={`dot ${attention}`} title={attentionLabel[attention]} />}
          <span className="truncate">{thread ? threadTitle(thread) : run.thread_id}</span>
        </Button>
      )}
    </li>
  );
}
