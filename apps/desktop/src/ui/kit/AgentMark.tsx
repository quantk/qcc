// The mark of an agent where its name does not fit: the logo its provider declares,
// painted in the color of the text around it, or the initials of its name.

import type { AgentMark } from "../../state/selectors";
import { cn } from "@/lib/utils";

/**
 * A box of one width whatever it holds, so rows of different agents line up. A
 * logo is used as a mask over the text color rather than drawn as given: it fits
 * the light and the dark theme alike, and its markup never reaches the page.
 */
export function AgentMarkView({ mark, className }: { mark: AgentMark; className?: string }) {
  const box = "inline-flex h-4 w-5 flex-none items-center justify-center";
  if (mark.kind === "logo") {
    const image = `url("${mark.src}")`;
    return (
      <span role="img" aria-label={mark.name} className={cn(box, className)}>
        <span
          aria-hidden
          className="size-3.5 bg-current"
          style={{
            maskImage: image,
            WebkitMaskImage: image,
            maskSize: "contain",
            WebkitMaskSize: "contain",
            maskRepeat: "no-repeat",
            WebkitMaskRepeat: "no-repeat",
            maskPosition: "center",
            WebkitMaskPosition: "center",
          }}
        />
      </span>
    );
  }
  return (
    <span
      aria-label={mark.name}
      className={cn(
        box,
        "overflow-hidden rounded-sm border border-border text-[10px] leading-none font-medium",
        className,
      )}
    >
      {mark.text}
    </span>
  );
}
