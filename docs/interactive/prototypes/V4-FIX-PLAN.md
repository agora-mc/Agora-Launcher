# V4 — fix pass

**Target file:** `docs/interactive/prototypes/v4-world.html` (edit in place).
**For:** a fresh session with no prior context.

The V4 build is structurally good and **most of it is correct** — do not rewrite it. All 54 easter
eggs are declared and wired, the tier-3 chains are real state machines, and every accessibility and
performance requirement was met. This pass fixes rendering and physics defects found in review.

**Do not break these (verified working):** 54/54 eggs with live `findEgg` call sites, tier split
26/14/14, 30 species, 43 props, entity cap of 12, `document.hidden` skip, reduced-motion guard,
Field Journal + `localStorage` persistence, `aria-hidden` on the canvas, live region, and the whole
v3 UI (hero, shelf, drag-to-rearrange, preflight, Crash Doctor).

Work through F1–F13 in order, then A1. Each is independently verifiable.

**F2 and F4 are already done** — see the ✅ markers. Do not re-apply them.

**Verify your work with `verify-eggs.js`** (same directory). It drives all 54 eggs through the real
click paths and prints a pass/blocked/fail table. Baseline before this fix pass: **47 / 54 reachable,
7 blocked, 0 unexpected failures.** After F3 + F10 it must read **54 / 54**. Run it after every fix —
it is the fastest way to catch an F7 coordinate regression, which otherwise fails silently.

**Priority.**
- **F1, F7, F8** decide whether the world feels alive at all — F1/F7 because nothing currently sits
  convincingly on the ground, F8 because ambient life stops permanently after a few minutes.
- **F10** is required for the Field Journal to be completable: three eggs are currently impossible
  because their items have no source. Without it the game cannot be finished.
- **F9** (pond) depends on F7's coordinate work — do it after.
- **F2, F3, F5, F11, F12** are contained and independent.
- **F4** (species silhouettes) is the one judgment call and can be deferred without breaking
  anything.
- **A1** (music) is an addition, not a fix — do it last.

---

## F1 — Ground alignment is off by one terrain segment (CRITICAL)

**This was an error in the original plan, not an implementation mistake.** The plan specified
`Math.round`; it must be `Math.floor`.

`bgFrame` draws each ridge segment as a rectangle starting at `i*step + off`, so the segment
covering screen x is `floor((x-off)/step)`. Rounding selects the *neighbouring* column for any x
past the segment midpoint.

**Measured impact:** across 205 sampled positions, 33% resolved to the wrong segment, off by
12–24px. Animals visibly sink into or float above the ground, and flicker between correct and wrong
as they walk.

**Location:** `function groundYAt(x,layer)`, currently line ~698.

```js
  var i = Math.round((x-off)/step);      // BEFORE
  var i = Math.floor((x-off)/step);      // AFTER
```

**Verify:** in the console, this must report 0 mismatches:

```js
let bad=0;
for(let x=60;x<W-60;x+=7){ if(Math.abs(groundYAt(x,2)-groundYAt(x+5.9,2))>0.5 &&
    Math.floor((x-((mx-0.5)*-8-OVER))/12)===Math.floor((x+5.9-((mx-0.5)*-8-OVER))/12)) bad++; }
console.log("segment mismatches:", bad);   // expect 0
```

Then watch any walking animal cross the screen — its feet must stay in contact with the terrain the
whole way, with no vertical popping.

---

## F2 — Quadruped legs anchored to the ground instead of the body — ✅ DONE

> **Already applied** (2026-08-11). `drawQuad` now pins each hip to the body underside and lifts the
> feet, and draws **four** legs in a diagonal gait instead of two. Left here for reference; skip it.

**Location:** `function drawQuad(...)`, currently lines ~772–778.

The body's underside sits at `gy - lh`. The current leg rect is drawn from `gy-lh+lift` with height
`lh-lift`, which pins the **foot** at `gy` and raises the **top** — so the leg detaches from the
body and a gap opens, up to 55% of leg length. Most visible on the bear (`lh:8`, `size:1.5`).

Also: quadrupeds currently draw only **2 legs**. They need 4 — a front pair and a back pair, with
the diagonal pairs in opposite phase.

```js
  if(shape.legs!==false){
    var lw2 = lw, hipY = Math.round(gy - lh);
    // foot lifts, hip stays fixed to the body underside
    var liftA = walking ? Math.max(0, Math.sin(phase))          * lh*0.5 : 0;
    var liftB = walking ? Math.max(0, Math.sin(phase+Math.PI))  * lh*0.5 : 0;
    ctx.fillStyle = pal.dark;
    // back pair (A phase), front pair (B phase) — diagonal gait
    ctx.fillRect(Math.round(bx0+bw*0.10), hipY, lw2, Math.max(1, lh-liftA));
    ctx.fillRect(Math.round(bx0+bw*0.26), hipY, lw2, Math.max(1, lh-liftB));
    ctx.fillRect(Math.round(bx0+bw*0.62), hipY, lw2, Math.max(1, lh-liftB));
    ctx.fillRect(Math.round(bx0+bw*0.80), hipY, lw2, Math.max(1, lh-liftA));
  }
```

**Verify:** a walking bear's legs must remain attached to its body at all times, with feet lifting
off the ground rather than the legs shrinking from the top.

---

## F3 — There is no fish (CRITICAL)

The pond currently spawns `SPECIES_BY_KEY.frog` with `fx:"fishjump"`. `"fishjump"` has **no case**
in `drawEntity`'s effect switch, so it falls through to `default: break` and `y` never changes
(verified: sampled for 1.05s, `y` was `0` for every frame). After 0.9s it flips to `state:"walk"`
and becomes a frog paddling in the pond forever.

This breaks `pond-fish` visually and, because nothing else ever produces a `fish` item, blocks
**four** eggs — verified by driving each chain in the browser:

| Egg | Tier | Blocked because |
|---|---|---|
| `bear-fish` | 3 | needs a caught fish |
| `fish-otter` | 2 | otter accepts `fish`, nothing yields one |
| `bear-feast` | 3 | needs fish + berry + honey |
| `the-long-con` | 3 | the fox is befriended **with a fish** — the whole flagship chain is gated on it |

All four were confirmed to pass end-to-end when a `fish` item is injected by hand, so F3 is the only
thing standing in their way — no other repair is needed once the fish exists.

### F3a — add a real fish species

Add to the `SPECIES` array (it needs no spawn weight; it is never ambient-spawned):

```js
 {key:"fish",name:"Fish",w:0,t:"A",layer:"pond",speed:0,kind:"fish",
  shape:shape({bw:12,bh:6,ears:false,tail:true,legs:false,tw:6,th:5}),
  pal:P("#6FA8DC","#3D6FA8","#DCE9F5"),
  voice:{freqs:[900,1200],gap:50},
  onClick:function(e){ /* caught mid-air */
    if(e.data.caught) return;
    e.data.caught=true; e.state="gone";
    WORLD.pickUp("fish", e.x, e.y);
    findEgg("pond-fish");
  }},
```

Add a `fish` branch in `WORLD.drawEntity` alongside `bird`/`flutter`/`glow`:

```js
  else if(sp.kind==="fish"){
    ctx.save(); ctx.translate(e.x, e.jy||gy);
    ctx.rotate((e.vy||0)*0.012);            // nose follows the arc
    ctx.fillStyle=pal.body; ctx.fillRect(-6*e.scale,-3*e.scale,12*e.scale,6*e.scale);
    ctx.fillStyle=pal.accent;               // tail fin
    ctx.fillRect(-10*e.scale,-3*e.scale,4*e.scale,6*e.scale);
    ctx.fillStyle=pal.belly; ctx.fillRect(-6*e.scale,0,12*e.scale,2*e.scale);
    ctx.fillStyle="#161616"; ctx.fillRect(3*e.scale,-1*e.scale,1*e.scale,1*e.scale);
    ctx.restore(); size={w:20,h:14};
  }
```

### F3b — give it a real jump arc

Replace `WORLD.spawnFish`:

```js
WORLD.spawnFish=function(pond){
  var waterY = H*0.865 + (my-.5)*-3.2;
  var f={ sp:SPECIES_BY_KEY.fish, layer:"pond", x:pond.x + (Math.random()*80-40),
          y:0, jy:waterY, vy:-330, waterY:waterY, vx:(Math.random()<.5?-1:1)*26,
          dir:1, scale:1.4, state:"jump", t:0, hb:{w:26,h:22}, data:{}, phase:0 };
  WORLD.entities.push(f); WORLD.flags.fishJumping=f;
  WORLD.splash(pond.x, waterY);
  return f;
};
```

Add the `jump` state to the entity update loop (near the other `state` branches):

```js
    if(e.state==="jump"){
      e.vy += 900*dt;                    // gravity
      e.jy += e.vy*dt;
      e.x  += e.vx*dt;
      if(e.jy >= e.waterY && e.vy>0){    // splashed back down
        WORLD.splash(e.x, e.waterY);
        e.state="gone";
        if(WORLD.flags.fishJumping===e) WORLD.flags.fishJumping=null;
      }
      return;                            // skip normal walk handling
    }
```

Add a small splash helper (reuse the existing particle burst):

```js
WORLD.splash=function(x,y){ blip(300,.18,"sine",.05);
  if(typeof burst==="function") burst(x,y,"#8FD3FF",12,6); };
```

### F3c — make it catchable and hook the chain

`WORLD.hit` already tests entities, so the arcing fish is clickable in flight. Confirm
`WORLD.pickUp("fish", …)` matches the existing carry API — if the signature differs, use whatever
`dropGroundItem`/`pickUp` already use elsewhere. Carrying the fish to a bear must still fire
`bear-fish`; the bear already lists `accepts:["fish","berry","honey"]`.

Also remove the now-dead `case "fishjump"` handling if any remains, and make sure the pond's
`reaction` still calls `findEgg("pond-fish")` **only** on a successful catch, not merely on the
click — the discovery should reward catching it.

**Verify:** click the pond → a blue fish arcs out of the water, rotates nose-down as it falls, and
splashes back in. Click it mid-air → it is caught and appears on the cursor. Give it to a bear →
`bear-fish` fires.

---

## F4 — Species all look identical — ✅ DONE

> **Already applied** (2026-08-11). `DEFSHAPE` gained silhouette features (`earShape`, `tailShape`,
> `snout`, `neck`, `hump`, `antlers`, `tusks`, `spikes`, `shell`, `mane`, `mask`), `drawQuad` draws
> them, and 16 species were given distinct profiles. Measured on shape-normalised masks with colour
> removed: mean pairwise IoU **0.522**, and **zero** pairs above 0.8 (three were before). Aspect
> ratios now span 0.58 (rabbit) to 1.98 (otter). Left here for reference; skip it.

Every four-legged animal is drawn by one generic `drawQuad` box template. Bear, fox, deer, wolf,
boar, badger, hedgehog and turtle are the same blob at different sizes and colours, which is why the
world doesn't read as charming.

**Do NOT convert to hand-authored pixel-sprite matrices.** The original plan asked for that, but 30
hand-drawn sprite grids is a large art task with a poor consistency floor. The procedural approach is
sounder — it just needs species character. Extend it instead.

### F4a — add feature flags to the shape system

Extend `DEFSHAPE` with optional silhouette features (all default off):

```js
var DEFSHAPE={ bw:16,bh:10,hw:8,hh:8,lw:3,lh:6,tw:8,th:6,ew:3,eh:5,
  ears:true, tail:true, legs:true,
  // NEW silhouette features
  earShape:"point",   // "point" | "round" | "long" | "none"
  tailShape:"stub",   // "stub" | "bushy" | "long" | "flat" | "none"
  snout:0,            // px of muzzle projection
  neck:0,             // px the head sits above the body
  hump:0,             // shoulder hump height
  antlers:false, tusks:false, spikes:false, shell:false, mane:false
};
```

### F4b — draw the features

In `drawQuad`, after the body and head are drawn, add a feature pass. Keep every feature a few
`fillRect` calls — the blocky style must stay consistent.

- **snout** — a smaller box projecting from the head in `dir`, at `pal.belly` or `pal.dark`.
- **neck** — offset `hy` upward by `neck*s` and draw a connecting box from body to head.
- **hump** — a rounded step of 2–3 stacked rects on the shoulder end of the body.
- **antlers** — 2 vertical rects from the skull with 2 short branches each (`pal.dark`).
- **tusks** — 2 small light rects angled forward from the snout.
- **spikes** — a row of 5–7 short rects along the top of the body (`pal.dark`).
- **shell** — a dome of 3 stacked rects, wider than the body, drawn *over* it.
- **mane** — a taller, darker block between head and body.
- **earShape** — `point`: narrow tall rects; `round`: square rects; `long`: tall thin (rabbit).
- **tailShape** — `bushy`: 3 stacked rects widening outward (fox); `long`: thin horizontal;
  `flat`: wide and short (beaver-ish); `stub`: current behaviour.

### F4c — apply per species

At minimum, differentiate these eight — they are the ones most visible:

| Species | Changes |
|---|---|
| Bear | `hump:4, earShape:"round", tail:false, snout:3, lw:4` |
| Deer | `neck:6, antlers:true, earShape:"long", tailShape:"stub", lw:2, lh:11` |
| Fox | `tailShape:"bushy", earShape:"point", snout:4, tw:10` |
| Wolf | `mane:true, earShape:"point", snout:4, tailShape:"long"` |
| Boar | `tusks:true, snout:5, hump:3, earShape:"round", lh:4` |
| Hedgehog | `spikes:true, earShape:"none", snout:3` |
| Turtle | `shell:true, legs:true, lh:3, earShape:"none", neck:3` |
| Rabbit | `earShape:"long", eh:9, tailShape:"stub", lh:5` |

Then sweep the remaining quadrupeds (squirrel: `tailShape:"bushy"`; mouse: `earShape:"round"`,
tiny; badger: `snout:4`; moose: `antlers:true, neck:8, size` large; raccoon: `earShape:"round"`,
masked face via `pal.belly` on the head).

**Verify:** screenshot the world with a bear, deer, fox and hedgehog on screen simultaneously. Each
must be identifiable in silhouette alone, with colour removed.

---

## F5 — Key item drops at the wrong ground height

**Location:** the `fetchQuest` completion in the `exit` state handler, currently line ~1501.

```js
dropGroundItem("key", cave.x-30, groundYAt(0,1));       // BEFORE — y sampled at x=0
```

The key is placed near the cave (~0.94·W) but its y comes from x=0, so it floats or sinks
regardless of F1.

```js
var kx = WORLD.props.filter(function(p){return p.key==="cave";})[0].x - 30;
dropGroundItem("key", kx, groundYAt(kx,1));             // AFTER
```

While here, audit **every** `dropGroundItem` and `groundYAt` call site for the same mistake — the y
argument must always be sampled at the same x the item is placed at, and at the item's own layer.

---

## F6 — The world has no far depth (MEDIUM)

30 species are distributed across only two depths: 3 at layer 1 and 18 at layer 2 (plus sky and
pond). Nothing uses layer 0, so the world looks flat.

Move these to `layer:0` (they read as distant, and `layerScale(0)` is 0.55):

- `moose`, `eagle` (already sky — leave), `crow` when perched, `wolf` (some spawns), `boar`.

Better: in `WORLD.spawn`, when a species declares `layerRange:[0,2]`, pick a random layer within it
so the same species appears at varying depths across spawns:

```js
var layer = sp.layerRange
  ? sp.layerRange[0] + Math.floor(Math.random()*(sp.layerRange[1]-sp.layerRange[0]+1))
  : sp.layer;
```

Give deer, fox, wolf, rabbit and squirrel `layerRange:[1,2]`, and moose/bear `layerRange:[0,1]`.
Confirm `groundYAt` handles layer 0 (it already maps layer 0 → `R1`, step 16, offMul −26).

**Verify:** several animals visible at once at visibly different sizes and depths, correctly
occluded — nearer animals drawn over farther ones.

---

## F7 — Nothing is tethered to the terrain under mouse parallax (CRITICAL)

**Do F1 first** (it is a one-character change and worth verifying on its own), then this. F7 replaces
`groundYAt` entirely, keeping F1's `floor` indexing.

### The bug

The terrain slides horizontally with the mouse — each ridge is drawn at
`i*step + (mx-0.5)*offMul - OVER` (line ~587). **No entity or prop applies that shift.** Verified:
zero call sites reference `mx` when positioning an entity's x.

Two visible symptoms, both immersion-breaking:

1. **Horizontal:** move the mouse and the hills slide, but animals and props hold a fixed screen x —
   so they visibly slide *across* the landscape instead of standing on it.
2. **Vertical:** `groundYAt` folds the parallax into the array *index*
   (`i = floor((x - off)/step)` where `off` contains `mx`), so as the mouse moves an entity
   re-samples a **different ridge column** and bobs up and down. It "swims" over the terrain.

### The fix — store world coordinates, transform at draw time

Treat every `e.x` and `p.x` as a **world** coordinate that is parallax-independent, and convert to
screen space only when drawing or hit-testing. This is the "physical tether": entities and terrain
then share one transform, so they cannot drift apart.

Replace `groundYAt` with:

```js
function paraMul(layer){ return layer===0?-26 : layer===1?-16 : layer==="sky"?-30 : -8; }
function stepFor(layer){ return layer===0?16 : layer===1?14 : 12; }
function paraX(layer){ return (mx-0.5)*paraMul(layer); }
function paraY(layer){ return (my-0.5)*paraMul(layer)*0.4; }

// wx is a WORLD x. No parallax in the index — that is what caused the bobbing.
function groundYWorld(wx,layer){
  var arr = layer===0?R1 : layer===1?R2 : R3;
  if(!arr) return H*0.86;
  var i = Math.floor((wx + OVER)/stepFor(layer));
  i = Math.max(0, Math.min(arr.length-1, i));
  return arr[i];
}

// Screen position of anything tethered to the ground.
function screenOf(wx,layer){
  return { x: wx + paraX(layer), y: groundYWorld(wx,layer) + paraY(layer) };
}
```

This is consistent with the terrain by construction: ridge segment `i` covers world x
`[i*step - OVER, (i+1)*step - OVER)` and is drawn at `worldX + (mx-0.5)*offMul`.

### Migration — every call site that must change

Search the file for `groundYAt(` and handle each:

| Call site | Change |
|---|---|
| `WORLD.drawEntity` (~1521) | `var s = screenOf(e.x, e.layer)` → draw at `s.x`, `s.y`. Do **not** use `e.x` directly for drawing any more. |
| `WORLD.hit` entities (~751) | Compute `screenOf(e.x, e.layer)` per entity and test the pointer against **that**, not `e.x`. |
| `WORLD.hit` items (~746) | Same — items are ground-tethered at layer 2. |
| `WORLD.hit` props (~758) | Same, using the prop's own layer. |
| `WORLD.drawProp` | Same transform as `drawEntity`. |
| `WORLD.drawItem` | Same. |
| `dropGroundItem(...)` | Store the **world** x it was dropped at; drop the pre-computed `gy` argument and let the draw call derive y via `screenOf`. This also removes the whole class of bug in F5. |
| Pond entities (`layer:"pond"`) | Use `paraX(2)` / `paraY(2)` so they stay in the water — the pond prop is layer 2. The existing `H*0.865 + (my-.5)*-3.2` already equals `paraY(2)`; keep the y, add the missing x shift. |
| Sky entities | Use `paraX("sky")` (−30, matching the clouds) and leave their own `y` alone. |
| `hoverGlow` | Draw at the transformed position, or it will highlight empty space. |

### Things that must NOT change

- Movement integration stays in world space: `e.x += e.vx*dt` is already correct and needs no shift.
- Spawn x (`-60` / `W+60`) and seeded prop positions become world coordinates. Parallax is at most
  ±26px, so no repositioning is needed.
- Despawn bounds should test **screen** x (`screenOf(e.x,e.layer).x`) so nothing lingers visible at
  the edge.
- Egg logic that compares positions (`nearPond`, campfire gathering, fetch targets) should keep
  comparing **world** x to world x. Do not mix spaces — that is the single most likely way to break
  a chain during this refactor.

### Verify

1. Park an animal mid-screen, then sweep the mouse fully left → right. The animal must stay locked
   to the same spot **on the hillside** — no sliding across the terrain, no vertical bobbing.
2. Do the same with a rock, a tree and the pond. Props must move with their layer.
3. Animals at layer 1 must shift about twice as far as layer 2 animals (−16 vs −8), matching their
   ridges.
4. Hover a small target (a rock) and sweep the mouse — the hover glow and the click hitbox must stay
   on the rock, not drift off it.
5. Re-run the tier-3 chains that depend on positions: `campfire-tales`, `the-long-con`,
   `water-flowers`, `truffle-pond`.

---

## F8 — Ambient spawning stops permanently after a few minutes (CRITICAL)

### Evidence

Instrumented run, sampling every 5s for 97s. Entity count climbed monotonically and never fell:

```
t=7s   n=1    immortal=0
t=47s  n=8    immortal=0
t=67s  n=9    immortal=2   (woodpecker perched, frog in pond)
t=97s  n=10   immortal=2   → heading to the cap of 12
```

### Cause

The spawner (line ~1450) is gated on `this.entities.length < 12`, but the despawn rule
(line ~1506) exempts three categories **permanently**:

```js
if(e.layer!=="pond" && !sp.perch && e.state!=="sleep" && (e.x<-140||e.x>W+140)) e.state="gone";
```

- `layer==="pond"` — frog, duck, otter, dragonfly (and the broken fish) never leave the pond.
- `sp.perch` — owl, crow, woodpecker perch and never fly away.
- `state==="sleep"` — a fox you clicked curls up and sleeps **forever**.

These accumulate one by one until they occupy all 12 slots, and then **no animal ever spawns
again**. The world dies quietly a few minutes in. This is exactly the reported symptom.

### Fix

**F8a — give residents a lifespan.** Nothing should be immortal.

```js
// on spawn, for perch/pond species:
e.residentUntil = WORLD.t + 25 + Math.random()*45;      // 25–70s
// in update, before the out-of-bounds check:
if(e.residentUntil && WORLD.t > e.residentUntil && e.state!=="react"){
  e.state="exit";                                        // fly/swim/walk off naturally
  e.data.exitTarget = (e.x < W/2) ? -160 : W+160;
  e.vx = Math.abs(e.vx||20) * (e.x < W/2 ? -1 : 1) * 1.4;
}
```

Sleeping animals must wake:

```js
if(e.state==="sleep" && WORLD.t - (e.sleepStart||0) > 30 + Math.random()*30){
  e.state="walk"; e.fx=null;                             // stretch, then wander off
}
```

Set `e.sleepStart = WORLD.t` wherever `state="sleep"` is assigned.

**F8b — cap only ambient wanderers.** Residents should not starve the spawner:

```js
var ambient = this.entities.filter(function(e){ return !e.residentUntil && e.state!=="sleep"; }).length;
if(this.spawnTimer<=0 && ambient < 9 && this.entities.length < 16){ ... }
```

**F8c — guarantee liveliness.** Add a watchdog so the world can never go quiet:

```js
this.sinceLastSpawn = (this.sinceLastSpawn||0) + dt;
if(this.sinceLastSpawn > 20){          // nothing has spawned in 20s
  var oldest = this.entities.filter(function(e){return e.residentUntil;})
                 .sort(function(a,b){return a.residentUntil-b.residentUntil;})[0];
  if(oldest) oldest.residentUntil = 0;  // evict the longest-staying resident
  this.spawnTimer = 0;                  // and spawn immediately
}
```
Reset `this.sinceLastSpawn = 0` inside the spawn branch.

**F8d — rebalance the mix.** Over 97s the observed spawns were dominated by bee, butterfly,
hedgehog, seagull and squirrel; no deer, bear or fox appeared at all. The tiny insects are visually
negligible and crowd out the animals people actually want to see. Halve the weights of `bee`,
`butterfly`, `firefly`, `mouse` and `snail`, and raise `deer`, `fox`, `bear`, `wolf`, `rabbit` and
`owl`. Aim for a charismatic animal on screen most of the time.

**Verify:** leave the page open for 5 minutes without interacting. Animals must still be arriving
and leaving at the end, entity count should oscillate rather than climb, and at least one large
animal (deer/fox/bear) should have appeared. Click a fox and confirm it eventually wakes and leaves.

---

## F9 — The pond is an ellipse pasted onto pixel art (HIGH)

### The problem

```js
ctx.ellipse(p.x, gy, p.rw, p.h, 0, 0, 6.283);   // line ~1573
```

A smooth anti-aliased 260px ellipse, in a world drawn entirely from `fillRect` blocks. It is also
positioned at the ground height of **one** x sample, so on sloped terrain it floats above the
ground on one side and sinks into the hill on the other. Pond creatures inherit the same single
`H*0.865` constant, so they do not sit on the water surface either.

### The fix — water fills a valley, it does not sit on a hill

Water should be a **level**, with terrain below that level submerged. That is both physically
right and naturally blocky.

**F9a — carve a basin.** Right after `R3` is generated, carve a guaranteed valley so the pond always
has somewhere to sit:

```js
function carveBasin(arr, step, centerFrac, widthCols, depth){
  var c = Math.floor(arr.length*centerFrac);
  for(var i=c-widthCols; i<=c+widthCols; i++){
    if(i<0||i>=arr.length) continue;
    var d = Math.cos((i-c)/widthCols * Math.PI/2);        // smooth bowl, 0 at rim
    arr[i] += Math.round(depth*d*d/step)*step;            // keep it quantised to the grid
  }
  return { c:c, half:widthCols };
}
var basin = carveBasin(R3, 12, 0.30, 11, 46);
```

**F9b — derive the water level from the rim.** The level is the terrain height at the basin edge:

```js
WATER_LEVEL = R3[basin.c - basin.half];       // world y, no parallax
```

**F9c — draw water as columns, not an ellipse.** For every ridge column whose terrain is below the
water level, fill from the level down to the terrain:

```js
case "pond": {
  var stepW = 12, lvl = WATER_LEVEL + paraY(2), offX = paraX(2) - OVER;
  for(var i=0;i<R3.length;i++){
    var ty = R3[i] + paraY(2);
    if(ty <= lvl) continue;                              // land, not water
    var x = i*stepW + offX;
    ctx.fillStyle = "rgba(58,124,176,.78)";
    ctx.fillRect(Math.round(x), Math.round(lvl), stepW+1, Math.round(ty-lvl));
    // animated surface shimmer on the top row only
    ctx.fillStyle = "rgba(150,215,245,"+(0.5+0.3*Math.sin(WORLD.t*2 + i*0.6))+")";
    ctx.fillRect(Math.round(x), Math.round(lvl), stepW+1, 3);
  }
  break; }
```

Ripples become expanding **horizontal bars** along the surface rather than concentric ellipses:
draw 2–3 thin light rects spreading left and right from the click x, fading with `p.ripple`.

Lily pads sit on the surface: `y = lvl - 2`, x anywhere in the submerged span.

**F9d — pond creatures float on the surface.** Replace the hard-coded `H*0.865` for `layer:"pond"`
everywhere (draw, hit-test, spawn) with the real surface:

```js
function waterSurfaceY(){ return WATER_LEVEL + paraY(2); }
function waterSpan(){    // world-x range of submerged columns
  var a=null,b=null;
  for(var i=0;i<R3.length;i++){ if(R3[i]>WATER_LEVEL){ if(a===null)a=i; b=i; } }
  return { x0:(a*12)-OVER, x1:(b*12)-OVER };
}
```

Spawn pond creatures with a world x inside `waterSpan()`, clamp their wandering to it, and draw them
at `waterSurfaceY()` (ducks/otters float on it; frogs sit on lily pads slightly above it; the F3 fish
launches from it).

**Verify:** the pond fills a visible dip in the terrain with a flat top and blocky edges that follow
the hillside — no ellipse outline, nothing floating over a slope. Sweeping the mouse moves the water
with its terrain layer. Ducks sit on the surface, not above or below it. The F3 fish launches from
the surface and splashes back into it.

---

## F10 — Three easter eggs are impossible to complete (CRITICAL)

Found by auditing every item id against its producers. `WORLD.carry` is **only ever** set by
`WORLD.pickUp`, which is only ever reached by clicking a ground item created by `dropGroundItem`.
There is no `gives` mechanism anywhere (0 occurrences in the file).

Cross-referencing items *consumed* by a chain against items *produced*:

| Item | Consumed at | Produced | Egg blocked |
|---|---|---|---|
| `flower` | `1174` deer accepts it | **never** | `flower-deer` (Flower Power) |
| `firefly` | `1340` cave accepts it | **never** | `firefly-cave` (Night Light) |
| `water` | `1316` flowers accept it | **never** | `water-flowers` (Watering Can) |
| `fish` | bear / otter / fox | **never** | fixed by **F3** — no extra work |

All other items check out: `acorn` (squirrel drops it), `berry`, `truffle`, `honey`, `feather`
(seagull drops it), `coin`, `key`, `pinecone`, `snowball`, `shell` all have producers.

### Fix — add the three missing sources

**Flower** — the flower patch blooms but yields nothing. Give it a pickable flower once bloomed:
```js
// in the flowers prop reaction, after p.bloom = true
if(p.bloom && !p.picked){ p.picked = true; dropGroundItem("flower", p.x + 14, groundYAt(p.x,2)); }
```

**Firefly** — clicking a firefly should let you carry it (it is a `kind:"glow"` entity, so it has no
ground item). In the firefly `onClick`, after the sync check:
```js
if(!WORLD.carry){ e.state="gone"; WORLD.pickUp("firefly", e.x, e.y); }
```

**Water** — there is no water source at all. Add one to the pond's reaction (it currently only
spawns the fish):
```js
// pond reaction, when the player is empty-handed
if(!WORLD.carry) WORLD.pickUp("water", p.x, groundYAt(p.x,2));
```
Note line `1284` contains a dead `if(WORLD.carry==="water"){ }` empty block — remove it.

**Verify:** all three chains complete end to end — pick a flower → give to deer; catch a firefly →
carry into the cave; carry water → water all three flower patches. Journal count must be able to
reach **54/54**.

---

## F11 — The cat can never appear

`{key:"cat", w:0, …}` with `gate:function(){ return WORLD.t - WORLD.lastInteractAt > 60; }`. The
intent is right — it should wander in after 60s of no interaction — but the spawner picks species by
**weighted** random:

```js
var r = Math.random()*totalW;
for(var i=0;i<pool.length;i++){ r -= pool[i].w; if(r<=0){ sp=pool[i]; break; } }
```

With `w:0` the cat contributes nothing to `totalW` and `r -= 0` never satisfies the break, so it can
never be selected. It is not spawned explicitly anywhere either (`SPECIES_BY_KEY.cat` appears
nowhere). The cat, its purr reaction, and its contribution to the **Gentle Hand** achievement
("pet 5 different animals") are all dead code.

**Fix** — spawn gated species directly rather than through the weighted pool:

```js
// before the weighted pick, in the spawn branch
var special = SPECIES.filter(function(sp){ return sp.w===0 && sp.gate && sp.gate(); });
if(special.length && !this.entities.some(function(e){ return e.sp===special[0]; })){
  this.spawn(special[0], {});
  this.sinceLastSpawn = 0;
  return;                                  // this tick belongs to the special spawn
}
```

**Verify:** leave the page completely untouched for ~70s — a cat walks in. Clicking it purrs and
counts toward Gentle Hand.

---

## F12 — Clicking a selected content tile should deselect it (QoL)

In the shelf, `b.onclick` calls `select(it.name)` unconditionally, so clicking the already-selected
tile just re-selects it. Clicking it again should clear the selection — closing the detail panel,
removing the dependency curves, and un-dimming the grid. Currently the only ways out are the panel's
× or Escape.

**Location:** the slot click handler, currently line ~866.

```js
b.onclick=function(){
  if(Date.now()-justDragged<250) return;
  if(selected === it.name){ blip(420,.06); clearFocus(); return; }   // toggle off
  blip(700,.08); achieve("🔍","Curious"); select(it.name);
};
```

`clearFocus()` already resets `selected`, the focus dimming, the SVG curves and the panel, so no
other change is needed. Use a lower blip pitch for deselect so the two actions sound different.

**Verify:** click a tile (panel opens, curves draw) → click the same tile again (panel closes, curves
clear, grid un-dims). Escape and the × still work.

---

## F13 — `snail-lily` can only fire by luck

`snail-lily` triggers when a snail is clicked **while near the pond** (`WORLD.nearPond(e.x)`), but
snails spawn at a random screen edge and move at `speed:3` — the slowest in the world. A snail that
spawns on the far side may never reach the pond before it despawns, and the player has no way to
influence it. Verified: clicking a snail at the default spawn x does nothing; clicking one spawned at
the pond's x fires immediately.

Also `lily` is a stub prop — `{key:"lily", x:0, w:1, h:1, reaction:function(){}}` — so the lily pads
the egg is named after do not exist as objects.

**Fix:** either spawn snails preferentially near the pond, or let the player carry one:
```js
// snail onClick, when not near the pond
if(!WORLD.carry){ e.state="gone"; WORLD.pickUp("snail", e.x, e.y); }
// then accept a carried snail at the lily prop, and fire the egg there
```
Carrying is the better option — it matches the egg's name and makes it a deliberate two-step chain
rather than a coincidence. Give `lily` a real position on the pond surface (see F9) and a hitbox.

---

## Verification record — all 54 eggs driven in the browser (2026-08-11)

Every egg was exercised through the same code path a real click takes (`WORLD.interact` for entities
and items, `prop.reaction` for props, real DOM events for the weather and time controls), on an
instrumented copy of the shipped file.

**Result: 47 of 54 reachable. 7 blocked, all by two already-documented causes.**

| Tier | Reachable | Blocked |
|---|---|---|
| 1 — single click (26) | **26** | 0 |
| 2 — two step (14) | 11 | 3 |
| 3 — multi step (14) | 10 | 4 |

**The 7 blocked:**

| Egg | Cause | Fixed by |
|---|---|---|
| `flower-deer` | no `flower` producer | F10 |
| `firefly-cave` | no `firefly` item producer | F10 |
| `water-flowers` | no `water` producer | F10 |
| `fish-otter` | no `fish` producer | F3 |
| `bear-fish` | no `fish` producer | F3 |
| `bear-feast` | needs fish + berry + honey | F3 |
| `the-long-con` | fox is befriended with a fish | F3 |

Every one of the 7 was confirmed to complete end-to-end once the missing item is supplied by hand,
so **F3 + F10 together unblock all of them** and no further chain repair is needed.

**Notable passes** (verified, not assumed): `moonlit-rave` (7 distinct fireflies → swarm → click),
`constellation` (5 stars in brightness order), `snowman` (3 piles in size order), `fairy-ring`
(7 mushrooms inside the 6s window), `campfire-tales` (fox/rabbit/deer gather on real timers over
~5s, each clicked), `wolf-pack`, `migration`, `rainbow` (weather cycled via the real button),
`acorn-hunt` (3 acorns **dropped** — not given to a squirrel — then a full time-slider sweep),
`full-day`, and `the-long-con` (boulder → befriend → fetch → key → cave) with an injected fish.

**Testing notes for whoever re-runs this:**
- `document.hidden` is true whenever the preview pane is not displayed, which **stops
  `WORLD.update` entirely** — `WORLD.t` never advances and entity `uid`s are never assigned. Drive
  frames manually with `for(i…) WORLD.update(1/60)` or the results are meaningless.
- Prop state (`clicks`, `rolled`, `lit`, `bloom`) persists across a `found`/`flags` reset. Reload the
  page between tests that mutate props, or you will get false failures.
- `campfire-tales` uses real `setTimeout`s (~4.8s), so it cannot be fast-forwarded by manual ticking.
- `acorn-hunt` counts acorns **dropped on the ground** at 3 spots ≥60px apart, not acorns given to a
  squirrel — giving one to a squirrel fires `acorn-squirrel` and returns early.
- `full-day` resets its range on any click outside the slider, so sweep it without touching anything
  else.

---

## A1 — Background music (ADDITION, not a fix)

This is new functionality, not a defect. Do it last, after F1–F9.

Everything is **synthesized from note data via WebAudio** — no audio files, no network, the page
stays one self-contained file. Reuse the existing `AudioContext` that `blip()` creates.

### Licensing

Every **composition** in `music-tracks.js` is public domain — the most recent composer death is
Satie in 1925, which clears life+70 everywhere. That part is settled and needs no further thought.

What differs is the **source file** each track was extracted from, and it is worth being precise
because the two questions are independent:

| Source | Tracks | Status |
|---|---|---|
| OpenScore, CC0 | Moonlight mvt I and II | Explicit public-domain dedication. Cleanest. |
| Declares `rights: Public Domain` | Mountain King | Stated by the file itself. |
| IMSLP scan → our own OMR | the three Gymnopédies | We derived the notes ourselves from a PD score. |
| MuseScore upload, no rights statement | the remaining six | See below. |

**Resolved — no open question, ship all 13.** The last group was collected from a GitHub library
that publishes MusicXML explicitly as a public-domain collection, and the project owner has accepted
that provenance. Nothing here needs further review.

Recorded for the future, because the reasoning is worth keeping if a track is ever swapped: composition
status and *source-file* status are independent questions. A community upload can be a
**transcription** of a public-domain original (Für Elise and Clair de Lune were written for solo
piano, so the uploader added no creative content) or a genuine **arrangement** — an orchestral work
reduced for piano, or an "easy piano" rewrite — where the arranger's choices can attract copyright of
their own. Symphony No. 5, Flight of the Bumblebee, Sugar Plum Fairy, Canon in D and Greensleeves
fall in the second category, which is why their provenance was worth confirming rather than assuming.
Do the same check on anything added later, and prefer a source that states its licence.

Separately: a modern **engraving** may carry thin typographic rights as a graphic work. We never ship
the engraving, only note data, so that does not reach us. Do not paste in or trace a published
edition.

### A1a — Engine

> **Use the engine in `music-preview.html`, not the sketch below.** That file has a working,
> browser-verified player for exactly this data, and the sketch that follows is kept only for its
> comments — it does **not** fit the shipped track format. Two concrete reasons:
>
> - It advances on a **fixed 16th-note grid** (`this.pos += 0.25`) and looks events up by
>   `v.at[beat.toFixed(2)]`. Real scores are not on a 16th grid: Moonlight's arpeggios are triplets
>   (`0.3333` of a beat) and Clair de Lune has `0.75` and `4.5` durations. Every one of those falls
>   between grid slots and would be silently dropped.
> - It expects a keyed `at` map; `music-tracks.js` ships **sequential `[pitch, beats]` arrays**.
>
> The preview engine instead gives each voice its own cursor and walks its `seq`, so any duration
> works and voices cannot drift apart. Lift `freq()`, `env()`, `decay()`, `rolloff()`, the `INSTR`
> bank, `blip()` and `pump()` from it.
>
> Verified in-browser: opening frequencies match the score exactly (Mountain King's F♯3+F♯4 dyad at
> 185/370 Hz, then the theme at 61.7/69.3/73.4/82.4/92.5 Hz interleaved against the left hand's
> 30.9/46.2 Hz pedal); chords schedule as simultaneous oscillators; the loop path wraps seamlessly;
> loop-off ends and cleans up; all 7 instruments survive a sweep of 30.9 Hz–3951 Hz against
> 0.089 s–4 s durations without throwing.

#### Instruments and register

`blip()` dispatches to an `INSTR` bank of **seven synth techniques**, not seven oscillator shapes —
subtractive (chiptune, strings), additive (music box, organ via `PeriodicWave`), FM (electric piano
at 2:1, bells at an inharmonic 1.41), and physical modelling (plucked string, Karplus–Strong). Each
track's `instrument` field names its default, chosen to suit the piece: Sugar Plum opens on the music
box because the original is a celesta, Mountain King and Greensleeves on plucked string (pizzicato,
lute), Symphony No. 5 on strings.

**Keep the register handling when you port this.** It is not decoration — it fixes a real complaint.
Sugar Plum reaches B7 (3951 Hz) with 27.5% of its notes above C6, and equal gain across the range is
not equal loudness because the ear peaks at 2–5 kHz, so those notes came out piercing. Two mechanisms,
both on by default:

- `rolloff(f)` — per-note gain reduction above 600 Hz: −3.4 dB at C6, −7.6 dB at C7, −10.5 dB at B7,
  floored at 0.3 so the top stays present.
- a master **high shelf** of −7 dB from 2.6 kHz. A shelf, not a lowpass — it pulls down the sensitive
  band without dulling everything below it.

Note that the tuning itself was never wrong (A4 = 440, middle C = C4 = 261.6 Hz); do not "fix" it by
shifting the octave mapping. A transpose control (−2…+1 octaves) covers the case where a piece still
sits too high — at −1 octave Sugar Plum's top note lands at 1975 Hz, clear of the band.

Karplus–Strong allocates an `AudioBuffer` per note, so it is cached by frequency and ring length with
a 500-entry cap. Worst measured load is the densest track on the most expensive instrument
(Bumblebee on music box, 3 oscillators per note) at **36 oscillators/second** — comfortable.

```js
var MUSIC = {
  ctx:null, master:null, timer:null, track:null, nextT:0, pos:0, playing:false, vol:0.35,
  NOTE:{C:0,D:2,E:4,F:5,G:7,A:9,B:11},
  freq:function(n){                                   // "C#4" | "Bb3" | "R" (rest)
    if(n==="R") return 0;
    var m=/^([A-G])([#b]?)(-?\d)$/.exec(n); if(!m) return 0;
    var s=this.NOTE[m[1]] + (m[2]==="#"?1:m[2]==="b"?-1:0) + (+m[3]+1)*12;
    return 440*Math.pow(2,(s-69)/12);
  },
  start:function(id){
    this.ctx = this.ctx || new (window.AudioContext||window.webkitAudioContext)();
    if(!this.master){ this.master=this.ctx.createGain();
      this.master.gain.value=this.vol; this.master.connect(this.ctx.destination); }
    this.track=TRACKS.filter(function(t){return t.id===id;})[0];
    this.pos=0; this.nextT=this.ctx.currentTime+0.1; this.playing=true;
    clearInterval(this.timer);
    this.timer=setInterval(this.tick.bind(this),25);      // lookahead scheduler
  },
  stop:function(){ this.playing=false; clearInterval(this.timer); this.timer=null; },
  tick:function(){
    if(!this.playing||!this.track) return;
    var spb = 60/this.track.bpm;
    while(this.nextT < this.ctx.currentTime + 0.15){      // schedule 150ms ahead
      var beat=this.pos, t=this.track;
      t.voices.forEach(function(v){ this.playVoiceAt(v,beat,this.nextT,spb); },this);
      this.pos += 0.25;                                    // 16th-note grid
      this.nextT += spb*0.25;
      if(this.pos >= t.bars*t.beatsPerBar) this.pos = 0;    // seamless loop
    }
  },
  playVoiceAt:function(v,beat,when,spb){
    var ev=v.at[beat.toFixed(2)]; if(!ev) return;
    var self=this;
    (Array.isArray(ev.n)?ev.n:[ev.n]).forEach(function(n){
      if(n==="R") return;
      var o=self.ctx.createOscillator(), g=self.ctx.createGain();
      o.type=v.wave||"triangle"; o.frequency.value=self.freq(n);
      var peak=(v.gain||0.18), dur=ev.d*spb;
      g.gain.setValueAtTime(0.0001,when);
      g.gain.exponentialRampToValueAtTime(peak, when+(v.attack||0.02));
      g.gain.exponentialRampToValueAtTime(0.0001, when+Math.max(dur,0.08));
      o.connect(g); g.connect(self.master); o.start(when); o.stop(when+dur+0.05);
    });
  }
};
```

**Track authoring format.** Each voice is a sequential line; a build step converts it to the
beat-indexed `at` map the scheduler reads. `[pitch, durationInBeats]`, arrays for chords, `"R"` for
a rest:

```js
function buildVoice(v){                       // seq -> beat-indexed lookup
  var at={}, b=0;
  v.seq.forEach(function(e){ at[b.toFixed(2)]={n:e[0], d:e[1]}; b+=e[1]; });
  v.at=at; v.beats=b; return v;
}
```

### A1b — Loop rules (apply to every track)

These are background loops that may run for an hour. Seam quality matters more than fidelity.

Rules 1–2 are now **enforced by the generator and checked on load** — `music-tracks.js` throws if any
voice stops summing to the track's `beats`. They are kept here because they explain *why* that check
exists, and they still apply to any track added by hand.

1. **Every voice must sum to exactly `beats`.** (Not `bars × beatsPerBar` — pickups and irregular
   bars break that identity. Für Elise opens with a half-bar pickup; Symphony No. 5 bar 269 is a
   genuine 10-beat cadenza in a 2/4 movement.) If voices differ by even a fraction of a beat they
   drift apart a little more each loop, and the track slowly falls to pieces. `mxl2track.py` forces
   each voice to the exact total; the left hand of Mountain King summed to 351.9975 against 352
   before that correction went in.
2. **Do not re-round durations.** Triplets ship as `0.3333` and the generator absorbs the residue
   into the final event so the voice total stays exact. Rounding them again reintroduces the drift.
3. **Close the harmony into the loop point.** The last chord should be the same as the first, or a
   dominant/subdominant that resolves to it.
4. **Keep the melodic seam small.** The last melody note should be within a step or a comfortable
   leap of the first, or the phrase should end with a rest.
5. **Let tails ring across the seam.** Do not stop oscillators at the loop boundary; overlapping
   releases are what make the join inaudible.

Rules 3–4 were written for hand-built 8-bar loops. The shipped tracks are **complete pieces** that
repeat from the top the way the music does, so their seam is the composer's own ending-to-opening
join — leave it alone rather than editing note data to smooth it.

### A1c — Track data

**Use `music-tracks.js` as-is. Do not hand-enter, edit, or "improve" any track in it.** Nothing in
it is worked from memory — every one of the 13 tracks is generated by `mxl2track.py` from real
MusicXML, and they are complete pieces rather than loop edits.

```html
<script src="music-tracks.js"></script>   <!-- window.MUSIC_TRACKS -->
```

| id | Piece | Bars | Length | Mood |
|---|---|---|---|---|
| `sugar-plum` | Tchaikovsky, Dance of the Sugar Plum Fairy (♩=70) | 53 | 1:31 | playful |
| `moonlight-allegretto` | Beethoven, Moonlight Sonata mvt II (♩=168) | 60 | 1:04 | playful |
| `bumblebee` | Rimsky-Korsakov, Flight of the Bumblebee (♩=144) | 101 | 1:24 | exciting |
| `mountain-king` | Grieg, In the Hall of the Mountain King (♩=138) | 88 | 2:33 | exciting |
| `fate` | Beethoven, Symphony No. 5 mvt I (♩=164) | 508 | 6:12 | dramatic |
| `fur-elise` | Beethoven, Für Elise (♩=72) | 104 | 2:10 | moody |
| `moonlight` | Beethoven, Moonlight Sonata mvt I (♩=60) | 69 | 4:36 | moody |
| `greensleeves` | Traditional, Greensleeves (♩=120) | 33 | 0:50 | moody |
| `canon-in-d` | Pachelbel, Canon in D (♩=100) | 102 | 4:05 | calm |
| `clair-de-lune` | Debussy, Clair de Lune (♩=66) | 72 | 4:55 | calm |
| `gymnopedie-1` | Satie, Gymnopédie No. 1 (♩=76) | 78 | 3:05 | calm |
| `gymnopedie-2` | Satie, Gymnopédie No. 2 | 65 | 2:34 | calm |
| `gymnopedie-3` | Satie, Gymnopédie No. 3 | 60 | 2:22 | calm |

**37 minutes across 5 moods**, 10,875 note events. Each entry carries `{id, name, composer, year,
marking, mood, bpm, beatsPerBar, bars, beats, source, instrument, voices:[{name, wave, gain, seq}]}`
— the engine's format, no conversion needed. `beats` is the authoritative length; the file throws on load
if any voice stops summing to it. `bpm` is quarter-notes per minute and `seq` durations are quarter
notes, so seconds-per-beat is just `60 / bpm`.

Two tracks carry a `tempoNote` recording that their source file's tempo mark was overridden: Clair de
Lune's marked 48 gives a dotted-quarter pulse of 32, slower than any recording, and the Moonlight
Allegretto's marked 210 is likewise outside the performed range. Every other track uses its file's
own first tempo mark unchanged.

**Audition them** by serving this directory and opening `music-preview.html` — a card per track,
click to play, per-voice mute, loop toggle. There is a `prototypes` entry in `.claude/launch.json`
that serves this folder on port 5180.

**Adding more is cheap.** Find a MusicXML for the piece (MuseScore, IMSLP) rather than decoding a
scan, then run `python mxl2track.py <file.mxl> <bars> <beatsPerBar>`. Things that piece of
tooling handles, learned the hard way — each of these produces plausible-sounding but wrong output
if unhandled, which is why none of them can be skipped:

- **Chords can span staves.** Satie's accompaniment is B3 on the bass staff plus D4/F♯4 on the
  treble, all one voice — grouping chord notes by `(staff, voice)` splits the chord and attaches the
  upper notes to the wrong event.
- **Voice numbers are not stable.** Audiveris gives the Gymnopédie accompaniment voice 1 for bars
  1–4, then voice 2 once the melody enters. Streams are assigned by content, not voice number.
- **Two split strategies.** `classify()` splits melody/chord/bass by pitch — right for Satie, wrong
  for arrangements like Mountain King whose theme *starts* in octave 1–2 and climbs, which a fixed
  octave threshold files entirely as bass. `classify_by_staff()` splits right/left hand and is the
  right choice for two-staff piano writing.
- **Overlaps and rounding both cause drift.** Overlapping notes in one stream must be clipped at the
  next onset, and per-event rounding is corrected so each voice sums to the total exactly.
- **The first tempo mark wins, and `<per-minute>` is not quarter-notes.** A multi-movement file
  carries a tempo per movement, so keeping the last one made the Moonlight file report 155 — its
  third-movement Presto — for a piece that opens at 60. And `<per-minute>` counts *beat-units*:
  read raw, a 3/8 piece marked ♪=144 plays at twice speed. `sound/@tempo` is always quarter-notes
  per minute and is preferred; the metronome fallback converts via `beat-unit`.
- **1st/2nd endings must be skipped, repeats must not be taken.** Playing a volta pair back to back
  puts an audible stutter exactly where the music should flow on. One pass through, skipping
  non-final endings, is what a background loop wants.
- **A measure advances by how far it reached, not where its last voice stopped.** A measure whose
  final voice ends early otherwise drags every later bar forward and desyncs bar numbering.
- **`clamp_bars` is for OMR output only.** Audiveris writes an accompaniment chord as a *sequential*
  note after the melody in the same voice, so Gymnopédie No. 2 bar 59 reads 2+2 beats in a 3/4 bar
  and the piece gains a beat. Clamping to the notated length fixes it. Leave it **off** for engraved
  sources, where a long bar is usually real — Symphony No. 5 bar 269 is a genuine 10-beat cadenza
  that clamping would truncate.

Optional: Mountain King is exported at a constant 138 as written. If you want the accelerando, ramp
`bpm` per loop rather than editing the note data.

---

> ## ⛔ SUPERSEDED — everything from here to "A1d — UI and behaviour" is dead
>
> The `seq` arrays below (and the "Original tracks" section that follows them) were written from
> memory before any score was available. **Do not enter, copy, or merge any of them.** Every piece
> they sketch now exists in `music-tracks.js` as a complete, score-accurate version, and the ids
> collide — a hand-entered `canon`/`mountain-king`/`fur-elise` would shadow the real one.
>
> They are kept only so the reasoning behind the loop rules stays readable. Skip to
> **A1d — UI and behaviour**.

**2. `canon` — Canon in D — peaceful — bpm 64, 4/4, 4 bars (16 beats)**

Pachelbel's ground bass is *designed* to repeat forever — it is the best natural loop in the set.
Melody is one standard division, ending C#5 → F#5 (a rising fourth, idiomatic and smooth).

```js
bass: [["D3",2],["A2",2],["B2",2],["F#2",2],["G2",2],["D2",2],["G2",2],["A2",2]]   // 16
mel : [["F#5",2],["E5",2],["D5",2],["C#5",2],["B4",2],["A4",2],["B4",2],["C#5",2]] // 16
```

**3. `moonlight` — Moonlight Sonata — moody — bpm 54, 4/4, 4 bars (16 beats)**

The triplet ostinato *is* the mood; the melody is omitted deliberately (it is where my memory is
weakest, and the figure alone is more usable as background). Loop-edit: the progression is closed as
i–i–iv–V so it resolves back to the top.

```js
// arp: 12 triplet-eighths per bar, duration 4/12 each — use the fraction
arp : [ ...×4 ["G#3","C#4","E4"],   ...×4 ["G#3","C#4","E4"],
        ...×4 ["A3","C#4","F#4"],   ...×4 ["B#3","D#4","F#4"] ]              // 48 notes = 16
bass: [[["C#2","C#3"],4], [["C#2","C#3"],4], [["F#1","F#2"],4], [["G#1","G#2"],4]]  // 16
```

**4. `greensleeves` — Greensleeves — wistful — bpm 100, 3/4, 8 bars (24 beats)**

Traditional, genuinely anonymous. Ends with a rest bar so the pickup restarts cleanly.

```js
mel : [["R",2],["A4",1],
       ["C5",1.5],["D5",.5],["E5",1],
       ["F5",1],["E5",1],["D5",1],
       ["B4",1.5],["G4",.5],["A4",1],
       ["B4",1],["C5",1],["A4",1],
       ["A4",1.5],["G#4",.5],["A4",1],
       ["B4",1],["G#4",1],["E4",1],
       ["R",3]]                                                            // 24
bass: [["A2",3],["A2",3],["G2",3],["G2",3],["A2",3],["E2",3],["A2",3],["A2",3]]  // 24
```

**5. `ode-to-joy` — Ode to Joy — bright — bpm 120, 4/4, 8 bars (32 beats)**

Already a closed period: ends on the tonic C, restarts on E. Nothing to edit.

```js
mel : [["E4",1],["E4",1],["F4",1],["G4",1], ["G4",1],["F4",1],["E4",1],["D4",1],
       ["C4",1],["C4",1],["D4",1],["E4",1], ["E4",1.5],["D4",.5],["D4",2],
       ["E4",1],["E4",1],["F4",1],["G4",1], ["G4",1],["F4",1],["E4",1],["D4",1],
       ["C4",1],["C4",1],["D4",1],["E4",1], ["D4",1.5],["C4",.5],["C4",2]]   // 32
bass: [["C2",4],["G2",4],["C2",4],["G2",4],["C2",4],["F2",4],["G2",4],["C2",4]]  // 32
```

**6. `minuet` — Minuet in G — playful — bpm 132, 3/4, 8 bars (24 beats)**

Loop-edit: the 8th bar is rewritten to land on **D5**, the same note the phrase opens on — a
seamless join.

```js
mel : [["D5",1],["G4",.5],["A4",.5],["B4",.5],["C5",.5],
       ["D5",1],["G4",1],["G4",1],
       ["E5",1],["C5",.5],["D5",.5],["E5",.5],["F#5",.5],
       ["G5",1],["G4",1],["G4",1],
       ["C5",1],["D5",.5],["C5",.5],["B4",.5],["A4",.5],
       ["B4",1],["C5",.5],["B4",.5],["A4",.5],["G4",.5],
       ["F#4",1],["G4",.5],["A4",.5],["B4",.5],["G4",.5],
       ["A4",2],["D5",1]]                                                   // 24
bass: [["G2",1],["B2",1],["D3",1], ["G2",1],["B2",1],["D3",1],
       ["C3",1],["E3",1],["G3",1], ["G2",1],["B2",1],["D3",1],
       ["A2",1],["C3",1],["E3",1], ["G2",1],["B2",1],["D3",1],
       ["D2",1],["F#2",1],["A2",1], ["G2",1],["B2",1],["D3",1]]             // 24
```

**7. `mountain-king` — In the Hall of the Mountain King — exciting — bpm 100→190, 4/4, 4 bars**

Loop-edit: the phrase already ends on **B3**, the note it starts on — a perfect seam. Increase `bpm`
by 6% each loop, reset to 100 above 190. The accelerando is the whole point.

```js
mel : [["B3",.5],["C#4",.5],["D4",.5],["E4",.5],["F#4",.5],["D4",.5],["F#4",1],
       ["F4",.5],["E4",.5],["C#4",.5],["E4",.5],["D4",.5],["B3",.5],["D4",1],
       ["B3",.5],["C#4",.5],["D4",.5],["E4",.5],["F#4",.5],["D4",.5],["F#4",1],
       ["A4",.5],["G4",.5],["F#4",.5],["E4",.5],["D4",1],["B3",1]]          // 16
bass: [["B1",1],["R",1],["B1",1],["R",1], ["F#1",1],["R",1],["F#1",1],["R",1],
       ["B1",1],["R",1],["B1",1],["R",1], ["F#1",1],["R",1],["B1",1],["R",1]]  // 16
```

**8. `fur-elise` — Für Elise — familiar — bpm 76, 3/4, 4 bars (12 beats)**

Loop-edit: reduced to the opening period and closed on E (the dominant), which resolves back to the
A-minor opening.

```js
mel : [["E5",.5],["D#5",.5],["E5",.5],["D#5",.5],["E5",.5],["B4",.5],
       ["D5",.5],["C5",.5],["A4",2],
       ["C4",.5],["E4",.5],["A4",.5],["B4",1.5],
       ["E4",.5],["G#4",.5],["B4",.5],["C5",1.5]]                           // 12
bass: [["A2",3],["A2",3],["A2",3],["E2",3]]                                 // 12
```

---

### Original tracks (written for this project, no source work)

Three fixed loops of my own. Built from modes and common progressions — progressions are not
copyrightable, and none of these quote an existing melody.

**9. `lantern` — original — calm/bright — bpm 68, 4/4, 4 bars (16 beats)**

F Lydian. The raised 4th (B natural over an F chord) is what gives it the open, slightly weightless
feel. Sparse bell-like melody over a slow pad.

```js
pad : [[["F3","A3","C4","E4"],4], [["G3","B3","D4"],4],
       [["E3","G3","B3"],4],      [["F3","A3","C4","E4"],4]]                // 16
mel : [["R",2],["C5",2], ["B4",2],["D5",2], ["R",2],["G4",2], ["A4",3],["R",1]]  // 16
```

**10. `hollow` — original — moody/dark — bpm 56, 4/4, 4 bars (16 beats)**

D Aeolian, i–VI–III–VII, which closes back on itself. Long tones and a lot of space. Good for night.

```js
pad : [[["D3","F3","A3"],4], [["Bb2","D3","F3"],4],
       [["F3","A3","C4"],4], [["C3","E3","G3"],4]]                          // 16
mel : [["R",4], ["A4",3],["R",1], ["F4",2],["E4",2], ["D4",3],["R",1]]      // 16
```

**11. `tinker` — original — playful/busy — bpm 104, 4/4, 4 bars (16 beats)**

G Dorian — the natural 6th (E) against the minor 3rd (Bb) is the characteristic colour, bright
without being sweet. A walking bass gives it forward motion; suits the "building something" mood.

```js
bass: [["G2",1],["D3",1],["G2",1],["D3",1], ["C3",1],["G3",1],["C3",1],["G3",1],
       ["G2",1],["D3",1],["G2",1],["D3",1], ["F2",1],["C3",1],["F2",1],["C3",1]]  // 16
mel : [["D4",.5],["F4",.5],["G4",1],["Bb4",1],["A4",1],
       ["G4",.5],["A4",.5],["E5",1],["Bb4",2],
       ["D5",1],["C5",1],["Bb4",1],["A4",1],
       ["G4",2],["R",2]]                                                    // 16
```

---

### Fidelity labelling

**No longer applicable — do not build this.** This table existed because the early tracks were
memory-based fragments ("ostinato figure only; melody omitted") that would have been dishonest to
present as the work itself. Every shipped track is now a complete piece generated from a real score,
so each one is labelled with its plain composer name and no "after X" qualifier. Greensleeves stays
*traditional* because it genuinely is.

Use the `composer`, `year` and `marking` fields already on each track object.

If a melody ever needs correcting, only its `seq` array changes — the engine is untouched. In
practice the fix is to re-run `mxl2track.py` rather than to edit note data by hand.

**Transcription method** (superseded by MusicXML, kept for the one case it still covers: a piece that
exists only as a scan). Render the page to an image; find
staff lines as rows dark across >55% of the width; the diatonic step is half the line spacing; detect
noteheads as **enclosed white regions** for hollow half-notes, or as wide dark blobs after stripping
staff lines with a vertical run-length filter for filled notes; convert with
`steps = (y − topLine) / stepPx` and walk the letter ladder down from the top line, then apply the
key signature. Sanity check: true notehead centres land within ~0.05 of integer step values — if
readings sit mid-step, the staff-line detection is wrong. Works well on sparse slow textures;
degrades on dense polyphony, beamed runs, and heavy ledger lines.

### A1c-note — recommended track list

**Resolved: ship all 13 tracks in `music-tracks.js`, and nothing else.**

This section previously recommended dropping Moonlight, Für Elise, Canon and Greensleeves because
they existed only as memory-based approximations, and a half-right rendering of a piece the listener
knows is worse than not offering it. That reasoning is spent — MusicXML was found for all four and
they went through `mxl2track.py` like the rest. There are no memory-based tracks left anywhere in
the file.

The three "original" compositions sketched earlier in this plan are **not needed** and should not be
written. 37 minutes across 5 moods is more than the mode requires, and every minute of it is a real
piece by a named composer rather than something invented to dodge a licensing question.

Licensing is closed too — see **Licensing** above. There is no outstanding decision on this section.

### A1d — UI and behaviour

- Add a **Music** control to the prototype bar: an on/off toggle, a track dropdown, and a volume
  slider bound to `MUSIC.master.gain`.
- **Never autoplay.** Browsers block audio before a user gesture, and silently starting music is
  hostile. Default OFF; start only on an explicit click.
- Persist the chosen track and volume to `localStorage` alongside the journal state.
- **Independent of the SFX mute.** Music and effects need separate controls — someone may want
  animal sounds without music or vice versa.
- Duck the music to ~35% for 400ms whenever the preflight fanfare or a discovery jingle plays, then
  restore. Prevents the two from fighting.
- Crossfade on track change: 300ms out, swap, 300ms in.
- Suspend the `AudioContext` when `document.hidden`, resume on focus — same rule as the world
  update loop.
- Mood tags drive an optional **Auto** mode: `calm` by default, `exciting` while the preflight check
  runs, `moody` at night, `bright` on Completionist.

### Verify

Each track loops seamlessly with no click or gap at the loop point; the picker switches without
artefacts; the accelerando on Mountain King builds and resets; nothing plays before a click; music
survives a reload with its settings; and it ducks rather than colliding with the existing blips.

---

## Regression checklist (run after all fixes)

0. Sweeping the mouse edge to edge moves animals and props **with** their terrain layer — nothing
   slides across the landscape or bobs vertically, and hitboxes stay on their targets.
1. Walking animals keep their feet on the terrain across the full screen width, at every layer.
2. Bear/deer/fox/hedgehog are distinguishable in silhouette.
3. Pond click → fish arcs, is catchable mid-air, splashes if missed.
4. `bear-fish` completes: catch fish → click bear → hearts → egg fires.
5. Field Journal still counts to 54 and persists across reload.
6. v3 UI untouched: Play → preflight runs; shelf tiles select; drag-to-rearrange works; Crash
   Doctor opens from the status pill; Advanced drawer opens.
7. `prefers-reduced-motion`: no ambient motion, but props and animals still clickable and
   discoveries still register.
8. No console errors; steady frame rate with ~12 entities.
9. **Five minutes idle:** animals are still arriving and leaving, entity count oscillates instead of
   climbing to the cap, and no entity has been on screen the whole time.
10. The pond fills a terrain dip with a flat surface and blocky edges; ducks and frogs sit on that
    surface; the fish launches from it and splashes back into it.
11. **The Field Journal can reach 54/54.** Every chain completes; no egg depends on an item with
    no source or a species that cannot spawn.
12. Clicking a selected shelf tile deselects it.
13. Music: nothing plays until clicked; tracks loop without a seam; switching crossfades cleanly;
    settings survive reload; music ducks under the preflight fanfare instead of colliding with it.

## Notes

- Keep everything in the single file. No new assets, no network.
- Do not add libraries. All drawing stays `fillRect` on the existing canvas.
- If a fix conflicts with something already working, prefer keeping the working behaviour and note
  the conflict rather than guessing.
