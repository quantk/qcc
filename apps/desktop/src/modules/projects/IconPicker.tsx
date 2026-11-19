import { BanIcon, RotateCcwIcon } from "lucide-react";
import { useState } from "react";

import type { ProjectIcon } from "../../api/protocol.gen";
import { emojiProblem, pickColor, pickEmoji, pickName } from "../../state/projectIcon";
import {
  DEFAULT_PROJECT_ICON,
  ICON_COLORS,
  PROJECT_ICONS,
  ProjectIconView,
  iconColorValue,
  projectIconLabel,
} from "../../ui/kit/ProjectIcon";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

/**
 * The icon of a project: a button showing it, opening a grid of icons, the palette
 * for them and a field for an emoji instead.
 */
export function IconPicker({ value, onChange }: { value: ProjectIcon | null; onChange: (icon: ProjectIcon | null) => void }) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState(value?.kind === "emoji" ? value.emoji : "");
  const problem = typed.trim() ? emojiProblem(typed) : null;
  const name = value?.kind === "lucide" ? value.name : value ? null : DEFAULT_PROJECT_ICON;
  const color = value?.kind === "lucide" ? (value.color ?? null) : null;
  const emoji = value?.kind === "emoji";

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="outline"
            size="icon"
            aria-label={`Icon: ${projectIconLabel(value)}`}
            title="Icon of the project"
          >
            <ProjectIconView icon={value} size={16} className="text-muted-foreground" />
          </Button>
        }
      />
      <PopoverContent align="start" className="w-auto gap-3 p-3">
        <div role="radiogroup" aria-label="Icon" className="grid grid-cols-9 gap-0.5">
          {Object.keys(PROJECT_ICONS).map((candidate) => {
            const chosen = candidate === name;
            return (
              <Button
                key={candidate}
                type="button"
                variant="ghost"
                size="icon-sm"
                role="radio"
                aria-checked={chosen}
                aria-label={candidate}
                title={candidate}
                className={cn(chosen && "bg-muted ring-1 ring-ring")}
                onClick={() => {
                  onChange(pickName(value, candidate));
                  setTyped("");
                }}
              >
                <ProjectIconView
                  icon={{ kind: "lucide", name: candidate, color }}
                  size={16}
                  className={cn(!color && "text-foreground/80")}
                />
              </Button>
            );
          })}
        </div>
        <div
          role="radiogroup"
          aria-label="Color"
          className="flex items-center gap-1"
          title={emoji ? "An emoji has its own colors" : undefined}
        >
          <Swatch label="Text color" chosen={!emoji && color === null} disabled={emoji} onPick={() => onChange(pickColor(value, null))}>
            <BanIcon className="size-3.5 text-muted-foreground" />
          </Swatch>
          {ICON_COLORS.map((candidate) => (
            <Swatch
              key={candidate.value}
              label={candidate.label}
              chosen={!emoji && color === candidate.value}
              disabled={emoji}
              onPick={() => onChange(pickColor(value, candidate.value))}
            >
              <span className="size-3.5 rounded-full" style={{ background: iconColorValue(candidate.value) }} />
            </Swatch>
          ))}
        </div>
        <div className="flex items-start gap-2">
          <div className="flex flex-1 flex-col gap-1">
            <Input
              aria-label="Emoji"
              value={typed}
              placeholder="Or an emoji: 🚀"
              aria-invalid={problem !== null}
              onChange={(event) => {
                setTyped(event.target.value);
                const picked = pickEmoji(event.target.value);
                if (picked) onChange(picked);
              }}
              onKeyDown={(event) => {
                // Enter picks the emoji and closes the picker, not the whole dialog.
                if (event.key !== "Enter") return;
                event.preventDefault();
                if (!problem) setOpen(false);
              }}
              spellCheck={false}
            />
            {problem && <p className="text-xs text-destructive">{problem}</p>}
          </div>
          <Button
            type="button"
            variant="ghost"
            disabled={value === null}
            title="No icon of its own: the default"
            // The button is disabled once clicked; closing hands focus back to the
            // trigger, so Escape afterwards does not fall through and close the dialog.
            onClick={() => {
              onChange(null);
              setTyped("");
              setOpen(false);
            }}
          >
            <RotateCcwIcon /> Default
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function Swatch(props: {
  label: string;
  chosen: boolean;
  disabled: boolean;
  onPick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      role="radio"
      aria-checked={props.chosen}
      aria-label={props.label}
      title={props.disabled ? undefined : props.label}
      disabled={props.disabled}
      className={cn("rounded-full", props.chosen && "ring-2 ring-ring")}
      onClick={props.onPick}
    >
      {props.children}
    </Button>
  );
}
