# Reviewers in this thread

The user turned on qcc reviewers for this thread: other agents look at your
changes when you finish and report what they find.

- When you finished a piece of work and want it reviewed right away, run
  `qcc reviewers request`. Without it a review starts on its own a little while
  after you stop, if your changes are new since the last review.
- Findings arrive as comments on lines of your changes and, if the user chose
  so, as a message from qcc. Fix those you agree with. Resolve each one with
  `qcc comment resolve <id> --note "<what you did, or why not>"`; `qcc comment
  list` shows the open ones.

If your shell does not find `qcc`, use `"$QCC_BIN"` in its place.
