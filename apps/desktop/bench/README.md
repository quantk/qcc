# Benchmark of the Changes tab

Renders `ChangesView` (or `ChangesPanel` with `?view=panel`) on a recorded large change,
in a production build in headless Chrome, and prints what it measured as one JSON line.
The daemon is mocked: it answers from the recorded data, `thread.diff` with `paths` after
the delay a real daemon takes (20 ms, and 4 more per file).

```sh
PATH=$PWD/../../target/release:$PATH bench/data.sh   # once: records bench/data (not committed)
nix develop ../..#desktop -c bench/prod.sh            # build, serve, run the tab and the panel
```

What the numbers mean:

| Key | |
| --- | --- |
| `listShown` | ms from mounting to the list of changed files on screen |
| `jumpToLines` | ms from clicking the 150th file in the list to its lines on screen |
| `settled` | ms from mounting until nothing more is drawn (diffs read, highlighting done) |
| `mountLongTasksMs`, `maxLongTaskMs` | main-thread tasks over 50 ms until then: the window is frozen during each |
| `domNodes`, `domNodesAfterScroll` | elements in the page |
| `scroll40FramesMs`, `worstScrollFrameMs` | scrolling 800 px a frame, 40 times |
| `bytesTotal` | diff text the daemon sent |
