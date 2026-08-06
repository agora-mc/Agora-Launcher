---
description: "Compile the Agora registry database and run a sanity check. Use after any registry/, loader-manifests/, or crash-signatures/ change."
name: "registry"
agent: "agent"
---
Compile the registry database and run the sanity check by executing:

```
cd compiler && python compile.py --skip-sign --out ../registry.db && python ../scripts/verify_db.py
```

Then summarize the result: report compilation success, any validation errors or warnings, and the final verification status of `registry.db`.
