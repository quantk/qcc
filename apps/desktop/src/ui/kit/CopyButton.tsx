import { CheckIcon, CopyIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { store } from "../../state/store";
import { copyText } from "../../util/clipboard";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** How long the check mark stays after a copy. */
const CONFIRM_MS = 1500;

/**
 * A small button that copies `text`: its icon turns into a check for a moment once the
 * clipboard took it, and a refusal is told as a notice.
 */
export function CopyButton(props: { text: string; label?: string; className?: string }) {
  const { text, label = "Copy", className } = props;
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), CONFIRM_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    if (await copyText(text)) setCopied(true);
    else store().notify("Could not copy to the clipboard");
  };

  return (
    <Tooltip>
      <TooltipTrigger
        delay={300}
        render={
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label={copied ? "Copied" : label}
            className={cn("text-muted-foreground hover:text-foreground", className)}
            onClick={() => void copy()}
          />
        }
      >
        {copied ? <CheckIcon /> : <CopyIcon />}
      </TooltipTrigger>
      <TooltipContent>{copied ? "Copied" : label}</TooltipContent>
    </Tooltip>
  );
}
