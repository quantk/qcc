---
name: security
description: Security problems in what changed
severity: medium
---
Look only for security problems: injection (shell, SQL, paths), secrets in code
or logs, missing checks of permissions or input that crosses a trust boundary,
unsafe deserialization, files or processes reached through user-controlled
paths, and weakened defaults. Say how each one could be exploited.
