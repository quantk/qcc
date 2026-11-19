import type { ReactNode } from "react";

import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

/** `icon` is drawn before the label, in the list and in the closed select. */
export type ChoiceOption = { value: string | null; label: string; icon?: ReactNode };

/**
 * One of a few values, on shadcn's Select. `null` is a value like any other: the
 * "agent default" every list of options here starts with.
 */
export function Choice(props: {
  value: string | null;
  options: readonly ChoiceOption[];
  onChange: (value: string | null) => void;
  disabled?: boolean;
  title?: string;
  size?: "sm" | "default";
  className?: string;
  id?: string;
  "aria-label"?: string;
}) {
  return (
    <Select
      items={props.options.map((option) => ({
        value: option.value,
        label: option.icon ? <WithIcon option={option} /> : option.label,
      }))}
      value={props.value}
      onValueChange={(value) => props.onChange((value as string | null) ?? null)}
      disabled={props.disabled}
    >
      <SelectTrigger
        id={props.id}
        size={props.size}
        className={props.className}
        title={props.title}
        aria-label={props["aria-label"]}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectGroup>
          {props.options.map((option) => (
            <SelectItem key={option.value ?? ""} value={option.value}>
              {option.icon ? <WithIcon option={option} /> : option.label}
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  );
}

function WithIcon({ option }: { option: ChoiceOption }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-2">
      {option.icon}
      <span className="truncate">{option.label}</span>
    </span>
  );
}
