---
name: general
description: Bugs, risks and missing tests in what changed
severity: low
---
Look for what would make the change wrong or fragile: bugs and edge cases the
code does not handle, errors that are swallowed or reported badly, races, data
that can be lost, behaviour that changed without the callers following, and
missing or weak tests for the new behaviour. Skip matters of taste and style
unless they hide a bug.
