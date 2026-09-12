# Experimental Agora plugin SDK

This package supplies TypeScript declarations for the host's `agora` module. It is not published to npm and is not an API stability commitment.

Install it from a checkout with `npm install --save-dev /path/to/Agora/sdk`. In your plugin, use `import { instances, storage } from 'agora'`. Compile to ES modules and keep `agora` external in your bundler. The launcher supplies that module at runtime; running the stub in Node intentionally fails.

Start with `examples/plugins/dashboard` and the author guide in `docs/plugins/README.md`. No Node APIs, native dependencies, unrestricted fetch, or Tauri commands are available to scripts. Bundle any other dependencies into your JavaScript; local relative ES module imports are supported inside the package.

## Checking your work without the GUI

```bash
agora plugin preview development ./my-plugin
```

reads and validates the manifest and prints exactly what a user would be asked to agree to, without
installing anything. It is the fastest way to find a manifest mistake. `agora plugin install
development ./my-plugin` then loads the folder in place, so you can edit and reload rather than
repackaging.
