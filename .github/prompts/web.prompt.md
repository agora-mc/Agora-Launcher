---
description: "Build the Next.js static web directory. Use after any web/ change."
name: "web"
agent: "agent"
---
Build the web directory by running the Next.js production build:

```
cd web && npm run build
```

If the build fails, fix the errors in `web/` and re-run until it succeeds, then report the outcome.
