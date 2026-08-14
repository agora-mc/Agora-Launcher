# V5 — Porting the prototype into Agora

Plan for reconstructing the prototypes inside the real desktop app, and for splitting them into a
**global ambience layer** (terrain, animals, weather, music, buddy, particles) and the **High
Interaction UI** that sits on top of it.

Covers four surfaces: the instance editor (§1–8), **Simple mode** (§10), **Browse** (§11) and
**the Lab** (§12).

Read docs\interactive\prototypes\README.md first for what the prototypes establish, and
docs\interactive\prototypes\V4-FIX-PLAN.md for the defect history — several fixes in there encode traps you
will hit again during the port. You might also find docs\interactive\prototypes\V4-WORLD-PLAN.md
useful for understanding the background world.

# THE GOAL
Make a highly graphical, friendly, playful, interactive UI mode for kids, graphically thinking individuals, 
and people that like a more interactive experience. Simplicity in terms of features can be intentional we
can add them later if they fit (If you ahve a suggestion let me know!). High interaction mode and agora lab
were intended to be an enjoyale "game before the game" to let users enjoy using agora and remove the complexity
advanced features, or information irrelevant to the average user. Standard mode meets the needs for advanced features,
high interaction is for fun and ease of use. Its an alternative not a replacement to the agora standard guide 
and instance editor. To meet this goal, we are completely rewriting the instance editor High Interaction mode and 
agora lab, as well as adding a new browse high interaction mode. we are also adding an interactive background 
throughout agora as an option for users.


---

## 0. Prime directive: PORT, DO NOT REWRITE

The v4-world.html prototype is not a sketch. It is ~2,500 lines of working, debugged, empirically verified code:
54 easter eggs driven through their real handlers, 205 terrain positions checked for alignment, a
pond hit region validated against its own renderer, 13 music tracks verified note-for-note against
the score. Every one of those numbers came from finding something broken and fixing it.

**A rewrite throws that away and silently reintroduces the bugs.** The failure mode is not "the
rewrite looks different" — it is "the rewrite looks fine and the terrain is off by one column
again".

So the rule for every system in section 2:

> Copy the function across. Change `var` → `const`/`let`, add types, keep **the same function
> names, the same variable names, the same formulas, and the same comments**. The comments are not
> decoration — they record why the code is shaped that way, and most of them exist because
> something was wrong first.

You should be able to put the prototype function and the ported function side by side and see the
same code in two dialects. If you cannot, you rewrote it.

**Specifically do not:**

- Turn entities, particles, or terrain columns into React state. They update 60×/second; React
  state would re-render the tree 60×/second. They stay in plain mutable arrays owned by the engine.
- Convert canvas drawing into components, SVG, or DOM nodes.
- "Improve" the math. `Math.floor` in `groundYAt` is deliberate — `Math.round` was the original
  bug (F1), and it was wrong on 33% of positions by 12–24px.
- Replace the music scheduler with a grid or a `setInterval` per note. See §7.
- Refactor while porting. Port first, verify against the prototype, refactor later if at all.

**Where you may exercise judgement:** TypeScript types, file organisation, React lifecycle
(mount/unmount/cleanup), the settings surface, and anything the prototype faked because it had no
backend. Those are listed explicitly below.

---

## 1. What exists

| File | Lines | What it is |
|---|---|---|
| [`v4-world.html`](v4-world.html) | 2,541 | The instance editor. Single file, no build. |
| [`v5-lab.html`](v5-lab.html) | ~700 | The Lab, revamped. Six benches; three fully playable. |
| [`v5-browse.html`](v5-browse.html) | ~600 | Browse, revamped. Stalls, taste model, gacha, shelf. |
| [`music-tracks.js`](music-tracks.js) | 2,375 | 13 verified public-domain tracks, generated. |
| [`music-preview.html`](music-preview.html) | 401 | The reference audio engine + 7 instruments. |
| [`verify-eggs.js`](verify-eggs.js) | 421 | Drives all 54 eggs through their real handlers. |
| [`make-tracks.py`](make-tracks.py) / [`mxl2track.py`](mxl2track.py) | — | Regenerate the track data from MusicXML. |

Serve the folder to open any of them (`.claude/launch.json` → `prototypes`, port 5180);
`v4-world.html` and `music-preview.html` both load `music-tracks.js` by `src`.

---

## 2. The split

The prototype's own section banners map almost one-to-one onto the split. **A**mbience is global
chrome; **F**oreground is the High Interaction UI; **P**rototype-only is scaffolding that exists
because there is no real app underneath.

| Prototype section | Line | Goes to |
|---|---|---|
| data (136-mod fixture) | 492 | **P** — real data comes from the registry |
| sound (`blip`, `chord` SFX) | 531 | **A** — shared audio, both layers use it |
| background music | 544 | **A** |
| living background (sky, ridges, weather, day/night) | 722 | **A** |
| particles (click burst + cursor trail) | 849 | **A** |
| buddy (cursor companion) | 875 | **A** |
| achievements (toast) | 894 | **A** — fired by world events |
| living world: engine core | 903 | **A** |
| shelf (mod tiles, drag-to-reorder) | 1188 | **F** |
| status + preflight (health check) | 1359 | **F** |
| crash doctor | 1423 | **F** |
| living world: species, spawner, items, input | 1506 | **A** |
| egg registry + Field Journal | 2252 | **A** (detection) + **F** (the journal panel) |
| remove + undo | 2412 | **F** |
| prototype control bar (problem/weather/time/crash toggles) | 2426 | **P+A** — **port the controls but redeisgn is welcome** |

**The dividing question** is *"does this know anything about mods, instances, or launching?"*
Terrain does not. The health check does. Anything that does not is ambience and belongs to every
page; anything that does is High Interaction UI.

Two consequences worth stating plainly:

- **Easter eggs live in ambience**, so the world stays discoverable on every page. Finding the fox
  asleep on the Browse page is the point, not a bug.
- **The prototype control bar does not exist in the app.** Weather, time of day and the simulated
  problem are driven by the real world clock and real instance health. Porting those toggles would
  ship a debug menu.

---

## 3. Where the code goes

```
desktop/src/features/ambience/            ← NEW top-level feature, no interactive dependency
  engine/                                 framework-free TypeScript, no React, no tauri
    terrain.ts        ridge/carveBasin/groundYWorld/screenOf/paraX/paraY/waterSpan/pondHit
    sky.ts            day-night, stars, clouds, sun/moon arc
    weather.ts        rain, snow, lightning, rainbow
    world.ts          WORLD: entities, props, spawner, carry, update/draw
    species.ts        the 30-species catalogue + drawQuad and friends
    props.ts          the 43 props and their reactions
    eggs.ts           the 54-egg registry + findEgg
    particles.ts      burst + trail
    buddy.ts          cursor companion
    audio/
      sfx.ts          blip / chord
      music.ts        scheduler + 7 instruments + rolloff  (from music-preview.html)
      tracks.ts       generated; see §6
    engine.ts         owns the canvas + rAF loop; start/stop/resize/setProfile
  AmbienceCanvas.tsx  ONE React component. Mounts a canvas, starts the engine, cleans up.
  AmbienceProvider.tsx  context: profile, enabled, volume; reads settings
  useAmbience.ts      hook for UI that wants to react to world events (eggs found)
  ambience.test.ts    engine unit tests (pure functions: terrain, hit-testing, scheduler)

desktop/src/features/interactive/live/    ← existing app-boundary layer, High Interaction UI only
  (shelf, preflight, crash doctor, journal panel — see §5 phase 4)
```

### Why ambience is NOT under `features/interactive/`

This is not a style preference, it is forced by the boundary checker
(`desktop/scripts/check-interactive-boundaries.mjs`):

- It classifies by **first path segment** under `features/interactive/`, and only
  `domain`, `visual`, `lab`, `live`, `testing` are known areas. Anything else classifies as
  `other`, and `ALLOWED.other = null` — **fail-closed, so a new `interactive/ambience/` directory
  errors immediately.**
- `domain/`, `visual/` and `lab/` may not import `live/` or `@tauri-apps/*`, so they could not
  consume an ambience layer that reads settings.
- Conceptually it is backwards regardless: the background must serve Home, Browse, Instances,
  Guide, Settings and everything else. Those pages must never import from `features/interactive/`.

**The clean answer: no page imports ambience at all.** `App.tsx` mounts it once, behind everything,
and every page simply renders on top. `features/interactive/` never references it either — High
Interaction just happens to be a page with a livelier profile.

> Extend `check-interactive-boundaries.mjs` (or add a sibling check) so `features/ambience/`
> **cannot import `features/interactive/`, `@/lib/tauri`, or any page**. The one permitted
> outside-in dependency is `AmbienceProvider` reading settings. Add negative fixtures under
> `desktop/scripts/boundary-fixtures/` the same way the interactive check does — a boundary with no
> failing fixture is not enforced, it is just documented.

---

## 4. The architecture that makes this a port and not a rewrite

**One canvas, one rAF loop, one React component.** The engine is a plain TypeScript class that
owns its canvas and animation frame. React's entire job is: create the element, call
`engine.start()`, call `engine.stop()` on unmount, and forward profile changes.

```tsx
// AmbienceCanvas.tsx — the whole React surface of the ambience layer
export function AmbienceCanvas({ profile }: { profile: AmbienceProfile }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const engine = useRef<AmbienceEngine | null>(null);

  useEffect(() => {
    if (!ref.current) return;
    engine.current = new AmbienceEngine(ref.current);
    engine.current.start();
    return () => { engine.current?.stop(); engine.current = null; };
  }, []);

  useEffect(() => { engine.current?.setProfile(profile); }, [profile]);

  return <canvas ref={ref} className="ambience-canvas" aria-hidden="true" />;
}
```

Everything else in `engine/` is prototype code in TypeScript. No hooks, no state, no JSX.

**Mounting.** In `App.tsx`, above the page switch and below nothing:

```tsx
<AmbienceProvider>
  <AmbienceCanvas profile={profile} />   {/* z-index 0, position fixed */}
  <div className="app-shell">...existing sidebar + pages...</div>
</AmbienceProvider>
```

The existing shell needs a stacking context above the canvas and, where pages currently assume an
opaque background, a translucent panel treatment (the prototype's `--glass` + `--edge` tokens are a
working reference).

> **That stacking context must be explicit — `.app-shell { position: relative; z-index: 2 }`.**
> This bit us once and the failure is silent. `.ambience-canvas` is `position: fixed; z-index: 0`,
> which paints it above *every non-positioned element*, and the shell wrapper is non-positioned.
> It appeared to work only because `backdrop-filter: blur(2px)` on `main` incidentally created a
> stacking context. The Living Background page sets `backdrop-filter: none` — deliberately, so the
> world is unobstructed — which removed that accident, and the entire page dropped behind the canvas:
> invisible and unclickable, with no error anywhere. Never rely on a blur, transform, opacity or
> filter to lift the UI above the world; those are visual choices and any page may turn them off.

**Events.** The engine attaches its own `pointermove`/`click` listeners, exactly as the prototype
does. It must **not** call `stopPropagation` — see §7.

---

## 5. Phases

Each phase is independently verifiable and independently shippable. Do not start a phase before the
previous one's gate passes.

### Phase 1 — Ambience skeleton, static

Port `terrain.ts` + `sky.ts` and the engine loop. No animals, no weather, no music. Mount in
`App.tsx` behind a hard-coded `false` flag; render it in a Storybook-style dev route or just flip
the flag locally.

*Gate:* ridges render at the right scale on resize; `groundYWorld` returns the same values as the
prototype for a fixed viewport (write the comparison test — it is 20 lines and it catches the
`Math.floor` regression permanently); zero console errors; `npm run build` clean.

### Phase 2 — Settings and profiles

`AmbienceProvider` reads a real setting, and the canvas responds.

| Profile | Terrain | Weather | Animals | Eggs | Buddy | Particles | Music |
|---|---|---|---|---|---|---|---|
| `off` | — | — | — | — | — | — | — |
| `calm` | ✓ | ✓ | few | — | — | — | optional |
| `full` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

- Default **`off` globally**; High Interaction forces at least `calm` and defaults to `full`, since
  the living world *is* that mode.
- Settings UI: one toggle ("Living background"), one profile select, one music volume, one
  "reduce motion" note pointing at the OS setting.
- Persist via the existing settings mechanism used by the rest of Settings (`getSetting`/
  `setSetting`) — ambience is the only place allowed to touch it.

*Gate:* toggling in Settings starts/stops the loop with no leak (see §7 leak test); the setting
survives a restart; `off` creates no canvas and no rAF.

### Phase 3 — The living world

Port `world.ts`, `species.ts`, `props.ts`, `particles.ts`, `buddy.ts`, `eggs.ts`. This is the bulk
of the port and the part most at risk of being rewritten — resist.

*Gate:* **port `verify-eggs.js` to a real test.** It already drives all 54 eggs through their
handlers; adapt it to run under vitest against the engine (no DOM needed for most of it). It must
report 54/54 once F3 and F10 from the fix plan are in. This is the single highest-value test in the
whole port, because egg chains break silently.

### Phase 4 — High Interaction UI on top

Now port the foreground: shelf, preflight health check, crash doctor, journal panel, remove/undo.
These go in `features/interactive/live/` and must obey the existing rules — React returns
decisions via callback, operations go through `invoke()`, shared visuals emit `VisualIntent` only
and may not take operation-shaped callback props (AST-checked).

Replace prototype fakes with real data: the 136-mod fixture becomes the real instance contents with
each mod's real `icon_url` (which the Standard view already renders and the old live adapter
discarded — that was TERRA-6b); the simulated problem becomes real health-report state; the crash
doctor's suspects come from real crash signatures.

*Gate:* `npm run build` (includes `check:boundaries`), `npm run test:unit`, and the interactive
boundary fixtures still all fail as designed:
```bash
node scripts/check-interactive-boundaries.mjs --root scripts/boundary-fixtures/interactive --fixtures
```

### Phase 5 — Music

Port `music.ts` from `music-preview.html` (**not** the sketch in `V4-FIX-PLAN.md` A1a — it uses a
16th-note grid that cannot represent the data; see §7). Ship `tracks.ts` per §6.

*Gate:* every track's voices sum to `beats` (keep the load-time assertion as a unit test); opening
frequencies match the score for at least Mountain King, Fate and Canon; audio context suspends on
`document.hidden` and resumes on focus; nothing plays before a user gesture.

### Phase 6a — Simple mode  (§10)
### Phase 6b — Browse       (§11)
### Phase 6c — The Lab      (§12)

Independent of each other; do any order. All three assume Phase 4's foreground exists.

### Phase 7 — Polish and perf

Reduced motion, DPR cap, entity cap, frame budget, battery behaviour, and the accessibility pass
(`aria-hidden` on the canvas, live region for egg announcements — both already correct in the
prototype).

---

## 6. Music data

`music-tracks.js` assigns to `window.MUSIC_TRACKS`. For the app, convert it to a module:

```ts
// features/ambience/engine/audio/tracks.ts — GENERATED, do not hand-edit
export interface MusicTrack { id: string; name: string; composer: string; year: number;
  marking: string; mood: Mood; bpm: number; beatsPerBar: number; bars: number; beats: number;
  source: string; instrument: InstrumentId; tempoNote?: string; voices: MusicVoice[]; }
export const MUSIC_TRACKS: MusicTrack[] = [ /* … */ ];
```

Change `make-tracks.py`'s `HEADER`/`FOOTER` to emit the module form instead of the `window` form —
**do not fork the data by hand.** Keep the load-time voice-sum assertion; move it into a unit test
so it runs in CI rather than throwing in production.

~192 KB of note data is a real bundle cost. Lazy-import `tracks.ts` on first play so it never lands
in the initial chunk.

Licensing is settled and recorded in the generated file's header — carry that header across.

---

## 7. Traps

Every one of these was a real bug in the prototype. They are listed because a port re-creates the
conditions for them.

1. **Do not call `stopPropagation` in the world's click handler.** The global click-burst listener
   sits on `document`; stopping the bubble made the backdrop the one place clicking produced no
   feedback. The burst listener is now **capture-phase** so nothing downstream can suppress it
   again. Keep both properties.
2. **`document.hidden` pauses everything.** The update loop, the music context, and rAF itself all
   stop. Any test that drives the world must step frames manually — `verify-eggs.js` documents the
   workaround. A test that "passes" against a hidden pane may be testing nothing at all.
3. **`Math.floor`, never `Math.round`, for the ground column index** (F1).
4. **World coordinates vs screen coordinates** (F7). Anything tethered to the ground stores a world
   x and converts with `screenOf(wx, layer)`. Mixing the two makes entities slide over the terrain
   under mouse parallax. The fix plan has the call-site migration table.
5. **The pond is not a rectangle.** `pondHit` asks the same question the renderer does — which
   column is under the cursor, is it submerged, is the point between surface and floor. The old
   fixed 260×40 box missed the deep water entirely (the basin runs to 84px deep) and covered dry
   land. Verified point-for-point against the renderer across four parallax positions; keep them
   in sync or the test in `ambience.test.ts` will tell you.
6. **The music scheduler must not sample a fixed grid.** Real scores are not on one: Moonlight's
   arpeggios are ⅓ of a beat, Clair de Lune has 0.75 and 4.5. Each voice keeps its own cursor and
   walks its `[pitch, beats]` list. A 16th-note grid silently drops every event between slots.
7. **Keep the register rolloff and the high shelf.** Equal gain is not equal loudness; Sugar Plum
   reaches B7 with 27% of its notes above C6 and is genuinely painful without them. The tuning is
   correct (A4 = 440, middle C = C4 = 261.6) — do not "fix" harshness by shifting the octave map.
8. **Canvas is a replaced element.** `position:fixed; inset:0` leaves it at intrinsic 0×0. Set
   width/height in CSS *and* assign `canvas.width`/`canvas.height` in JS. `innerWidth` can be 0 on
   the first tick in an embedded frame — fall back and re-measure on load.
9. **Leaks.** Every `addEventListener`, `setInterval` and `requestAnimationFrame` the engine
   creates must be released in `stop()`. Write the test: mount/unmount 50 times, assert the rAF
   handle count and listener count return to baseline. React strict mode double-mounts in dev and
   will find this for you.
10. **Ambient spawning stalls** (F8). Residents (pond, perched, sleeping) need lifespans or they
    fill the entity cap and the world goes still — measured climbing 1→10 in 97s and never falling.

---

## 8. Gates

Run per phase; run all before calling the port done.

```bash
python scripts/check_architecture.py && python scripts/check_docs.py
```
```bash
npm run build --prefix desktop && npm run test:unit --prefix desktop
```
```bash
node desktop/scripts/check-interactive-boundaries.mjs --root desktop/scripts/boundary-fixtures/interactive --fixtures
```

`npm run build` is `check:boundaries` + `tsc` + `vite build`. **CI's frontend job runs `tsc` and
`vite build` directly and skips `check:boundaries`** — so a boundary violation will not be caught
for you. Run it locally, every phase.

E2E (`npx playwright test`, ~243 tests) when UI flows change. Expect to add cases for: ambience off
by default, toggling it on, and the High Interaction editor rendering above it.

---

## 9. Not in scope

- Porting `v1`–`v3`. They are superseded design checkpoints; keep them as reference.
- Any change to `agora-core`. The ambience layer is pure presentation and must not reach the Rust
  side at all beyond reading its own setting.
- The three unbuilt Lab benches (§12.3). Design them with the owner; do not invent them.

---

## 10. Simple mode

A third presentation preference, and the cheapest large win in this plan.

The insight: **High Interaction's *structure* is already simpler than Standard's — it is the
cosmetics that make it loud.** Play as one hero action, an icon shelf instead of a card list, a
bounded diagram, and a health check that runs before you launch is a genuinely easier product than a
control panel. Simple mode keeps all of that and removes the stimulation, which serves adults new to
modding, low-powered machines, and anyone who finds the full mode too much.

```ts
export type InteractionPreference = 'standard' | 'simple' | 'high-interaction';
```

`features/interactive/live/presentationPreference.ts` already versions the record and validates
against an explicit list, so adding a value means widening the guard in `loadPreference`. Keep
`version: 1`: an unknown value must still fall back to `standard`, which means an old build reading a
`simple` record degrades safely on its own.

**Keeps — the structure**

- Play as the hero action; the screen reads as save-select, not a control panel
- Inventory shelf of icon tiles using each mod's real `icon_url`
- Bounded neighbourhood diagram on selection (real curves, capped node count).
  **This needs a real dependency read, and that read did not exist.** Relationships were built only
  from `health.blockers[].loader_compatibility`, which (a) exists solely when the instance is
  unhealthy and (b) describes mod→*loader version* requirements whose targets rarely resolve to
  another installed node. A healthy instance therefore had an empty graph — no curves to draw, and
  `requiredBy` stuck at 0 so **every item came out `common`**. Rarity and curves die together
  because they share this one input. The fix is `get_dependency_graph`
  (`agora_core::dependency_ops::build_dependency_graph`): one read, filename→filename edges, both
  endpoints installed by construction. Rarity is then genuinely dependency-count-derived:
  0 → common, 1+ → rare, 4+ → epic, 10+ → legendary.
- The pre-flight health check, including "Show me" / "Play anyway"
- Advanced drawer, closed by default
- Plain language ("One mod is missing its file", never "1 blocker · 24 recommendations")

**Drops — the cosmetics**

- Ambience defaults `off`; `calm` is offered, `full` is not
- Music defaults off (still available)
- No rarity tiers, no animated conic borders, no XP/collection meter
- No achievements, eggs or Field Journal
- No cursor buddy, no click particles, no 3D tilt, no drag-to-reorder flourish
- The health check still animates, but as a plain progress pass — no scan wave pinging across the
  shelf and stopping on the broken tile

**Simple mode is not a degraded High Interaction.** The health check, the diagram and the icons are
the parts that make the mode *useful*; every one of them stays. What goes is decoration.

Implement it as one capability object derived from the preference, not scattered `if (mode === …)`
checks — those are how a third mode turns into three subtly different products:

```ts
const CAPS = {
  standard:           { shelf: false, diagram: false, ambience: 'off',  flourish: false, eggs: false },
  simple:             { shelf: true,  diagram: true,  ambience: 'off',  flourish: false, eggs: false },
  'high-interaction': { shelf: true,  diagram: true,  ambience: 'full', flourish: true,  eggs: true  },
} as const;
```

Note the interaction with §2: **eggs live in ambience**, so Simple mode disabling eggs is expressed
by its ambience profile, not by a second switch inside the egg registry.

*Phase:* slot after Phase 4 (the foreground exists to configure). Small — mostly configuration.

*Gate:* all three modes render the editor; the preference survives a restart; an unknown persisted
value falls back to `standard`; `simple` mounts no ambience canvas and no rAF.

# The 2 v5 prototypes
These two were started to assist you with getting going but are much less complete or tested than
the v4-world for the instance editor is. the rules here are looser. If you want to be creative and
finish them during the port, go ahead, but remember you do not have vision capabilities, so you will
have limited ability to test your creation. On these please to ask questions and seek assistance. If 
you disagree with how something was done here, ask, you might be right!

---

## 11. Browse — `v5-browse.html`

A High Interaction presentation for `routes/Browse.tsx`. Same audience as the instance editor, so
the same rules apply: graphical, playful, plain language, advanced things behind a drawer.

**What to port**

| Piece | Why it matters |
|---|---|
| Market stalls as the category switch | Categories become places you walk to, not a filter dropdown |
| The five-axis taste model (`cosy`, `wild`, `silly`, `pretty`, `tricky`) | Sorting you can *see* |
| 👍/👎 on every tile | One-tap feedback, no forms |
| The vibe bars | The recommender explains itself |
| "Surprise me" machine | Discovery for people who do not know what to search for |
| Collectible creature art | A shelf of inhabitants, not rows of text |
| The fit line | The health check, asked *before* you commit |

**Rules the port must keep**

- **The taste model stays legible.** The bars exist so the user can see why the order changed. A
  hidden recommender would be worse than an unsorted list — it would feel like the shop is hiding
  things. Do not replace it with an opaque score.
- **Owned items sort to the back** (`scoreOf` returns −99 for anything in the bag) so the shelf
  keeps offering new things rather than re-selling what you have.
- **"Surprise me" is weighted by taste and never returns something already owned.** Unweighted
  random gets annoying by the third crank.
- **The fit line must call the same compatibility logic as the pre-flight check.** Two
  implementations of "does this fit" will drift, and the one on the browse page will be the one that
  lies. Reuse, do not reimplement.
- **Creature art is a fallback, not the design.** `hashOf(name)` art is deterministic so a given item
  always looks the same; in production prefer the real `icon_url` and fall back to the monogram
  creature when it is missing.

Bag state is prototype scaffolding — in the app, "put it in my bag" maps to the existing staging
flow and goes through the reviewed operation path like any other install. It does not get its own
side channel.

*Phase:* after Phase 4. *Gate:* boundary check, unit tests, and an E2E case proving Browse still
works in Standard presentation.

---

## 12. The Lab — `v5-lab.html`

### 12.1 The one rule that is different here

**In the Lab, animation communicates causality, not decoration.**

This is the deliberate divergence between the two modes, and it is recorded as a decision rather
than drift. High Interaction's whole point is decorative motion. The Lab's whole point is teaching,
and in a cause-and-effect lesson a butterfly drifting past is noise competing with the signal the
learner is supposed to read.

Concretely, in the Lab every animation must be the visible consequence of something the learner just
did: the helper mod walking itself into the slot, the conflicting pair shaking apart, the confidence
bars filling, the experiment stepping through save → disable → launch. Nothing moves on its own.

**Therefore a Lab bench drops ambience to `calm` while it is open**, and restores the previous
profile on close. The Lab *map* may run at the user's normal profile; a bench may not.

### 12.2 Every bench is four steps

A single snap-and-done moment is a *demonstration*, not teaching. Each bench escalates through four
steps, and later steps stay locked until the earlier ones are done so difficulty only goes up:

| Step | What it does | Why it is there |
|---|---|---|
| **1. Try it** | Handle the pieces. Guided, cannot fail badly. | Learn what the parts are and how they behave. |
| **2. Guess first** | Commit to a prediction *before* seeing the outcome. | The highest-value step. Being wrong here is the point — a surprise is what makes a rule stick, where a correct guess just confirms. |
| **3. Transfer** | The same principle wearing different clothes. | The only way to tell understanding from pattern-matching. If step 1 works and step 3 doesn't, they memorised a sequence. |
| **4. Why** | The mechanism and the real vocabulary. | Explanation lands only once there is something to attach it to. |

Concretely:

| Bench | 1. Try it | 2. Guess first | 3. Transfer | 4. Why |
|---|---|---|---|---|
| Build it | place version + loader; wrong pair bounces | "a mod built for 1.21 goes into a 1.20.1 world — what happens?" | **work backwards**: given a mod, build a world that fits it | loaders patch game internals, so they are version-locked |
| Add stuff | Better Caves pulls its helper in; Terrain Redo refused | "you remove Core Helper — what happens to Better Caves?" | **two mods share one helper**: remove one, the helper stays; remove both, it leaves | dependencies, reference counting, version ranges, conflicts |
| Something broke | clues → suspect → experiment | "what result would prove your guess **wrong**?" | **the messenger case**: the crash names a mod that is not the cause | the name in an error is where it stopped, not why |

Two of these deserve special care in the port because they carry the actual insight:

- **Add stuff step 3 teaches reference counting physically.** The helper is not removed until the
  last mod needing it is gone. Verified in the prototype: helper arrives once for two mods, stays
  when one leaves ("the other mod still needs it"), leaves when the second goes.
- **Something broke step 2 teaches falsifiability**, which is why the correct answer is *"if it still
  crashes, my guess was wrong"* rather than the flattering *"if it works, I was right."* And step 3
  punishes trusting the error message: turning off the named mod does not help, because the real
  cause is the dependency it was waiting on.

Rules the port must keep:

- **A wrong prediction is never a dead end.** The step still completes, and the reveal explains why
  the wrong answer is wrong *and* why the right one is right. A bare "correct!" teaches nothing.
- **Confidence is never a percentage.** The Fix bench fills bars and says likely / maybe / unlikely.
  `80%` claims a precision that does not exist. (The prototype drives the bar widths from those
  numbers but never prints them — that is intentional.)
- **Every drag has a keyboard equivalent.** Enter/Space on a piece places it. Drag is never the only
  route, and the prototype's `draggable()` helper wires both.
- **The safety net is stated before the experiment, not after.** "A return point is saved first"
  appears next to the button, not in the result.
- **"More info" is where the vocabulary lives.** `instance`, `mod loader`, `dependency`, `conflict`,
  `crash triage`, `snapshot` — all correct, all behind a disclosure so the surface stays plain. The
  label is deliberately neutral: this mode is for anyone who thinks visually, not only for children,
  and the disclosure must not talk down to the adults using it.
- **The pieces are fictional on purpose — do not "improve" them into real product names.** The
  benches use **Loader A** and **Loader B**, not Fabric and Forge. The lesson requires a loader that
  *does not fit*, and hanging that on a real product would be fabricating a compatibility claim about
  someone else's software — the sort of thing a user reasonably reads as a statement of fact from us.
  Real loader names appear in exactly one place, the Why step, where they are named accurately and
  paired with "which versions each one supports changes over time, so check the loader's own page
  rather than trusting a lesson". Apply the same rule to any bench you add.
- **The last step closes the bench.** Its button reads "Got it — back to the workshop" and returns to
  the map, with a beat's delay so the badge lands on screen first. A teaching flow should end by
  putting the learner back where they can choose what to do next, not leave them holding a dialog.

The existing scenario files (`buildIt.ts`, `modIt.ts`, `fixIt.ts`) keep their logic — you are
replacing presentation and adding steps around them, not rewriting the decision gates.

### 12.3 The three unbuilt benches

`healIt.ts` (health check), `takeItOffline.ts` (offline readiness) and `undoIt.ts` (snapshots and
restore) are marked "Sketch only" in the prototype and are **not designed**. Build them out to completion 
following the patterns and educational value of the provided 3, if needed maybe explore the original 
labs existing currently in agora for inspiration.
Take them to the owner, or follow the established pattern: a physical object you manipulate, an
immediate visible consequence, plain language on the surface, real vocabulary behind the disclosure —
and the same four steps, including a prediction the learner can get wrong.

*Phase:* after Phase 4, independent of Browse. *Gate:* existing Lab tests
(`LabShell.test.tsx`, `scenariosRemaining.test.ts`, `decisionGate.test.ts`) still pass — the scenario
logic is not what changed.
