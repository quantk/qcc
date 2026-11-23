import { type ReactNode, useEffect, useState } from "react";

import { type Timing, durationLabel, timingHint, untilShown } from "../../state/durations";
import { useNow } from "../../util/useNow";

/**
 * How long an action took, or has been going: nothing for a quick one. A running
 * action waits without a clock until it is worth counting, and only then ticks, so
 * the clock redraws this text and not the timeline around it.
 */
export function Duration(props: {
  timing: Timing | undefined;
  running: boolean;
  className?: string;
  /** Drawn before the duration when there is one, as a separator. */
  prefix?: ReactNode;
  /** Wraps what is drawn, so a slot that adds spacing is left out with it. */
  as?: (props: { title: string; children: ReactNode }) => ReactNode;
}) {
  const { timing, running } = props;
  const [shown, setShown] = useState(false);
  const startedAt = timing?.startedAt;
  useEffect(() => {
    if (!running || startedAt === undefined || shown) return;
    const timer = setTimeout(() => setShown(true), untilShown({ startedAt }, Date.now()));
    return () => clearTimeout(timer);
  }, [running, startedAt, shown]);
  const now = useNow(running && shown ? 1000 : null);
  const label = durationLabel(timing, running, now);
  if (!label || !timing) return null;
  const title = timingHint(timing);
  const children = (
    <>
      {props.prefix}
      {label}
    </>
  );
  if (props.as) return props.as({ title, children });
  return (
    <span className={props.className} title={title}>
      {children}
    </span>
  );
}
