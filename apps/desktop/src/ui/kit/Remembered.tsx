// State of what is drawn for one entry of a list that outlives its drawing. A
// virtualized list unmounts what scrolls out of view; what the user did there, such
// as opening a trace, is kept here and found again when the entry comes back.

import { type ReactNode, createContext, useCallback, useContext, useMemo, useState } from "react";

type Slot = { memory: Map<string, unknown>; key: string };

const RememberedContext = createContext<Slot | null>(null);

/** Gives what is drawn inside the state remembered under `entryKey` in `memory`. */
export function Remembered(props: { memory: Map<string, unknown>; entryKey: string; children: ReactNode }) {
  const { memory, entryKey } = props;
  const slot = useMemo(() => ({ memory, key: entryKey }), [memory, entryKey]);
  return <RememberedContext value={slot}>{props.children}</RememberedContext>;
}

/**
 * `useState` that is kept while its entry is out of view. `name` tells apart the
 * states of one entry. Outside a `Remembered` it is plain `useState`.
 */
export function useRemembered<T>(name: string, initial: T): [T, (value: T) => void] {
  const slot = useContext(RememberedContext);
  const id = slot ? `${slot.key}\u0000${name}` : "";
  const [value, setValue] = useState<T>(() => (slot && slot.memory.has(id) ? (slot.memory.get(id) as T) : initial));
  const remember = useCallback(
    (next: T) => {
      if (slot) slot.memory.set(id, next);
      setValue(next);
    },
    [slot, id],
  );
  return [value, remember];
}
