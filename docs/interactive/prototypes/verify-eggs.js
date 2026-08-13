/**
 * verify-eggs.js — reachability harness for the v4 living world.
 *
 * Drives all 54 easter eggs through the SAME code paths a real click takes
 * (`WORLD.interact` for entities and items, `prop.reaction` for props, real DOM
 * events for the weather and time controls) and reports which are actually
 * completable. Use it after any fix pass to prove nothing regressed.
 *
 * ── HOW TO RUN ──────────────────────────────────────────────────────────────
 * 1. The world lives inside an IIFE, so expose it once. In v4-world.html find
 *    the line `WORLD.interact=function(hit){` and insert immediately above it:
 *
 *      window.__W=WORLD; window.__SP=SPECIES_BY_KEY; window.__EGGS_REF=null;
 *
 *    (Optionally also `window.__EGGS=EGGS;` next to `var EGG_BY_ID={}`.)
 * 2. Serve the file and open it in a browser.
 * 3. Paste this whole file into the console, then run:  await verifyEggs()
 *
 * ── WHY IT LOOKS LIKE THIS ──────────────────────────────────────────────────
 * Three traps cost real time when this was written by hand. All are handled
 * below; do not "simplify" them away:
 *
 *  • `document.hidden` is TRUE whenever the tab/pane is not displayed, and the
 *    world skips its update loop entirely in that case. `WORLD.t` never
 *    advances and entity `uid`s are never assigned, so time-dependent eggs
 *    silently fail. We drive frames manually with `WORLD.update(1/60)`.
 *  • Prop state (`clicks`, `rolled`, `lit`, `bloom`, `picked`) persists across a
 *    `found`/`flags` reset, so a test that rolls the boulder makes the NEXT
 *    boulder test fail. `resetProps()` restores it.
 *  • `campfire-tales` uses real `setTimeout`s (~4.8s) and cannot be
 *    fast-forwarded by manual ticking — it gets a real wait.
 *
 * Exit: returns a summary object and prints a table. `blocked` lists eggs that
 * cannot be completed, each with the reason.
 */
(function (global) {
  'use strict';

  var W = function () { return global.__W; };
  var SP = function () { return global.__SP; };

  /* ---------- helpers ---------- */

  function tick(seconds) {                      // manual frame driver (see note above)
    var n = Math.round(seconds * 60);
    for (var i = 0; i < n; i++) W().update(1 / 60);
    return n;
  }
  function wait(ms) { return new Promise(function (r) { setTimeout(r, ms); }); }

  function resetWorld() {
    var w = W();
    w.found = {}; w.flags = {}; w.carry = null;
    w.entities.length = 0; w.items.length = 0;
    w.dl = 0.8;                                  // default to daytime
  }

  // Props are created once and mutate in place — restore every field a
  // reaction can touch, or earlier tests poison later ones.
  var PROP_SNAPSHOT = null;
  function snapshotProps() {
    PROP_SNAPSHOT = W().props.map(function (p) {
      return { p: p, clicks: p.clicks, rolled: p.rolled, lit: p.lit, bloom: p.bloom,
               picked: p.picked, tipped: p.tipped, shrooms: p.shrooms, berries: p.berries,
               glow: p.glow, opened: p.opened, ripple: p.ripple, blink: p.blink };
    });
  }
  function resetProps() {
    if (!PROP_SNAPSHOT) return;
    PROP_SNAPSHOT.forEach(function (s) {
      var p = s.p;
      p.clicks = s.clicks; p.rolled = s.rolled; p.lit = s.lit; p.bloom = s.bloom;
      p.picked = s.picked; p.tipped = s.tipped; p.shrooms = s.shrooms; p.berries = s.berries;
      p.glow = s.glow; p.opened = s.opened; p.ripple = s.ripple; p.blink = s.blink;
    });
    // drop props created at runtime (snowman, rainbow ends, saplings …)
    var keep = PROP_SNAPSHOT.map(function (s) { return s.p; });
    W().props = W().props.filter(function (p) { return keep.indexOf(p) >= 0; });
  }

  function reset() { resetWorld(); resetProps(); }

  function props(key) { return W().props.filter(function (p) { return p.key === key; }); }
  function prop(key, n) { var p = props(key)[n || 0]; if (p && p.reaction) p.reaction(p); return p; }
  function ent(key, opts) { var sp = SP()[key]; return sp ? W().spawn(sp, opts || { x: 600 }) : null; }
  function click(e) { W().interact({ kind: 'entity', obj: e }); }
  function grab(id) {
    var it = W().items.filter(function (i) { return i.id === id; })[0];
    if (!it) return false;
    W().interact({ kind: 'item', obj: it });
    return W().carry === id;
  }
  // F3: the fish is a clickable ENTITY arcing out of the pond, not a ground
  // item — click it mid-air to catch it (this is the real gameplay path).
  function catchFish() {
    prop('pond');
    var f = W().entities.filter(function (e) { return e.sp && e.sp.key === 'fish'; })[0];
    if (!f) return false;
    tick(0.2);
    W().interact({ kind: 'entity', obj: f });
    return W().carry === 'fish';
  }
  function has(id) { return !!W().found[id]; }
  function night() { W().dl = 0.2; }
  function pondX() { var p = props('pond')[0]; return p ? p.x : 400; }

  function sweepTime() {                         // real input events on the time slider
    var s = document.getElementById('timeSlider');
    if (!s) return false;
    for (var v = 0; v <= 100; v += 5) {
      s.value = v; s.dispatchEvent(new Event('input', { bubbles: true }));
    }
    return true;
  }
  function cycleWeather(times) {                 // real clicks on the weather button
    var b = document.getElementById('tglWeather');
    if (!b) return false;
    for (var i = 0; i < (times || 1); i++) b.click();
    return true;
  }

  /* ---------- the tests ----------
     Each entry: [eggId, fn]. fn returns true, or a string explaining the block.
     Tests that need real wall-clock time return a Promise.               */

  var CREATURE_CLICK = [
    ['fox-nap', 'fox'], ['deer-stare', 'deer'], ['bear-roar', 'bear'], ['bird-song', 'songbird'],
    ['goose-honk', 'goose'], ['gull-scream', 'seagull'], ['hog-ball', 'hedgehog'], ['wolf-howl', 'wolf'],
    ['squirrel-nut', 'squirrel'], ['rabbit-thump', 'rabbit'], ['owl-spin', 'owl'], ['bat-loop', 'bat'],
    ['frog-jump', 'frog'], ['turtle-hide', 'turtle'], ['butterfly-land', 'butterfly'],
    ['crow-scatter', 'crow'], ['pecker-hole', 'woodpecker'], ['raccoon-guilt', 'raccoon'],
    ['moose-bellow', 'moose'], ['boar-truffle', 'boar'], ['mouse-hide', 'mouse']
  ];

  var TESTS = [];

  CREATURE_CLICK.forEach(function (pair) {
    TESTS.push([pair[0], function () {
      night();                                   // night species are eligible; day ones ignore it
      var e = ent(pair[1]);
      if (!e) return 'species "' + pair[1] + '" missing';
      tick(0.2);                                 // uids are assigned in the update loop
      click(e);
      return has(pair[0]) || 'clicked, no findEgg';
    }]);
  });

  TESTS.push(
    ['firefly-sync', function () {
      night(); var f = [];
      for (var i = 0; i < 3; i++) f.push(ent('firefly', { x: 300 + i * 40 }));
      tick(0.2); f.forEach(click);
      return has('firefly-sync') || 'needs 3 distinct fireflies';
    }],
    ['flower-bloom', function () { prop('flowers'); return has('flower-bloom') || 'no fire'; }],
    ['rock-beetle',  function () { prop('rock');    return has('rock-beetle')  || 'no fire'; }],
    ['log-shroom',   function () { prop('log');     return has('log-shroom')   || 'no fire'; }],
    ['pond-fish', function () {
      prop('pond');
      var f = W().entities.filter(function (e) { return e.sp && e.sp.key === 'fish'; })[0];
      if (!f) return 'no fish spawned';
      tick(0.2); click(f);
      return has('pond-fish') || 'catching the fish did not fire';
    }],

    /* --- tier 2 --- */
    ['acorn-squirrel', function () {
      prop('oak'); if (!grab('acorn')) return 'oak dropped no acorn';
      var s = ent('squirrel'); tick(0.2); click(s);
      return has('acorn-squirrel') || 'squirrel did not accept acorn';
    }],
    ['pinecone-squirrel', function () {
      prop('pine'); if (!grab('pinecone')) return 'pine dropped no pinecone';
      var s = ent('squirrel'); tick(0.2); click(s);
      return has('pinecone-squirrel') || 'squirrel did not accept pinecone';
    }],
    ['berry-hog', function () {
      prop('bush'); if (!grab('berry')) return 'bush yielded no berry';
      var h = ent('hedgehog'); tick(0.2); click(h);
      return has('berry-hog') || 'hedgehog did not accept berry';
    }],
    ['honey-bear', function () {
      var hv = props('hive')[0]; if (!hv) return 'no hive prop';
      hv.reaction(hv); hv.reaction(hv); hv.reaction(hv);
      if (!grab('honey')) return 'hive yielded no honey after 3 clicks';
      var b = ent('bear'); tick(0.2); click(b);
      return has('honey-bear') || 'bear did not accept honey';
    }],
    ['feather-scarecrow', function () {
      var g = ent('seagull'); tick(0.2); click(g);
      if (!grab('feather')) return 'seagull dropped no feather';
      prop('scarecrow');
      return has('feather-scarecrow') || 'scarecrow did not accept feather';
    }],
    ['truffle-pond', function () {
      var b = ent('boar'); tick(0.2); click(b);
      if (!grab('truffle')) return 'boar yielded no truffle';
      W().drop(pondX(), 500);
      return has('truffle-pond') || 'pond did not accept truffle';
    }],
    ['acorn-pond', function () {
      prop('oak'); if (!grab('acorn')) return 'no acorn';
      W().drop(pondX(), 500);
      return has('acorn-pond') || 'pond did not accept acorn';
    }],
    ['duck-line', function () {
      var d = ent('duck'); tick(0.2);
      for (var i = 0; i < 6; i++) click(d);
      return has('duck-line') || '6 clicks did not line up ducklings';
    }],
    ['boulder-hole', function () {
      var b = props('boulder')[0]; if (!b) return 'no boulder prop';
      for (var i = 0; i < 6; i++) b.reaction(b);
      return has('boulder-hole') || 'boulder did not roll';
    }],
    ['bee-hive', function () {
      for (var i = 0; i < 4; i++) { var bee = ent('bee', { x: 300 + i * 50 }); tick(0.2); click(bee); }
      return has('bee-hive') || 'following bees did not reveal the hive';
    }],
    ['snail-lily', function () {
      // F13: a snail clicked at the pond's edge fires directly; away from the
      // water you now CARRY it to the lily pads instead (deliberate two-step).
      var s = ent('snail', { x: pondX() }); tick(0.2); click(s);
      return has('snail-lily') || 'snail near pond did not fire';
    }],
    ['fish-otter', function () {
      var o = ent('otter'); tick(0.2);
      if (!catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
      click(o);
      return has('fish-otter') || 'otter did not accept fish';
    }],
    ['flower-deer', function () {
      props('flowers').forEach(function (p) { p.reaction(p); });
      if (!grab('flower')) return 'BLOCKED: nothing produces a flower item (F10)';
      var d = ent('deer'); tick(0.2); click(d);
      return has('flower-deer') || 'deer did not accept flower';
    }],
    ['firefly-cave', function () {
      night(); var f = ent('firefly'); tick(0.2); click(f);
      if (W().carry !== 'firefly') return 'BLOCKED: firefly is not carryable (F10)';
      prop('cave');
      return has('firefly-cave') || 'cave did not accept firefly';
    }],

    /* --- tier 3 --- */
    ['moonlit-rave', function () {
      night(); var f = [];
      for (var i = 0; i < 7; i++) f.push(ent('firefly', { x: 300 + i * 40 }));
      tick(0.2); f.forEach(click);
      var sw = W().entities.filter(function (e) { return e.data && e.data.isSwarm; })[0];
      if (!sw) return '7 fireflies did not form a swarm';
      tick(0.2); click(sw);
      return has('moonlit-rave') || 'swarm click did not fire';
    }],
    ['migration', function () {
      var g = ent('goose', { x: 600 }); tick(0.2);
      click(g); tick(0.3); click(g); tick(0.3); click(g);
      return has('migration') || 'three lead-goose clicks did not fire';
    }],
    ['wolf-pack', function () {
      night(); var w = ent('wolf', { x: 600 }); tick(0.2); click(w); tick(0.3);
      var others = W().entities.filter(function (e) { return e.sp === SP().wolf && e !== w; });
      others.forEach(click);
      return has('wolf-pack') || ('only ' + others.length + ' wolves answered');
    }],
    ['snowman', function () {
      var piles = props('snowpile');
      if (!piles.length) return 'no snowpile props (needs snow weather to be visible in play)';
      piles.slice().sort(function (a, b) { return a.rank - b.rank; })
           .forEach(function (p) { p.reaction(p); });
      var sm = props('snowman')[0];
      if (!sm) return 'piles clicked in size order did not build a snowman';
      sm.reaction(sm);
      return has('snowman') || 'snowman click did not fire';
    }],
    ['constellation', function () {
      night(); var st = props('star');
      if (!st.length) return 'no star props';
      [5, 4, 3, 2, 1].forEach(function (r) {
        var s = st.filter(function (x) { return x.rank === r; })[0];
        if (s) s.reaction(s);
      });
      return has('constellation') || 'brightness order did not fire';
    }],
    ['fairy-ring', function () {
      var ms = props('mushroom');
      if (ms.length < 7) return 'only ' + ms.length + ' mushrooms in the ring';
      ms.forEach(function (m) { m.reaction(m); });
      return has('fairy-ring') || 'all mushrooms within the window did not fire';
    }],
    ['rainbow', function () {
      W().dl = 0.8;                              // must be daytime
      if (!cycleWeather(3)) return 'no weather button';
      var ends = props('rainbow-end');
      if (!ends.length) return 'rain->clear in daylight spawned no rainbow';
      ends.forEach(function (e) { e.reaction(e); });
      return has('rainbow') || 'clicking both ends did not fire';
    }],
    ['acorn-hunt', function () {
      // NOTE: counts acorns DROPPED at 3 separate spots — giving one to a
      // squirrel fires acorn-squirrel and returns early instead. The pond is
      // dynamic now (anchored to the water), so pick spots well clear of it.
      var px = pondX();
      var vw = global.innerWidth || 1280;
      var spots = [];
      [0.2, 0.5, 0.8].forEach(function (f) {
        var s = Math.round(vw * f);
        if (Math.abs(s - px) < 150) s = s < px ? Math.max(20, s - 300) : Math.min(vw - 20, s + 300);
        spots.push(s);
      });
      spots.forEach(function (x) { prop('oak'); if (grab('acorn')) W().drop(x, 500); });
      if (!W().flags.acornHuntArmed) return 'three buries did not arm the hunt';
      if (!sweepTime()) return 'no time slider';
      return has('acorn-hunt') || 'armed, but a full day sweep did not fire';
    }],
    ['full-day', function () {
      if (!sweepTime()) return 'no time slider';
      return has('full-day') || 'full sweep did not fire';
    }],
    ['water-flowers', function () {
      // The flowers consume the carried water (one dose per patch), so the
      // player refills from the pond between patches — that's the intended loop.
      var ok = props('flowers').every(function (p) {
        prop('pond');                              // refill (empty-handed each time)
        if (W().carry !== 'water') return false;
        p.reaction(p);
        return true;
      });
      if (!ok) return 'BLOCKED: nothing produces a water item (F10)';
      return has('water-flowers') || 'watering all patches did not fire';
    }],
    ['bear-fish', function () {
      var b = ent('bear'); tick(0.2);
      if (!catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
      click(b);
      return has('bear-fish') || 'bear did not accept fish';
    }],
    ['bear-feast', function () {
      var b = ent('bear'); tick(0.2);
      if (!catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
      click(b);
      ['berry', 'honey'].forEach(function (i) { W().carry = i; click(b); });
      return has('bear-feast') || 'three courses did not fire';
    }],
    ['the-long-con', function () {
      var bo = props('boulder')[0];
      for (var i = 0; i < 6; i++) bo.reaction(bo);
      if (!W().flags.boulderMoved) return 'boulder did not roll';
      var fx = ent('fox', { x: 700 }); tick(0.2);
      if (!catchFish()) return 'BLOCKED: cannot catch a fish (F3)';
      click(fx);                                  // befriend
      if (!fx.data.friend) return 'fox not befriended by fish';
      tick(1.5);
      click(fx);                                  // send to fetch
      if (!fx.data.fetchQuest) return 'second click did not start the fetch';
      tick(20);
      return wait(1200).then(function () {         // key drops on a real 900ms timer
        var key = W().items.filter(function (i) { return i.id === 'key'; })[0];
        if (!key) return 'fox fetched nothing';
        W().interact({ kind: 'item', obj: key });
        if (W().carry !== 'key') return 'key not picked up';
        prop('cave');
        return has('the-long-con') || 'cave did not open with the key';
      });
    }],
    ['campfire-tales', function () {
      // real setTimeouts (~4.8s) — cannot be fast-forwarded
      night();
      var cf = props('campfire')[0]; if (!cf) return 'no campfire prop';
      cf.reaction(cf); cf.reaction(cf); cf.reaction(cf);
      if (!cf.lit) return 'three clicks did not light the fire';
      return wait(5600).then(function () {
        var g = W().entities.filter(function (e) { return e.data && e.data.atCampfire; });
        if (g.length < 3) return 'only ' + g.length + ' animals gathered';
        g.forEach(click);
        return has('campfire-tales') || 'clicking all three did not fire';
      });
    }]
  );

  /* ---------- runner ---------- */

  global.verifyEggs = async function verifyEggs(opts) {
    opts = opts || {};
    if (!W()) { console.error('__W is not exposed — see the HOW TO RUN header.'); return; }
    if (!PROP_SNAPSHOT) snapshotProps();

    var results = [], blocked = [], failed = [];

    for (var i = 0; i < TESTS.length; i++) {
      var id = TESTS[i][0], fn = TESTS[i][1], out;
      reset();
      try { out = fn(); if (out && typeof out.then === 'function') out = await out; }
      catch (err) { out = 'ERROR: ' + (err && err.message ? err.message : String(err)); }

      var pass = out === true;
      var why = pass ? '' : String(out);
      results.push({ egg: id, result: pass ? 'PASS' : (why.indexOf('BLOCKED') === 0 ? 'BLOCKED' : 'FAIL'), detail: why });
      if (!pass) (why.indexOf('BLOCKED') === 0 ? blocked : failed).push(id + ' — ' + why);
    }

    var known = global.__EGGS ? global.__EGGS.length : 54;
    var passed = results.filter(function (r) { return r.result === 'PASS'; }).length;

    if (!opts.quiet) {
      console.table(results);
      console.log('%c' + passed + ' / ' + results.length + ' reachable  (registry declares ' + known + ' eggs)',
                  'font-weight:bold;font-size:13px');
      if (blocked.length) console.warn('BLOCKED (missing item source):\n  ' + blocked.join('\n  '));
      if (failed.length)  console.error('FAILED (unexpected):\n  ' + failed.join('\n  '));
      if (results.length < known) {
        console.warn('Harness covers ' + results.length + ' of ' + known +
                     ' declared eggs — add tests for any egg added since this was written.');
      }
    }
    reset();
    return { passed: passed, total: results.length, declared: known, blocked: blocked, failed: failed, results: results };
  };

  console.log('verify-eggs loaded — run:  await verifyEggs()');
})(typeof window !== 'undefined' ? window : globalThis);
