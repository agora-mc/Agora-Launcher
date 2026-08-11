# Luna — Visual Regression Report

## LUNA-5 release-candidate sweep (2026-08-11)

**Build / commit:** `master`, uncommitted working tree. Tauri **debug** build
`d:\agora\target\debug\agora-desktop.exe` running against the Vite dev server
(`tauri.conf.json` → `devUrl http://localhost:5173`), so the running app reflects the working copy.

**Environment:** Windows 11, 2560-wide primary display (OMEN 27k), app maximised. User's own appearance
preset: dark colour mode, custom yellow accent, "Bookish serif" font, comfortable density, soft corners,
decorative background effects on, motion "Follow system". This is a **non-default theme**, which makes it
a useful contrast check in its own right.

**Data:** real profile, four real instances. Primary subject: **"Copy of COBBLEVERSE"** — 136 mods,
49 resource packs, 3 shaders, 6 data packs, 1 real health warning, 24 real recommendations. This is the
first regression pass run against a large real instance rather than a fixture.

**Scope note:** this sweep was run *after* the TERRA-6 fix batch, to confirm the fixes render correctly
and nothing adjacent regressed. It is an execution report, not a redesign.

### Result summary

| Area | Result |
| --- | --- |
| Automated suites | **Pass.** Frontend unit 211/211 (24 files), e2e 243/243, boundary check OK (63 files), fixtures 24/24, `npm run build` green, root hermetic gates (architecture / docs / tauri-bindings) all OK. |
| High Interaction — large instance (136 mods) | **Pass.** Loads without stalling; 12-node spatial cap holds with `Show all 136 items (124 more)`. |
| High Interaction — instance bench | **Pass (fixed).** `Loader  fabric  Not verified` — the unverified state is now explicit (was blank). |
| High Interaction — content graph | **Pass (fixed).** Friendly names (`Advancement Plaques`, `Cobble Furnies`, `Better Third Person`) with the exact filename retained beneath each. **Zero** false `Warning` chips across all 136 nodes. |
| High Interaction — health lens | **Pass (fixed).** `0 blockers · 1 warning · 24 recommendations`, distinct headline vs body, `Content · this instance` for the unattributed warning, `Content · affects 1` for content recommendations. |
| High Interaction — escape controls | **Pass (fixed).** One `Use Standard view` button (was two stacked). `Refresh` and `← Back` present. |
| Health bridge → Standard | **Pass.** Leaves High Interaction, opens the reviewOnly `HealthDialog` for the correct instance, **0** Disable/Fix/Repair controls (§18.5 holds on real data). |
| Presentation preference | **Pass (fixed).** Survives a bridge round-trip; Settings still reads `High Interaction` after `Review health`. Editing an instance now opens directly into High Interaction. |
| Settings — Appearance | **Pass (new).** `Instance view` control present, correct default, explanatory copy renders at this width. |
| Lab — Mod It | **Pass (fixed).** Conflicts region, `Conflicts with Terrain Overhaul / Blocking` socket, `Blocker` + `conflicts 1` on both endpoints all render on the **played** path. |
| Lab — Mod It duplicate gating | **Pass (fixed).** Core Lib shows `Already proposed — review it in the staging dock.` instead of a dead `Stage install`. |
| Lab — selection screen | **Pass.** Six cards, progress persisted (`Build It — Complete`), no clipping at this width. |
| Standard UI | **Pass — no regression.** Instance list, editor header, tab strip, 136-row mod table, Health Check dialog all render as before. |

### Screenshot baselines captured (LUNA-2)

Captured during this sweep, at maximised width on the user's custom theme:

```text
High Interaction: bench + content graph (136 mods, post-fix)
High Interaction: health lens with 1 warning + 24 recommendations
High Interaction: bench loader row zoom ("fabric — Not verified")
Standard: instance editor with 136-mod table
Standard: reviewOnly Health Check dialog (0 repair controls)
Settings: Appearance with the new Instance view control
Lab: adventure selection (Build It complete)
Lab: Mod It step 1 / step 2 (satisfied socket) / step 3 (conflict visible)
```

### Notes and non-blocking observations

1. **Bundle size warning persists.** `dist/assets/index-*.js` is 1,284 kB (360 kB gzip), above Vite's
   500 kB warning. Pre-existing; the SAFE DEBT route-level lazy-loading note still applies. Not a
   regression from this batch.
2. **Content surface offers one verb.** Every one of the 136 nodes offers only `Stage removal`
   (install/update blocked on §19.5, enable/disable rejected). Renders correctly; flagged to Sol/Terra as
   a product question, not a visual defect. Recorded in SOL §22.5.
3. **Not re-checked this pass:** the full viewport/density/contrast matrix and reduced-motion fallbacks.
   TERRA-3's results stand and nothing in this batch touched motion or layout primitives, but a formal
   matrix sweep at 1366x768 / high-contrast / 200% text is still owed before release. Listed as the one
   open Luna item.
4. **Escalations raised:** none. No finding in this sweep required Terra or Sol judgement.

### Handoff

```text
Agent: Luna
Phase: LUNA-5 release-candidate sweep (post TERRA-6 fix batch)
Commit / branch / dirty status: master; uncommitted
Files changed: docs/interactive/VISUAL_REGRESSION_LUNA.md (new)
Tests run: unit 211/211, e2e 243/243, boundary 63 files OK, fixtures 24/24, npm run build,
  check_architecture.py / check_docs.py / check_tauri_bindings.py; manual visual sweep of the real
  launcher against a 136-mod instance (read-only: no install, remove, restore, launch, or sign-in).
How to launch/test: the debug build uses devUrl localhost:5173, so `npm run dev` + the running
  Tauri debug binary reflects the working copy. My Instances -> Copy of COBBLEVERSE -> Edit.
Known failures: none. One owed item: the full viewport/density/contrast/reduced-motion matrix.
Decisions made: none — execution report only.
Required next agent: none blocking. SOL-4 may proceed.
Why work is stopping: sweep complete and green.
```
