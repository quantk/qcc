import type { Virtualizer } from "@tanstack/react-virtual";

/** The parts of the virtualizer this reaches into. They are private, so a test pins them. */
type Internals = { _canWriteScroll: () => boolean; _reportedOverscroll: boolean };

/**
 * Keep the virtualizer from writing the scroll position while `busy` says so.
 *
 * As entries above the view are measured, the virtualizer makes up for their guessed
 * heights by moving the scroll position. WebKit ends a wheel step's smooth scrolling
 * at any such write, so scrolling up through many small entries lost about half of
 * each step. The virtualizer already knows not to write
 * while iOS momentum scrolling runs: it shifts the drawn rows instead and settles the
 * position once the scrolling stops. This asks it to do the same while `busy` holds.
 */
export function deferScrollWrites<T extends Element>(virtualizer: Virtualizer<T, Element>, busy: () => boolean) {
  const internals = virtualizer as unknown as Internals;
  internals._canWriteScroll = () => !busy() && !internals._reportedOverscroll;
}
