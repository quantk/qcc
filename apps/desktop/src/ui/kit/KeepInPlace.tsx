// Opening or closing something in a scrolled list changes the height of what is
// around it. WebKit has no scroll anchoring, and a list that follows its end moves
// everything up as it grows, so what the reader clicked could leave the screen.

import { type ReactNode, createContext, useCallback, useContext, useLayoutEffect, useRef } from "react";

/** What a scrolled list offers to what is drawn in it. */
export type Place = {
  /** The element that scrolls. */
  viewport: () => HTMLElement | null;
  /** The reader changed something: keep the view still, instead of following the end, until they scroll. */
  pin: () => void;
};

const PlaceContext = createContext<Place | null>(null);

/** Lets what is drawn inside keep itself in place in `place`. */
export function KeepInPlace(props: { place: Place; children: ReactNode }) {
  return <PlaceContext value={props.place}>{props.children}</PlaceContext>;
}

/**
 * Gives `keep(element)`: called right before a change this component draws, it
 * keeps `element` where it is on the screen once the change is drawn, in the same
 * frame. Outside a `KeepInPlace` it does nothing.
 */
export function useKeepInPlace(): (element: Element | null) => void {
  const place = useContext(PlaceContext);
  const kept = useRef<{ element: Element; top: number } | null>(null);
  // After every commit of the component, so after the one the change made.
  useLayoutEffect(() => {
    const current = kept.current;
    kept.current = null;
    const viewport = place?.viewport();
    if (!current || !viewport || !current.element.isConnected) return;
    const moved = current.element.getBoundingClientRect().top - current.top;
    if (Math.abs(moved) > 0.5) viewport.scrollTop += moved;
  });
  return useCallback(
    (element: Element | null) => {
      if (!place || !element) return;
      place.pin();
      kept.current = { element, top: element.getBoundingClientRect().top };
    },
    [place],
  );
}
