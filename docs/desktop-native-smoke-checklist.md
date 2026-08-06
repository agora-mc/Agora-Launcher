# Desktop Native Smoke Checklist

This checklist must be run manually against a real Tauri build (not Vite-only dev).
Check each item after building with `npm run tauri build` (or `cargo tauri dev` for iterative testing).

## Prerequisites

- [ ] Build succeeds (`cargo tauri build`)
- [ ] Application launches without console errors
- [ ] No Tauri IPC panic traces in the terminal

## 1. Startup & First Run

- [ ] Fresh install (delete `local_state.db`) shows Onboarding WelcomeStep
- [ ] Branded splash screen appears before onboarding loads
- [ ] Onboarding: download registry succeeds
- [ ] Onboarding: skip registry shows Continue Without Catalog warning, Browse is empty
- [ ] Onboarding: Github device flow opens browser, poll succeeds
- [ ] Onboarding: cancel device flow, later "do this later" navigates forward
- [ ] Onboarding: Back preserves service toggle choices
- [ ] After onboarding completion, app returns to Home

## 2. Theme & Appearance

- [ ] Light/dark/system theme switching works without flash
- [ ] Custom accent color survives restart
- [ ] Windows Accent color detection (on supported Windows) does not overwrite custom accent
- [ ] Hanging Windows accent request does not block app rendering (close Tauri immediately if unresponsive)

## 3. Registry

- [ ] `getRegistryStatus` returns correct cached/uncached state on cold start
- [ ] `checkRegistryUpdate` downloads and verifies the registry signature
- [ ] Registry update with network available: updates cleanly
- [ ] Registry update with network unavailable: shows Retry, cached data intact
- [ ] Browse loads categories and items when registry is available
- [ ] Browse shows recovery panel when registry is missing

## 4. Browse (native)

- [ ] Search by query filters across curated registry items
- [ ] Category filter works
- [ ] Content-type filter works
- [ ] MC version + loader filters work
- [ ] Sort options work (Net Score, Trending, Newest, Most Upvoted, Most Downvoted)
- [ ] Pagination: scroll-to-load-more retrieves next page without duplicates
- [ ] Out-of-order requests: rapidly changing filters does not display stale results
- [ ] Pagination failure shows Retry button, retry succeeds

## 5. Instance Management

- [ ] Create instance with valid name/version/loader succeeds
- [ ] Create instance progress events displayed
- [ ] Create instance error (invalid name, missing loader) shows inline error
- [ ] Instance appears in list after creation
- [ ] Edit instance: name, MC version, loader change persists
- [ ] Delete instance moves folder to trash
- [ ] Locked instance shows lock icon, cannot be modified

## 6. Launch (Direct)

- [ ] Health scan runs before launch
- [ ] Health warnings show dialog, "Launch Anyway" continues, button shows Running PID
- [ ] Health blockers with filename show Disable button; clicking re-runs health
- [ ] Health finding without filename hides Disable button
- [ ] Game console output visible after direct launch
- [ ] Game-exited event clears Running state, returns to stopped/launchable
- [ ] Kill button terminates the process, state resets
- [ ] Concurrent launch attempted while running shows disabled Kill button
- [ ] Health dialog Cancel cancels launch flow (no process started)
- [ ] Navigating away and back preserves running state
- [ ] Reloading the window preserves running state (or recovers from backend)

## 7. Launch (Delegated / Mojang launcher)

- [ ] Health scan runs before launch
- [ ] "Launch Anyway" hands off to official launcher
- [ ] No fake "Running (PID ...)" state for delegated launch
- [ ] Delegated launch does not create a console view

## 8. Crash Investigation

- [ ] Crash log file detection after a launched instance crashes
- [ ] Manual paste-log: pasted content is analyzed
- [ ] Suspect list shows rank, evidence sources, and confirmations
- [ ] Test-disable a candidate by filename
- [ ] Restore disabled mod after ruling out candidate
- [ ] Investigation cancel restores original mod state

## 9. Snapshot & Restore

- [ ] Automatic snapshot created before install/update/remove
- [ ] Last Known Good marker established after successful launch
- [ ] Restore to Last Known Good returns files and manifest to saved state
- [ ] Diff view shows changes since Last Known Good

## 10. Settings

- [ ] Modrinth Integration toggle saves and persists
- [ ] AI Chat toggle hides/shows AI tab in sidebar
- [ ] MCP token shown/hidden with Show/Hide toggle
- [ ] MCP token regenerate requires confirmation
- [ ] Launcher path Save persists the value
- [ ] Launch mode Direct/Delegation toggle persists
- [ ] One setting failure (e.g. ai_mcp_enabled unreachable) does not block other sections

## 11. Command Palette

- [ ] `Ctrl+K` / `Cmd+K` opens palette
- [ ] Sidebar "Quick actions" button opens palette
- [ ] Shortcut is suppressed when typing in an input field
- [ ] Arrow keys navigate but never select a section heading
- [ ] Enter on an instance row opens that instance's editor
- [ ] Enter on a nav row navigates to that tab
- [ ] Escape closes the palette
- [ ] Empty query shows "No results found."

## 12. Sidebar & Navigation

- [ ] Each tab navigates to correct page
- [ ] Browse → mod → browser Back returns to Browse
- [ ] Instances → editor → browser Back returns to Instances
- [ ] Collapse button collapses sidebar to icon-only; expand restores
- [ ] aria-current="page" set on active tab

## 13. Offline

- [ ] Disconnect network, launch app: banner shows registry is offline
- [ ] Cached registry: Browse still shows items
- [ ] Uncached: Browse shows recovery panel, Retry shows error, no crash

## 14. Build & Packaging

- [ ] `npm run build` succeeds
- [ ] `cargo test --workspace` passes
- [ ] No `console.log` or `debugger` statements in production code
- [ ] Binary launch from installed location works
