# V4 — Living World implementation plan

**For:** a fresh implementation session with no prior context.
**Deliverable:** `docs/interactive/prototypes/v4-world.html` — one self-contained HTML file.
**Start from:** copy `docs/interactive/prototypes/v3-maximalist.html` verbatim, then add to it.
Do not restructure v3's existing UI (hero card, shelf, detail panel, preflight, Crash Doctor). This
work is **additive** and almost entirely inside the background canvas layer.

---

## 0. What this is

The High Interaction mode is Agora's launcher UI for kids, graphical thinkers, and people who want a
playful experience. Design brief: *"a game before the game."* v3 established the layout and the
juice. v4 turns the background from a static parallax backdrop into a **living, clickable world with
50+ discoverable easter eggs and an achievement/journal system**.

The world is *deliberately* not useful. It is there to make the launcher a place people enjoy
opening. It must never block, obscure, or compete with the real UI (Play button, shelf, health).

### Success criteria

1. 50+ easter eggs implemented and discoverable, at least 10 of them multi-step.
2. Ambient wildlife wanders through continuously without any user input.
3. Every animal and prop responds to a click with an animation.
4. A Field Journal panel tracks discovery (found + silhouetted-unfound with hints).
5. Achievements fire and a completion percentage is visible.
6. `prefers-reduced-motion` disables ambient motion but keeps everything clickable and reportable.
7. Steady 60fps at 1080p with ~40 live entities. Never blocks UI input.
8. Still one file, no network, no external assets.

---

## 1. Architecture

Add one new IIFE section, `WORLD`, between the existing background renderer and the particle system.
The existing `bgFrame()` sky/ridge renderer stays as the **backdrop**; WORLD draws on top of it,
inside the same `#bg` canvas, before the vignette.

```js
var WORLD = {
  t: 0,                 // seconds since load
  entities: [],         // live wandering animals (spawned/despawned)
  props: [],            // fixed scenery, created once at init, never despawn
  carry: null,          // {item:'fish', sprite:'fish'} or null
  flags: {},            // arbitrary progress flags for multi-step eggs
  found: {},            // eggId -> true
  hover: null,          // entity currently under cursor
  groundY(x) {},        // terrain height at world x  (reuse R3 ridge array)
  spawn(species, opts) {},
  despawn(e) {},
  hit(px, py) {},       // topmost interactive entity at screen point
  update(dt) {},
  draw(ctx) {}
};
```

### Ground alignment (important)

v3 already computes three parallax ridge arrays `R1/R2/R3` where `R3` is the nearest. Entities must
walk **on** the terrain, not float. Implement:

```js
function groundYAt(x){                    // x in screen px
  if(!R3) return H*0.86;
  var off = (mx-0.5)*-8 - OVER;           // same offsets bgFrame uses for R3
  var i = Math.round((x - off)/12);
  i = Math.max(0, Math.min(R3.length-1, i));
  return R3[i] + (my-0.5)*-3.2;
}
```

Entities on `layer 0` (far) use `R1` and draw at 0.55 scale; `layer 1` uses `R2` at 0.75 scale;
`layer 2` (near) uses `R3` at 1.0 scale. Draw order: layer 0 → 1 → 2, so near things overlap far
things. Sky creatures (birds, bats) ignore ground and use their own `y`.

### Update loop

Extend the existing `bgFrame(ts)`. After the ridges and before weather:

```js
var dt = Math.min((ts - lastTs)/1000, 0.05); lastTs = ts;
WORLD.update(dt);
WORLD.draw(bx);
```

Keep weather/fireflies drawing **after** WORLD so rain falls in front of animals.

### Input

The `#bg` canvas is behind the UI and currently has no pointer events. Give it
`pointer-events:auto` and add `z-index:0` (UI panels are `z-index:2` and will still capture their
own clicks). Then:

```js
bg.addEventListener("pointermove", e => { WORLD.hover = WORLD.hit(e.clientX, e.clientY);
  bg.style.cursor = WORLD.hover ? "pointer" : "default"; });
bg.addEventListener("click", e => {
  var t = WORLD.hit(e.clientX, e.clientY);
  if (t) { WORLD.interact(t); e.stopPropagation(); }
  else if (WORLD.carry) WORLD.drop(e.clientX, e.clientY);
});
```

`WORLD.hit` tests entities in reverse draw order (nearest first) against their bounding box, then
props. Add a 6px forgiveness margin — these are small targets.

**Hover affordance:** when `WORLD.hover` is set, draw a soft 2px outline glow around that entity so
users learn that things in the world are clickable. This is the single most important discoverability
feature — without it nobody finds anything.

---

## 2. Sprite system

All art is drawn with `fillRect` at integer coordinates — blocky, matching the existing terrain. Do
**not** hand-author SVG paths or import images.

Define sprites as pixel-string matrices with a palette:

```js
var SPRITES = {
  fox: {
    pal: { o:"#E2761B", l:"#F5A64B", w:"#FFF3E0", k:"#2A1A0E", '.':null },
    px: [
      "...o......o...",
      "..oooo..oooo..",
      ".oolooooooloo.",
      ".okoooooooko o",  // k = eye
      ".oooowwwwoooo.",
      "..oowwwwwwoo..",
      "...wwwwwwww...",
      "..ll......ll.."
    ]
  },
  // ...
};

function drawSprite(ctx, name, x, y, scale, flip, tint){
  var s = SPRITES[name], px = s.px, h = px.length, w = px[0].length;
  for (var r=0;r<h;r++) for (var c=0;c<w;c++){
    var col = s.pal[px[r][c]]; if(!col) continue;
    var dx = flip ? (w-1-c) : c;
    ctx.fillStyle = tint || col;
    ctx.fillRect(Math.round(x + dx*scale), Math.round(y + r*scale),
                 Math.ceil(scale), Math.ceil(scale));
  }
}
```

Sprite scale: `2` at layer 2, `1.5` at layer 1, `1` at layer 0, multiplied by a per-species size.

**Animation** = swapping between 2–3 frame variants (`fox`, `fox_walk2`, `fox_curl`) on a timer, plus
transform tricks (y-bob, rotation via drawing offsets). Keep every sprite ≤ 20×16 cells.

Each species needs at minimum: `idle`, `walk1`, `walk2`, plus whatever its reaction needs
(`curl`, `sleep`, `stand`, `fly1`, `fly2`, …).

---

## 3. Entity behaviour model

```js
{
  species:"fox", layer:2, x:-40, y:0, vx:26, dir:1, scale:2,
  state:"walk", stateT:0, anim:0,
  hb:{w:28,h:20},              // hitbox in screen px
  night:false, day:true,        // when it may spawn
  gives:null,                   // item it can yield when clicked
  accepts:["fish"],             // items it reacts to
  onClick(e){}, onGive(item){}, update(dt){}
}
```

**State machine per entity.** Generic states: `walk`, `idle`, `react`, `flee`, `exit`. Each species
overrides `react`. When `stateT` expires, return to `walk` unless the state is terminal (`sleep`).

**Spawner.** Every 4–9s (randomised), if `entities.length < 12`, spawn a species eligible for the
current time-of-day/weather from the weighted table in §4. Spawn just off the left or right edge,
walk across, despawn 80px past the far edge. Rare species use low weights so sightings feel special.

---

## 4. Ambient wildlife catalogue

`w` = spawn weight, `T` = time (D day / N night / A any), `L` = preferred layer.

| # | Species | w | T | L | Ambient behaviour | Click reaction |
|---|---|---|---|---|---|---|
| 1 | Songbird | 10 | D | sky | Flies in shallow arcs, lands on tree branches | Chirps (3-note), flies up |
| 2 | Canada goose | 5 | D | sky | Spawns as a **V of 5–7**, crosses high | Honks; whole V honks in sequence |
| 3 | Seagull | 6 | D | sky | Hovers, drifts, occasionally dives | Screeches, **drops a feather** (item) |
| 4 | Deer | 6 | D | 1–2 | Walks, stops to graze (head down 3s) | Freezes, stares at cursor 2s, then **darts off fast** |
| 5 | Bear | 3 | D | 2 | Slow lumber, occasional sniff | Stands on hind legs, roars, drops back |
| 6 | Fox | 6 | A | 2 | Trots, pauses, ear-flick | **Spins 3×, curls up, sleeps** (Zzz particles, stays asleep) |
| 7 | Wolf | 3 | N | 1–2 | Prowls, head low | Howls; two offscreen wolves answer |
| 8 | Squirrel | 9 | D | 2 | Darts, stops, tail flick; climbs trees | Drops an **acorn**, runs up nearest tree |
| 9 | Hedgehog | 5 | A | 2 | Slow waddle, snuffles | Curls into a ball, rolls 20px, uncurls |
| 10 | Rabbit | 8 | A | 2 | Hops in bursts | Thumps foot twice, bolts offscreen |
| 11 | Owl | 4 | N | 1 | Perches on a tree, still | Hoots, **head rotates 360°** |
| 12 | Bat | 6 | N | sky | Erratic sine-wave flight | Squeaks, does a full loop |
| 13 | Frog | 5 | A | pond | Sits on lily pad | Ribbits, big arcing jump to another pad |
| 14 | Duck | 5 | D | pond | Swims; **3 ducklings trail** it | Quacks; ducklings snap into a straight line |
| 15 | Turtle | 3 | D | 2 | Extremely slow | Retracts into shell for 3s |
| 16 | Butterfly | 8 | D | 2 | Flutters randomly near flowers | **Lands on the cursor** and follows for 8s |
| 17 | Firefly | 9 | N | 2 | Drifts, pulses | Glows bright; nearby fireflies **sync their pulse** |
| 18 | Crow | 5 | D | 1 | Perches on scarecrow/fence | Caws; 3 crows burst offscreen |
| 19 | Woodpecker | 4 | D | 2 | Clings to a tree, pecks rhythmically | Pecks fast, **opens a hole** in the tree (permanent) |
| 20 | Raccoon | 4 | N | 2 | Rummages, looks around | Looks guilty, runs off carrying something |
| 21 | Moose | 1 | D | 1 | Very slow, very large | Bellows; birds scatter |
| 22 | Boar | 3 | A | 2 | Snuffles the ground | Digs, unearths a **truffle** (item) |
| 23 | Mouse | 7 | A | 2 | Very fast, hugs the ground | Squeaks, hides under the nearest rock |
| 24 | Badger | 2 | N | 2 | Digs periodically | Sprays dirt, disappears into a hole |
| 25 | Cat | — | A | 2 | **Only spawns after 60s of no interaction** | Purrs, hearts, follows the cursor indefinitely |
| 26 | Dragonfly | 6 | D | pond | Hovers over water, darts | Zips a fast figure-8 |
| 27 | Bee | 8 | D | 2 | Wanders between flowers | Buzzes, flies straight back to the hive |
| 28 | Snail | 2 | A | 2 | Crawls on a log, leaves a trail | Retracts; trail sparkles |
| 29 | Otter | 2 | D | pond | Swims, rolls | Floats on back, cracks a shell on its belly |
| 30 | Eagle | 1 | D | sky | Circles very high | Screeches, **dives** across the screen |

---

## 5. Fixed props

Created once at init, positioned along the terrain, persistent. Positions should be derived from a
fixed seed so the world is the same every load (recognisable = learnable).

| # | Prop | Placement | Click reaction |
|---|---|---|---|
| 1 | Oak tree ×3 | layer 2, spread | Shakes; drops an **acorn**; 10% chance a **squirrel** falls out instead |
| 2 | Pine tree ×2 | layer 1 | Shakes; drops a **pinecone**; in snow, dumps a snow puff |
| 3 | Flower patch ×3 | layer 2 | Flowers **bloom open**; bees arrive |
| 4 | Pond | layer 2, centre-left | Ripples; a **fish jumps** in an arc (catchable mid-air) |
| 5 | Rock ×4 | layer 2 | Tips over, reveals a **beetle** that scurries off |
| 6 | Fallen log | layer 2 | **Mushrooms sprout** along it |
| 7 | Berry bush ×2 | layer 2 | **Berries** appear and become pickable |
| 8 | Mushroom ring | layer 2, 7 mushrooms | Each puffs spores |
| 9 | Beehive | hangs in an oak | Bees swarm out; after 3 clicks **honey** drips (pickable) |
| 10 | Campfire | layer 2 | Click 3× to light; then crackles; attracts moths at night |
| 11 | Stump | layer 2 | Shows growth rings + a tiny "127 years" label |
| 12 | Cave mouth | layer 1, far right | Two eyes blink inside |
| 13 | Scarecrow | layer 2 | Arms spin; crows scatter |
| 14 | Lily pads ×3 | on pond | Bob; frogs land on them |
| 15 | Cattails | pond edge | Puff seeds into the wind |
| 16 | Boulder | layer 2 | Wobbles; after 5 clicks **rolls away**, revealing a hole |
| 17 | Signpost | layer 2 | Cycles silly directions ("Nether: 3km", "Home: you're soaking in it") |
| 18 | Anthill | layer 2 | Ants march out in a line |
| 19 | Sun / Moon | sky | Sun: brightens briefly. Moon: cycles phase |
| 20 | Clouds | sky | Click → that cloud rains on its own for 5s |

---

## 6. Carry / item system

Items: `acorn`, `pinecone`, `fish`, `berry`, `honey`, `feather`, `truffle`, `flower`, `firefly`,
`snowball`, `coin`, `key`, `shell`, `water`.

- Only **one** item carried at a time. Picking up a new one drops the old.
- The carried item draws attached to the cursor (small sprite, slight bob).
- Clicking an entity whose `accepts` list contains the carried item triggers `onGive`.
- Clicking empty ground drops it (it falls to the terrain and sits there, still pickable).
- Dropped items despawn after 60s with a small poof.
- Add a small carried-item indicator near the cursor with the item name on first pickup only.

---

## 7. Easter egg registry

Declarative. One entry per egg. The registry drives the Field Journal, achievements, and completion
percentage — **do not hardcode discovery logic anywhere else**.

```js
var EGGS = [
  { id:"fox-nap", name:"Sleepy Fox", hint:"Some animals get tired when bothered.",
    steps:1, tier:1 },
  { id:"bear-fish", name:"Fisherman's Friend", hint:"Bears are hungry. Ponds have food.",
    steps:3, tier:3 },
  // ...
];
function findEgg(id){ if(WORLD.found[id]) return; WORLD.found[id]=true;
  journalPop(id); checkAchievements(); }
```

### Tier 1 — single click (26)

| id | Name | Trigger |
|---|---|---|
| `fox-nap` | Sleepy Fox | Click a fox |
| `deer-stare` | Deer in Headlights | Click a deer |
| `bear-roar` | Bear Necessities | Click a bear |
| `bird-song` | Dawn Chorus | Click a songbird |
| `goose-honk` | Honk | Click a goose |
| `gull-scream` | Sky Rat | Click a seagull |
| `hog-ball` | Prickle Ball | Click a hedgehog |
| `wolf-howl` | Lone Howl | Click a wolf (night) |
| `squirrel-nut` | Nut Job | Click a squirrel |
| `rabbit-thump` | Thumper | Click a rabbit |
| `owl-spin` | Who? | Click an owl (night) |
| `bat-loop` | Echolocation | Click a bat (night) |
| `frog-jump` | Ribbit | Click a frog |
| `turtle-hide` | Shell Shock | Click a turtle |
| `butterfly-land` | Landing Pad | Click a butterfly |
| `firefly-sync` | Synchrony | Click a firefly (night) |
| `crow-scatter` | A Murder | Click a crow |
| `pecker-hole` | Knock Knock | Click a woodpecker |
| `raccoon-guilt` | Trash Panda | Click a raccoon (night) |
| `moose-bellow` | Timber! | Click a moose (rare spawn) |
| `boar-truffle` | Truffle Shuffle | Click a boar |
| `mouse-hide` | Squeak | Click a mouse |
| `flower-bloom` | Bloom | Click a flower patch |
| `pond-fish` | Something's Biting | Click the pond |
| `rock-beetle` | Under a Rock | Click a rock |
| `log-shroom` | Fungus Among Us | Click the fallen log |

### Tier 2 — two steps (14)

| id | Name | Chain |
|---|---|---|
| `acorn-squirrel` | Special Delivery | Get acorn from tree → give to squirrel |
| `berry-hog` | Berry Nice | Pick berries → give to hedgehog → it follows you |
| `flower-deer` | Flower Power | Pick a flower → give to deer → it stays and can be petted |
| `fish-otter` | Sharing is Caring | Catch a fish → give to otter → it juggles it |
| `pinecone-squirrel` | Pinecone Post | Pinecone from pine → give to squirrel → it buries it |
| `feather-scarecrow` | Dapper | Seagull feather → give to scarecrow → it wears it in its hat |
| `truffle-pond` | Plop | Boar's truffle → throw in pond → a huge fish surfaces |
| `honey-bear` | Sweet Tooth | Get honey (3 clicks on hive) → give to bear |
| `firefly-cave` | Night Light | Catch a firefly → carry into the cave → cave paintings glow |
| `snail-lily` | Snail Mail | Put the snail on a lily pad → it rides across the pond |
| `duck-line` | Duck Duck Goose | Click a duck 5× → ducklings form a perfect line |
| `acorn-pond` | Wishing Well | Throw an acorn in the pond → ripples → a coin surfaces |
| `bee-hive` | Bee Line | Follow 3 different bees back → they lead you to the hive |
| `boulder-hole` | Rolling Stone | Click the boulder 5× → it rolls away revealing a hole |

### Tier 3 — multi-step, require thought (14)

| id | Name | Chain | Hint shown when unfound |
|---|---|---|---|
| `bear-fish` | Fisherman's Friend | Click pond → fish jumps → **catch it mid-air** → give to bear → hearts, bear walks off with it | "Bears are hungry. Ponds have food." |
| `snowman` | Frosty | In snow weather, click 3 snow piles **in size order** (big→medium→small) → snowman builds → click it → it waves | "Build from the bottom up." |
| `constellation` | Stargazer | At night, click 5 stars in **brightness order** → constellation lights → a shooting star crosses | "The brightest one goes first." |
| `bear-feast` | The Great Feast | Bring the bear **fish + berries + honey** (any order) → happy dance → leaves a paw print | "Three courses." |
| `acorn-hunt` | Green Thumb | Bury acorns in **3 different spots** → advance the time slider a full cycle → saplings appear | "Plant, then wait a day." |
| `fairy-ring` | Fairy Ring | Click **all 7** mushrooms in the ring within 6 seconds → a fairy appears and sparkles | "All of them, quickly." |
| `rainbow` | Rainbow's End | Make it rain, then set weather clear during **daytime** → rainbow appears → click **both ends** → pot of gold | "Rain, then sun." |
| `wolf-pack` | Pack Leader | Click a wolf → 3 answer → click **each of the 3** before they leave → the pack runs across together | "Answer every call." |
| `campfire-tales` | Campfire Stories | Light the fire **at night** → fox, rabbit and deer gather one by one → click each while gathered | "Warmth draws a crowd." |
| `moonlit-rave` | Moonlit Rave | At night click 7 fireflies → they gather into a swarm → click the swarm → they spell **AGORA** | "Gather the lights." |
| `the-long-con` | The Long Con | Feed the fox a fish → it follows → lead it to the hole under the boulder → it fetches a **key** → key opens the cave → treasure chest | "Make a friend. Friends fetch things." |
| `migration` | Migration | Click the **lead** goose → the V circles → click again → they land → click again → they take off in a new formation | "Lead the leader." |
| `water-flowers` | Watering Can | Carry water from the pond → water all 3 flower patches → they grow huge → butterflies swarm | "Flowers are thirsty." |
| `full-day` | Time Traveler | Move the time slider through a complete night→day→night cycle without clicking anything else | "Just watch for a while." |

**Total: 54 eggs.** If any prove impractical, cut from tier 1 first and keep ≥50.

---

## 8. Field Journal + achievements

**Journal panel.** A new collapsed `<details class="panel">` below Advanced, titled
`Field Journal — 0 / 54`. Inside, a grid of small cards:

- **Found:** the egg's sprite icon (or a coloured dot), its name, and a one-line description.
- **Unfound:** a `?` silhouette, name hidden as `???`, and the `hint` string in muted text.
- Group by tier: "Sightings" (1), "Friendships" (2), "Mysteries" (3).
- A progress ring at the top showing completion %.

**Discovery toast.** When an egg is found, reuse the existing `.ach` achievement toast styling but
with a distinct accent so discoveries feel different from achievements:
`DISCOVERY · Sleepy Fox · 12/54`.

**Achievements** (reuse the existing `achieve()` function):

| Trigger | Achievement |
|---|---|
| 1st egg | First Discovery |
| 10 eggs | Naturalist |
| 25 eggs | Zoologist |
| 40 eggs | Ranger |
| All 54 | **Completionist** (gold, confetti storm, permanent crown badge on the hero card) |
| All tier-3 eggs | Puzzle Master |
| 5 night-only eggs | Night Owl |
| Pet 5 different animals | Gentle Hand |
| Carry every item type at least once | Collector |
| Find an egg on your first day (< 5 min) | Quick Study |

Persist `WORLD.found` and unlocked achievements to `localStorage` under
`agora-world-journal` (versioned, same pattern as v3's other state). This matters — nobody
completes a 54-egg hunt in one sitting.

---

## 9. Sound

Extend v3's existing `blip(freq, dur, type, vol)` WebAudio helper. Keep the global mute toggle;
default OFF. Give each animal a short synthesized voice, 2–4 notes:

- Birds: fast rising triangle notes. Owl: two low sine notes. Goose: sawtooth honk.
- Wolf: long sine glide down. Bear: low sawtooth rumble. Frog: short square burp.
- Discovery: rising 4-note arpeggio. Completionist: an 8-note fanfare.

Never play more than 3 voices at once — track active voices and drop extras.

---

## 10. Accessibility and performance (non-negotiable)

- **Reduced motion:** under `prefers-reduced-motion`, stop spawning wanderers, freeze ambient
  animation, and keep props static — but keep everything **clickable**, and still fire discoveries
  and journal updates. Reactions become an instant state change plus the toast, no tweening.
- **The world must never steal UI clicks.** All UI panels sit above `#bg`. Verify that clicking the
  Play button, tiles, tabs, and drawers never triggers a world interaction.
- **Keyboard:** the world is explicitly optional, decorative content and is not keyboard reachable.
  The Field Journal panel **is** fully keyboard accessible and lists every discovery in text — that
  is the accessible equivalent. Say this in a note inside the journal.
- **Screen readers:** `#bg` gets `aria-hidden="true"`. Discovery toasts go through a polite live
  region so they are announced.
- **Performance budget:** cap live entities at 12, particles at 200, and skip the world update
  entirely when `document.hidden`. Use one canvas — do not add more.

---

## 11. Gotchas already discovered (do not rediscover these)

1. **Canvas is a replaced element** — `position:fixed; inset:0` leaves it at intrinsic 0×0. It needs
   explicit `width:100vw; height:100vh` in CSS *and* `canvas.width/height` set in JS.
2. `innerWidth` can be 0 on the first tick in an embedded frame. Size canvases with
   `innerWidth || document.documentElement.clientWidth || 1280`, and re-size on `load` and one rAF.
3. **`.tile` has `overflow:hidden`** — anything meant to overhang a tile corner must be a child of
   `.slot`, not `.tile`.
4. **Terrain must be overscanned** (`OVER = 90px`) past both edges, or the parallax shift exposes
   sky at the screen edges.
5. Drag-to-reorder and click both fire on pointerup; v3 guards this with a `justDragged` timestamp.
   Any new world click handling must not break that guard.
6. Distributing links with `arr[idx % arr.length]` while filtering on `idx % 3` only ever hits two
   entries. Watch for the same class of bug when distributing spawn positions.

---

## 12. Suggested build order

Each phase should leave the file working and openable.

1. **Engine skeleton** — `WORLD` object, ground alignment, update/draw hooks, hit-testing, hover
   glow. Ship with one hard-coded walking fox to prove the loop.
2. **Sprite system** + 8 core species (fox, deer, bear, squirrel, rabbit, songbird, butterfly, frog).
3. **Spawner** with the weight/time table; verify day/night eligibility.
4. **Props** — pond, 3 oaks, flowers, rocks, log, campfire, bush, hive. Fixed seeded positions.
5. **Carry system** + items; the first two-step chain (`acorn-squirrel`) end to end.
6. **Egg registry + Field Journal + persistence**, wired to the 26 tier-1 eggs.
7. **Remaining species** (the other 22).
8. **Tier-2 chains** (14).
9. **Tier-3 chains** (14) — these need the most testing; each is a small state machine on
   `WORLD.flags`.
10. **Achievements**, completion ring, Completionist celebration.
11. **Sound pass**, then the reduced-motion and performance pass.

Verify after each phase that the shelf, Play, preflight, and Crash Doctor still work — the world
shares a canvas and an input surface with them.

---

## 13. Relationship to production

This stays a prototype. If the direction is approved, the port targets
`desktop/src/features/interactive/live/`, where the world becomes a single self-contained React
component (`<LivingBackdrop />`) with the egg registry as a plain data module. Nothing in the world
system may touch Tauri, instance data, or any Agora operation — it is purely decorative and must sit
entirely outside the existing safety boundary that
`desktop/scripts/check-interactive-boundaries.mjs` enforces.
