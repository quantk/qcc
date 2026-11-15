import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

export type ChoiceOption = { value: string | null; label: string };

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
      items={props.options}
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
              {option.label}
            </SelectItem>
          ))}
        </SelectGroup>
      </SelectContent>
    </Select>
  );
}
