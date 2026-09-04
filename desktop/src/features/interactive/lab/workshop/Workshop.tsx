/**
 * The Workshop — Agora Lab.
 *
 * Six benches as physical workstations, each four steps deep that escalate:
 *   do       — handle it. Learn what the pieces are and how they behave.
 *   predict  — commit to a guess BEFORE seeing the outcome. Being wrong is
 *              the point; a surprise is what makes the rule stick.
 *   transfer — the same principle wearing different clothes (the only way to
 *              tell understanding from pattern-matching).
 *   explain  — the mechanism and the real vocabulary, once there is something
 *              to attach them to.
 * Later steps stay locked until the earlier ones are done, so difficulty only
 * goes up. In the Lab, animation communicates causality, not decoration —
 * nothing moves on its own; every snap/reject is the consequence of what the
 * learner just did.
 *
 * The bench widgets build their own DOM imperatively (exactly as the
 * prototype did) so the drag/place/reject logic is a faithful port. All loaders
 * are fictional (Loader A/B/C, No loader) — real product names appear only in
 * the explain ("Why") steps, which open the Field Guide for the accurate
 * version.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ART, draggable, paint, type ArtFn } from './workshopPieces';
import { blip, nope, syncWorkshopSound, yay } from './workshopSound';
import {
  STATIONS,
  benchComplete,
  firstUndone,
  loadProgress,
  markStepDone,
  saveProgress,
  stepCount,
  stepsDone,
  type ProgressMap,
  type WorkshopBench,
} from './workshopModel';
import './workshop.css';

export interface WorkshopProps {
  /** Opens the Field Guide at a topic (the "Why" step's real vocabulary). */
  onOpenGuide?: (topicId: string) => void;
  /** Real Agora destinations (kept for app-boundary wiring; the workshop is a
   * simulation and only the Guide is opened from the benches). */
  onNavigateStandard?: (dest: import('../../domain/intents').StandardDestination) => void;
  /** A bench drops ambience to `calm`; restore on close. */
  onAmbienceChange?: (profile: 'calm' | null) => void;
  reducedMotion?: boolean;
  /** Respects the global ambience sound setting (read by the page). */
  soundOn?: boolean;
}

/* ── DOM helpers ── */

function sayEl(host: HTMLElement): HTMLElement {
  const d = document.createElement('div');
  d.className = 'ws-say';
  host.appendChild(d);
  return d;
}

function tell(el: HTMLElement, msg: string, kind?: 'good' | 'bad'): void {
  el.textContent = msg;
  el.className = 'ws-say' + (kind ? ' ' + kind : '');
}

function moreInfo(parent: HTMLElement, html: string): void {
  const d = document.createElement('details');
  d.className = 'ws-grown';
  d.innerHTML = '<summary>More info</summary><p></p>';
  d.querySelector('p')!.innerHTML = html;
  parent.appendChild(d);
}

function centerOf(el: Element): [number, number] {
  const r = el.getBoundingClientRect();
  return [r.left + r.width / 2, r.top + r.height / 2];
}

function burst(x: number, y: number, color: string, n = 18, spread = 8): void {
  const fx = document.querySelector('.ws-fx') as HTMLCanvasElement | null;
  if (!fx) return;
  const fc = fx.getContext('2d');
  if (!fc) return;
  for (let i = 0; i < n; i++) {
    particles.push({
      x, y,
      vx: (Math.random() - 0.5) * spread,
      vy: (Math.random() - 0.95) * spread,
      s: 3 + Math.random() * 4,
      life: 1,
      color,
      rot: Math.random() * 6,
    });
  }
  if (particles.length > 240) particles.splice(0, particles.length - 240);
  if (!fx.dataset.looping) {
    fx.dataset.looping = '1';
    (function loop() {
      fc.clearRect(0, 0, fx.width, fx.height);
      for (let i = particles.length - 1; i >= 0; i--) {
        const p = particles[i];
        p.vy += 0.3; p.x += p.vx; p.y += p.vy; p.life -= 0.016; p.rot += 0.2;
        if (p.life <= 0) { particles.splice(i, 1); continue; }
        fc.globalAlpha = Math.max(p.life, 0);
        fc.fillStyle = p.color;
        fc.save();
        fc.translate(p.x, p.y);
        fc.rotate(p.rot);
        fc.fillRect(-p.s / 2, -p.s / 2, p.s, p.s);
        fc.restore();
      }
      fc.globalAlpha = 1;
      if (particles.length > 0) requestAnimationFrame(loop);
      else { fx.dataset.looping = ''; fc.clearRect(0, 0, fx.width, fx.height); }
    })();
  }
}

const particles: Array<{ x: number; y: number; vx: number; vy: number; s: number; life: number; color: string; rot: number }> = [];

/* Station art for the map tiles — the prototype paints each station's icon on
   its tile; the React port declared the canvases but never painted them. */
const ART_BY_STATION: Record<string, ArtFn> = {
  build: ART.build,
  mod: ART.mod,
  fix: ART.fix,
  heal: ART.heal,
  offline: ART.offline,
  undo: ART.undo,
};

/* ── predict step ── */

interface QuizOption { t: string; right?: boolean; why: string }
interface Quiz { options: QuizOption[]; after?: (host: HTMLElement) => void }

function askStep(host: HTMLElement, q: Quiz, onDone: () => void): void {
  const wrap = document.createElement('div');
  wrap.className = 'ws-quiz';
  host.appendChild(wrap);
  const out = sayEl(host);
  tell(out, 'No penalty for guessing wrong. Guessing wrong is how this works.');
  const LETTERS = ['A', 'B', 'C', 'D'];
  q.options.forEach((o, i) => {
    const b = document.createElement('button');
    b.className = 'ws-opt';
    // Each answer gets its own letter badge (A/B/C/D) so it is visually
    // special on its own — NOT a green/red icon that teaches pattern
    // recognition. The explanation text carries the right/wrong.
    const key = document.createElement('span');
    key.className = 'ws-opt-key';
    key.textContent = LETTERS[i] ?? String(i + 1);
    const label = document.createElement('span');
    label.textContent = o.t;
    b.appendChild(key);
    b.appendChild(label);
    b.onclick = () => {
      if (wrap.dataset.answered) return;
      wrap.dataset.answered = '1';
      wrap.querySelectorAll('.ws-opt').forEach((x, k) => {
        x.classList.add(q.options[k].right ? 'right' : 'wrong');
        (x as HTMLButtonElement).disabled = true;
      });
      b.classList.add('picked');
      if (o.right) {
        blip(880, 0.14);
        const c = centerOf(b);
        burst(c[0], c[1], '#FFD34E', 20, 9);
        tell(out, 'Yes. ' + o.why, 'good');
      } else {
        nope();
        const right = q.options.filter((x) => x.right)[0];
        tell(out, 'Not quite. ' + o.why + ' ' + right.t + ' — ' + right.why, 'bad');
      }
      if (q.after) q.after(host);
      // Mark the step and let the caller append "Next" — the reveal and the
      // explanation stay on screen until the learner chooses to move on.
      onDone();
    };
    wrap.appendChild(b);
  });
}

/* ── explain step ── */

function explainStep(host: HTMLElement, html: string, onDone: () => void, guideTopic?: string, openGuide?: (t: string) => void): void {
  const d = document.createElement('div');
  d.className = 'ws-explain';
  d.innerHTML = html;
  host.appendChild(d);
  const row = document.createElement('div');
  row.className = 'ws-acts';
  if (guideTopic && openGuide) {
    const g = document.createElement('button');
    g.className = 'ws-btn';
    g.textContent = 'Open the Field Guide';
    g.onclick = () => openGuide(guideTopic);
    row.appendChild(g);
  }
  const b = document.createElement('button');
  b.className = 'ws-btn p';
  b.textContent = 'Got it — back to the workshop';
  b.onclick = () => { b.disabled = true; onDone(); };
  row.appendChild(b);
  host.appendChild(row);
}

/* ── Build it: worldBench ── */

interface WorldCfg {
  slotLabel?: string;
  goal?: { name: string; note: string; art: ArtFn };
  needVersion?: string;
  needLoaderNotNone?: boolean;
  hint: string;
  win: (sp: { id: string; name: string }) => string;
}

function worldBench(host: HTMLElement, cfg: WorldCfg, onWin: () => void): void {
  const state: { version: string | null; loader: string | null } = { version: null, loader: null };
  const wrap = document.createElement('div');
  wrap.className = 'ws-row';
  host.appendChild(wrap);
  const tray = document.createElement('div');
  tray.className = 'ws-tray';
  tray.innerHTML = '<h4>Parts bin</h4>';
  wrap.appendChild(tray);
  const slot = document.createElement('div');
  slot.className = 'ws-slotbig';
  slot.innerHTML = '<h4>' + (cfg.slotLabel || 'Your world') + '</h4>';
  wrap.appendChild(slot);
  if (cfg.goal) {
    const g = document.createElement('div');
    g.className = 'ws-piece';
    g.style.cssText = 'width:126px;cursor:default';
    g.innerHTML = '<canvas></canvas><b></b><small></small>';
    g.querySelector('b')!.textContent = cfg.goal.name;
    g.querySelector('small')!.textContent = cfg.goal.note;
    slot.appendChild(g);
    requestAnimationFrame(() => paint(g.querySelector('canvas') as HTMLCanvasElement, cfg.goal!.art));
  }
  const out = sayEl(host);
  tell(out, cfg.hint);

  type PartSpec = { id: string; name: string; note: string; kind: string; fits?: string; art: ArtFn };
  const PARTS: PartSpec[] = [
    { id: 'v1201', name: 'Game 1.20.1', note: 'a version', kind: 'version', art: ART.crystal('#6FD3E8') },
    { id: 'v121', name: 'Game 1.21', note: 'a version', kind: 'version', art: ART.crystal('#8BB6F2') },
    // Deliberately fictional. The lesson needs a loader that does NOT fit, and
    // attaching that to a real product would be inventing a compatibility
    // claim about someone else's software. Real names appear only in the Why
    // step, where they are described accurately.
    { id: 'loaderA', name: 'Loader A', note: 'made for 1.20.1', kind: 'loader', fits: 'v1201', art: ART.gear('#8BE24F') },
    { id: 'loaderB', name: 'Loader B', note: 'made for 1.21', kind: 'loader', fits: 'v121', art: ART.gear('#FFB25E') },
    { id: 'none', name: 'No loader', note: 'plain game', kind: 'loader', fits: '*', art: ART.gear('#9FB3C0') },
  ];
  const partsByName: Record<string, PartSpec> = {};
  PARTS.forEach((sp) => { partsByName[sp.id] = sp; });

  PARTS.forEach((sp) => {
    const el = document.createElement('div');
    el.className = 'ws-piece';
    el.innerHTML = '<canvas></canvas><b></b><small></small>';
    el.querySelector('b')!.textContent = sp.name;
    el.querySelector('small')!.textContent = sp.note || '';
    tray.appendChild(el);
    requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, sp.art));
    draggable(el, (target) => place(sp, el, target));
  });

  function reject(el: HTMLElement, msg: string): void {
    el.classList.add('bad');
    nope();
    tell(out, msg, 'bad');
    setTimeout(() => el.classList.remove('bad'), 340);
  }

  function place(sp: PartSpec, el: HTMLElement, target: HTMLElement | null): void {
    if (target !== slot) return;
    if (sp.kind === 'version') {
      if (state.version) { reject(el, 'You already have a game version in there. One is enough.'); return; }
      const needVersion = cfg.needVersion;
      if (needVersion && sp.id !== needVersion) {
        reject(el, cfg.goal!.name + ' was built for ' + partsByName[needVersion].name + '. Put that version in instead — the mod can\'t bend to fit.');
        return;
      }
      state.version = sp.id;
      slot.appendChild(el);
      el.classList.add('ok');
      blip(760, 0.1);
      const c = centerOf(el);
      burst(c[0], c[1], '#6FD3E8', 14, 7);
      tell(out, "That's the game itself. Now a loader — that's the part that lets add-ons work.");
      return;
    }
    if (!state.version) { reject(el, 'Put a game version in first — a loader needs something to sit on.'); return; }
    const fits = sp.fits;
    if (fits && fits !== '*' && fits !== state.version) {
      reject(el, sp.name + " doesn't fit this game. It's built for " + partsByName[fits].name + '. Try the other one.');
      return;
    }
    if (state.loader) { reject(el, "You've already got a loader. One at a time."); return; }
    if (cfg.needLoaderNotNone && sp.id === 'none') {
      reject(el, 'With no loader there\'s nothing for ' + cfg.goal!.name + ' to plug into. It needs a real one.');
      return;
    }
    state.loader = sp.id;
    slot.appendChild(el);
    el.classList.add('ok');
    blip(880, 0.14);
    const c2 = centerOf(el);
    burst(c2[0], c2[1], '#8BE24F', 22, 9);
    tell(out, cfg.win(sp), 'good');
    onWin();
  }
}

/* ── Add stuff: modBench ── */

function modBench(host: HTMLElement, opt: { mode: 'basic' | 'shared' }, onWin: () => void, reduce: boolean): void {
  const placed: Record<string, boolean> = {};
  const shared = opt.mode === 'shared';
  const wrap = document.createElement('div');
  wrap.className = 'ws-row';
  host.appendChild(wrap);
  const tray = document.createElement('div');
  tray.className = 'ws-tray';
  tray.innerHTML = '<h4>Shelf</h4>';
  wrap.appendChild(tray);
  const slot = document.createElement('div');
  slot.className = 'ws-slotbig';
  slot.innerHTML = '<h4>Your world' + (shared ? ' — click anything in here to take it out' : '') + '</h4>';
  wrap.appendChild(slot);
  const out = sayEl(host);
  tell(out, shared ? 'Add both cave mods. Then take them out one at a time.' : 'Drag Better Caves in. Watch what comes with it.');

  type ModSpec = { id: string; name: string; note: string; art: ArtFn; needs?: string; enemy?: string };
  const MODS: ModSpec[] = shared ? [
    { id: 'caves', name: 'Better Caves', note: 'needs a helper', art: ART.box('#B48CF2', 'hole'), needs: 'core' },
    { id: 'deep', name: 'Deep Tunnels', note: 'needs a helper', art: ART.box('#8BB6F2', 'hole'), needs: 'core' },
    { id: 'core', name: 'Core Helper', note: 'a helper', art: ART.box('#6FD3E8', 'tab') },
  ] : [
    { id: 'caves', name: 'Better Caves', note: 'needs a helper', art: ART.box('#B48CF2', 'hole'), needs: 'core', enemy: 'terrain' },
    { id: 'core', name: 'Core Helper', note: 'a helper', art: ART.box('#6FD3E8', 'tab') },
    { id: 'tex', name: 'Nice Textures', note: 'optional', art: ART.box('#8BE24F') },
    { id: 'terrain', name: 'Terrain Redo', note: 'changes caves too', art: ART.box('#FF7A6B') },
  ];
  const byId: Record<string, ModSpec> = {};
  MODS.forEach((m) => { byId[m.id] = m; });

  MODS.forEach((m) => {
    const el = document.createElement('div');
    el.className = 'ws-piece';
    el.innerHTML = '<canvas></canvas><b></b><small></small>';
    el.querySelector('b')!.textContent = m.name;
    el.querySelector('small')!.textContent = m.note || '';
    el.dataset.mid = m.id;
    tray.appendChild(el);
    requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, m.art));
    draggable(el, (target) => place(m, el, target));
    if (shared) el.addEventListener('click', () => { if (placed[m.id]) remove(m, el); });
  });

  const elFor = (id: string): HTMLElement | null => host.querySelector('[data-mid="' + id + '"]');
  function needers(id: string): string[] {
    return Object.keys(placed).filter((k) => byId[k].needs === id);
  }

  function remove(m: ModSpec, el: HTMLElement): void {
    if (m.needs === undefined && needers(m.id).length) {
      el.classList.add('bad');
      nope();
      tell(out, byId[needers(m.id)[0]].name + ' still needs ' + m.name + ', so it stays. Take the mods that need it out first.', 'bad');
      setTimeout(() => el.classList.remove('bad'), 340);
      return;
    }
    delete placed[m.id];
    tray.appendChild(el);
    el.classList.remove('ok');
    blip(360, 0.12);
    if (placed.core && !needers('core').length) {
      const he = elFor('core');
      setTimeout(() => {
        if (!he) return;
        delete placed.core;
        tray.appendChild(he);
        he.classList.remove('ok');
        blip(300, 0.14);
        tell(out, 'Nothing needs Core Helper now, so it left too. It only stayed while something used it.', 'good');
        onWin();
      }, reduce ? 0 : 420);
      tell(out, 'Took ' + m.name + ' out. Watch the helper.');
      return;
    }
    tell(out, placed.core
      ? 'Took ' + m.name + ' out — but Core Helper stayed, because the other mod still needs it.'
      : 'Took ' + m.name + ' out.');
  }

  function place(m: ModSpec, el: HTMLElement, target: HTMLElement | null): void {
    if (target !== slot || placed[m.id]) return;
    const foe = m.enemy && placed[m.enemy]
      ? m.enemy
      : Object.keys(placed).filter((k) => byId[k].enemy === m.id)[0];
    if (foe) {
      el.classList.add('bad');
      nope();
      const fe = elFor(foe);
      if (fe) { fe.classList.add('bad'); setTimeout(() => fe.classList.remove('bad'), 340); }
      tell(out, m.name + ' and ' + byId[foe].name + ' both want to change the same caves. They can\'t both be in there — take one out.', 'bad');
      setTimeout(() => el.classList.remove('bad'), 340);
      return;
    }
    placed[m.id] = true;
    slot.appendChild(el);
    el.classList.add('ok');
    blip(780, 0.11);
    const c = centerOf(el);
    burst(c[0], c[1], '#B48CF2', 16, 8);
    if (m.needs && !placed[m.needs]) {
      const helper = elFor(m.needs);
      setTimeout(() => {
        if (!helper) return;
        placed[m.needs!] = true;
        slot.appendChild(helper);
        helper.classList.add('ok');
        blip(980, 0.12);
        const c2 = centerOf(helper);
        burst(c2[0], c2[1], '#6FD3E8', 18, 8);
        tell(out, m.name + ' can\'t run on its own, so it brought ' + byId[m.needs!].name + ' along. You don\'t have to do anything.', 'good');
        check();
      }, reduce ? 0 : 420);
      tell(out, m.name + ' is going in… and it needs a helper.');
      return;
    }
    if (m.needs && placed[m.needs]) {
      tell(out, m.name + ' needs the same helper — and it\'s already in there, so nothing new arrived.', 'good');
    }
    if (m.id === 'tex') tell(out, 'Optional extras are safe to skip. Nothing breaks either way.', 'good');
    check();
  }

  function check(): void {
    if (shared) {
      if (placed.caves && placed.deep && placed.core) {
        tell(out, 'Both mods in, one helper. Now take them out one at a time.', 'good');
      }
      return;
    }
    if (placed.caves && placed.core) onWin();
  }
  if (!shared) {
    moreInfo(host, 'Required helpers are <code>dependencies</code>; the clash is a <code>conflict</code>. Agora resolves the whole graph before staging anything.');
  }
}

/* ── Something broke: fixBench ── */

function fixBench(host: HTMLElement, opt: { mode: 'basic' | 'messenger' }, onWin: () => void, reduce: boolean): void {
  const messenger = opt.mode === 'messenger';
  const read: Record<string, boolean> = {};
  let picked: (typeof SUSPECTS)[number] | null = null;

  const CLUES = messenger ? [
    { id: 'c1', t: 'The last thing it said', s: 'It named "Nice Textures" — a mod you\'ve had for months.' },
    { id: 'c2', t: 'What changed recently', s: 'You updated Core Helper yesterday. Nothing else.' },
    { id: 'c3', t: 'What it was doing', s: '"Nice Textures" stopped because something it asked for wasn\'t there.' },
  ] : [
    { id: 'c1', t: 'The last thing it said', s: 'It named one of your add-ons right before it closed.' },
    { id: 'c2', t: 'How long it lasted', s: 'It shut down about two seconds after starting.' },
    { id: 'c3', t: 'What changed recently', s: 'You added something new yesterday.' },
  ];

  const SUSPECTS = messenger ? [
    { id: 'tex', name: 'Nice Textures', hint: 'The one named in the message', conf: 0.4, art: ART.door('#8BE24F', '?') },
    { id: 'core', name: 'The Core Helper update', hint: 'Changed yesterday; others depend on it', conf: 0.75, right: true, art: ART.door('#6FD3E8', '?') },
    { id: 'world', name: 'A damaged world', hint: 'Nothing points this way', conf: 0.1, art: ART.door('#FFB25E', '?') },
  ] : [
    { id: 'mod', name: 'The new add-on', hint: 'It was named in the message', conf: 0.8, right: true, art: ART.door('#B48CF2', '?') },
    { id: 'mem', name: 'Not enough memory', hint: 'Usually takes longer to fail', conf: 0.35, art: ART.door('#6FD3E8', '?') },
    { id: 'world', name: 'A damaged world', hint: 'Would break later, not at startup', conf: 0.15, art: ART.door('#FFB25E', '?') },
  ];

  const row = document.createElement('div');
  row.className = 'ws-row';
  host.appendChild(row);
  const box = document.createElement('div');
  box.className = 'ws-clues';
  row.appendChild(box);
  const out = sayEl(host);
  tell(out, 'Read all three clues before you guess.');

  CLUES.forEach((c) => {
    const el = document.createElement('button');
    el.className = 'ws-clue';
    el.innerHTML = '<b>' + c.t + '</b><span>Tap to read</span>';
    el.onclick = () => {
      if (read[c.id]) return;
      read[c.id] = true;
      el.classList.add('read');
      el.querySelector('span')!.textContent = c.s;
      blip(560, 0.09);
      const cc = centerOf(el);
      burst(cc[0], cc[1], '#FFD34E', 10, 6);
      tell(out, Object.keys(read).length < 3
        ? 'Good. Read the others too — one clue on its own can point the wrong way.'
        : (messenger ? 'Careful. The name in a message is where it stopped, not always why.' : "Now you've got the whole picture. Pick the most likely suspect."));
      if (Object.keys(read).length === 3) doors.style.display = 'flex';
    };
    box.appendChild(el);
  });

  const doors = document.createElement('div');
  doors.className = 'ws-doors';
  doors.style.display = 'none';
  host.appendChild(doors);

  SUSPECTS.forEach((s) => {
    const el = document.createElement('button');
    el.className = 'ws-door';
    el.innerHTML = '<canvas></canvas><b></b><small></small><div class="ws-meter"><i></i></div>';
    el.querySelector('b')!.textContent = s.name;
    el.querySelector('small')!.textContent = s.hint;
    doors.appendChild(el);
    requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, s.art));
    setTimeout(() => {
      const m = el.querySelector('.ws-meter i') as HTMLElement | null;
      if (m) {
        m.style.width = (s.conf * 100) + '%';
        m.style.background = s.conf > 0.6 ? 'hsl(120 60% 48%)' : s.conf > 0.3 ? 'hsl(45 95% 50%)' : 'hsl(8 90% 60%)';
      }
    }, 60);
    el.onclick = () => {
      doors.querySelectorAll('.ws-door').forEach((d) => d.classList.remove('picked'));
      el.classList.add('picked');
      picked = s;
      blip(640, 0.1);
      tell(out, 'You picked ' + s.name + '. Now test it — safely.');
      runBtn.disabled = false;
    };
  });

  const net = document.createElement('div');
  net.className = 'ws-net';
  net.textContent = 'Safety net: a return point is saved first. Whatever happens, you can put it back exactly as it was.';
  host.appendChild(net);

  const acts = document.createElement('div');
  acts.className = 'ws-acts';
  host.appendChild(acts);
  const runBtn = document.createElement('button');
  runBtn.className = 'ws-btn p';
  runBtn.textContent = 'Try it (one change only)';
  runBtn.disabled = true;
  acts.appendChild(runBtn);

  runBtn.onclick = () => {
    if (!picked) return;
    runBtn.disabled = true;
    const steps = ['Saving a return point…', 'Changing one thing…', 'Starting it up…'];
    let i = 0;
    (function step() {
      if (i < steps.length) {
        tell(out, steps[i]);
        blip(420 + i * 90, 0.1);
        i++;
        setTimeout(step, reduce ? 60 : 600);
        return;
      }
      if (picked!.right) {
        tell(out, messenger
          ? 'It started. The crash named Nice Textures, but the cause was the helper it depends on — the mod that fell over was just the messenger.'
          : 'It started. So it really was the new add-on — you changed one thing, so you know it was that one.', 'good');
        burst(window.innerWidth / 2, window.innerHeight * 0.4, '#8BE24F', 40, 12);
        yay();
        onWin();
      } else {
        tell(out, picked!.id === 'tex'
          ? 'Still broken. Turning off the mod that was named didn\'t help — which is a strong hint it wasn\'t the cause, only where it stopped.'
          : "Still broken. That's useful — you've ruled one out. Put it back and try another.", 'bad');
        nope();
        const u = document.createElement('button');
        u.className = 'ws-btn';
        u.textContent = 'Undo that change';
        u.onclick = () => {
          tell(out, 'Put back exactly as it was. Pick another suspect.');
          blip(520, 0.12);
          u.remove();
          runBtn.disabled = false;
        };
        acts.appendChild(u);
      }
    })();
  };
}

/* ── Heal it: healthBench ── */

function healthBench(host: HTMLElement, onWin: () => void, reduce: boolean): void {
  const out = sayEl(host);
  tell(out, 'Run the scan and read what it finds. Then pick a fix.');
  const row = document.createElement('div');
  row.className = 'ws-row';
  host.appendChild(row);
  const scanBtn = document.createElement('button');
  scanBtn.className = 'ws-btn p';
  scanBtn.textContent = 'Run the health check';
  row.appendChild(scanBtn);
  const status = document.createElement('div');
  status.className = 'ws-say';
  host.appendChild(status);

  // Loader candidates — fictional (Loader A/B/C), consistent with the other
  // benches; real names appear only in the Why step via the Field Guide.
  const candidates = [
    { id: 'loaderA', name: 'Loader A', ver: '0.15', conf: 0.85, right: true, why: 'Proven compatible with the installed mods.', art: ART.gear('#8BE24F') },
    { id: 'loaderB', name: 'Loader B', ver: '47.1', conf: 0.15, why: 'Does not fit the installed mods.', art: ART.gear('#FFB25E') },
    { id: 'loaderC', name: 'Loader C', ver: '0.25', conf: 0.45, why: 'Needs review — compatibility is not proven.', art: ART.gear('#6FD3E8') },
  ];
  const doors = document.createElement('div');
  doors.className = 'ws-doors';
  doors.style.display = 'none';
  host.appendChild(doors);

  let scanned = false;
  let picked: (typeof candidates)[number] | null = null;

  scanBtn.onclick = () => {
    if (scanned) return;
    scanned = true;
    const steps = ['Checking loaders', 'Checking each mod', 'Reading the results'];
    let i = 0;
    status.textContent = steps[i];
    const iv = setInterval(() => {
      i++;
      if (i >= steps.length) {
        clearInterval(iv);
        status.textContent = 'Scan complete: 1 blocker. Loader B does not fit the installed mods.';
        blip(523, 0.1);
        doors.style.display = 'flex';
      } else {
        status.textContent = steps[i];
      }
    }, reduce ? 80 : 450);
  };

  candidates.forEach((c) => {
    const el = document.createElement('button');
    el.className = 'ws-door';
    el.innerHTML = '<canvas></canvas><b></b><small></small><div class="ws-meter"><i></i></div>';
    el.querySelector('b')!.textContent = c.name + ' ' + c.ver;
    el.querySelector('small')!.textContent = c.why;
    doors.appendChild(el);
    requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, c.art));
    setTimeout(() => {
      const m = el.querySelector('.ws-meter i') as HTMLElement | null;
      if (m) {
        m.style.width = (c.conf * 100) + '%';
        m.style.background = c.conf > 0.6 ? 'hsl(120 60% 48%)' : c.conf > 0.3 ? 'hsl(45 95% 50%)' : 'hsl(8 90% 60%)';
      }
    }, 60);
    el.onclick = () => {
      doors.querySelectorAll('.ws-door').forEach((d) => d.classList.remove('picked'));
      el.classList.add('picked');
      picked = c;
      blip(640, 0.1);
      tell(out, 'You picked ' + c.name + '. Apply it and re-check.');
      applyBtn.disabled = false;
    };
  });

  const acts = document.createElement('div');
  acts.className = 'ws-acts';
  host.appendChild(acts);
  const applyBtn = document.createElement('button');
  applyBtn.className = 'ws-btn p';
  applyBtn.textContent = 'Apply and re-check';
  applyBtn.disabled = true;
  acts.appendChild(applyBtn);

  applyBtn.onclick = () => {
    if (!picked) return;
    applyBtn.disabled = true;
    if (picked.right) {
      tell(out, 'Green — no blockers. Loader A is proven for every installed mod. A kept warning would stay, but this setup is clean.', 'good');
      blip(880, 0.14);
      const c = centerOf(applyBtn);
      burst(c[0], c[1], '#8BE24F', 34, 11);
      yay();
      onWin();
    } else if (picked.id === 'loaderC') {
      tell(out, 'Loader C is indeterminate — not proven, so it cannot clear the blocker. Check which loaders each mod supports; Loader A is already proven for all of them.', 'bad');
      nope();
      applyBtn.disabled = false;
    } else {
      tell(out, 'Still blocked. Loader B does not fit the installed mods. Try a proven-compatible one.', 'bad');
      nope();
      applyBtn.disabled = false;
    }
  };
  moreInfo(host, 'A <code>blocker</code> stops launch. A <code>warning</code> needs a choice. A <code>recommendation</code> never blocks. The bars say likely, maybe, unlikely — never a percentage.');
}

/* ── Going offline: offlineBench ── */

function offlineBench(host: HTMLElement, onWin: () => void, _reduce: boolean): void {
  const out = sayEl(host);
  tell(out, 'Your wifi is about to drop. Pack only the things that still work offline — tap each item to check it.');
  const tray = document.createElement('div');
  tray.className = 'ws-tray';
  tray.innerHTML = '<h4>Your pack</h4>';
  host.appendChild(tray);

  const items = [
    { id: 'catalog', name: 'Installed mods', note: 'already on disk', worksOffline: true },
    { id: 'versions', name: 'Installer files', note: 'need to be downloaded first', worksOffline: false },
    { id: 'worlds', name: 'Your worlds', note: 'on this computer', worksOffline: true },
    { id: 'accounts', name: 'Online sign-in', note: 'needs a connection', worksOffline: false },
    { id: 'launcher', name: 'The launcher', note: 'runs locally', worksOffline: true },
    { id: 'updates', name: 'Fresh updates', note: 'arrive over the internet', worksOffline: false },
  ];
  const state: Record<string, 'untouched' | 'correct' | 'wrong'> = {};
  items.forEach((it) => { state[it.id] = 'untouched'; });

  items.forEach((it) => {
    const el = document.createElement('button');
    el.className = 'ws-piece';
    el.style.width = '112px';
    el.innerHTML = '<canvas></canvas><b></b><small></small>';
    el.querySelector('b')!.textContent = it.name;
    el.querySelector('small')!.textContent = it.note;
    tray.appendChild(el);
    requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, ART.box(it.worksOffline ? '#8BE24F' : '#FF7A6B', it.worksOffline ? 'tab' : undefined)));
    el.onclick = () => {
      const guessOffline = it.worksOffline;
      if (guessOffline) {
        state[it.id] = 'correct';
        el.classList.add('ok');
        blip(880, 0.12);
        tell(out, 'Right — ' + it.name + ' is already here, so it works with no internet.', 'good');
      } else {
        state[it.id] = 'wrong';
        el.classList.add('bad');
        nope();
        tell(out, 'Not quite — ' + it.name + ' ' + it.note + '. It can\'t work offline.', 'bad');
        setTimeout(() => el.classList.remove('bad'), 340);
      }
      const allCorrect = items.filter((x) => x.worksOffline).every((x) => state[x.id] === 'correct');
      if (allCorrect) {
        blip(880, 0.14);
        const c = centerOf(el);
        burst(c[0], c[1], '#8BE24F', 34, 11);
        yay();
        tell(out, 'That\'s the offline pack: what\'s already on this computer. Downloads and sign-ins wait for wifi.', 'good');
        onWin();
      }
    };
  });
  moreInfo(host, 'A cached catalog is not the same as ready-to-go: Agora can <i>show</i> you content it has not downloaded. Offline readiness means the files you need are already on disk.');
}

/* ── Undo it: undoBench ── */

function undoBench(host: HTMLElement, onWin: () => void, _reduce: boolean): void {
  const out = sayEl(host);
  tell(out, 'You changed two things and now it broke. Use the return points to go back in time.');
  const row = document.createElement('div');
  row.className = 'ws-row';
  host.appendChild(row);

  // timeline of snapshots (fictional but honest about what a snapshot covers)
  const points = [
    { id: 'today', name: 'Just now', note: 'after the change', ok: false },
    { id: 'before', name: 'Before the change', note: 'saved this morning', ok: true },
    { id: 'known', name: 'Known good', note: 'two days ago', ok: true },
  ];
  const timeline = document.createElement('div');
  timeline.className = 'ws-tray';
  timeline.innerHTML = '<h4>Return points</h4>';
  timeline.style.flex = '1 1 100%';
  host.appendChild(timeline);
  const state: Record<string, boolean> = {};

  points.forEach((p) => {
    const el = document.createElement('button');
    el.className = 'ws-piece';
    el.style.width = '118px';
    el.innerHTML = '<canvas></canvas><b></b><small></small>';
    el.querySelector('b')!.textContent = p.name;
    el.querySelector('small')!.textContent = p.note;
    timeline.appendChild(el);
    requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, ART.undo));
    el.onclick = () => {
      if (state[p.id]) return;
      if (!p.ok) {
        nope();
        tell(out, 'That point is AFTER the break — restoring it brings the problem back. Pick an earlier one.', 'bad');
        return;
      }
      state[p.id] = true;
      el.classList.add('ok');
      blip(880, 0.14);
      const c = centerOf(el);
      burst(c[0], c[1], '#6FD3E8', 22, 9);
      if (p.id === 'before') {
        tell(out, 'Restored to before the change — the break is gone, and nothing newer is lost. A snapshot covers the mods, versions, and settings.', 'good');
        onWin();
      } else {
        tell(out, 'Known good restored. Going further back would also throw away the work you did since. "Before the change" was the right distance.', 'good');
        onWin();
      }
    };
  });
  moreInfo(host, 'A <code>snapshot</code> records which mods were present, their versions, and your settings. It is not a copy of your worlds — those are protected separately.');
}

/* ── The six benches ── */

const GUIDE_TOPIC: Record<string, string> = {
  build: 'modding-foundations',
  mod: 'content-management',
  fix: 'crash-recovery',
  heal: 'launching',
  offline: 'privacy-offline',
  undo: 'snapshots-loadouts',
};

const BENCHES: WorkshopBenchWithBuild[] = [
  {
    id: 'build',
    title: 'Build it',
    why: 'Two pieces have to agree before anything works. You can see it when they snap.',
    steps: [
      {
        id: 'do', kind: 'do', title: 'Try it',
        lead: 'Drag a game version into your world, then try a loader on top. Try a wrong one on purpose — it\'s the fastest way to see the rule.',
        build: (host, done) => {
          worldBench(host, {
            hint: 'Start with a game version.',
            win: (sp) => sp.id === 'none'
              ? 'That works. No loader means no add-ons — a plain game. Perfectly fine.'
              : 'Snap! ' + sp.name + ' agrees with this version. Now your world can take add-ons.',
          }, done);
          moreInfo(host, 'A world is an <code>instance</code>. The game version and the <code>mod loader</code> have to match.');
        },
      },
      {
        id: 'predict', kind: 'predict', title: 'Guess first',
        lead: 'Your world is Game 1.20.1 with Loader A. A friend sends you a mod built for Game 1.21.',
        build: (host, done) => {
          askStep(host, {
            options: [
              { t: 'It works fine — mods don\'t really care about versions.', why: 'Mods hook directly into the game\'s internals, and those move between versions.' },
              { t: 'It refuses to go in — it needs the version it was built for.', right: true, why: 'A mod is built against one version\'s internals. Change the version and the hooks it expects aren\'t there any more.' },
              { t: 'It works, but the game runs slower.', why: 'A mismatch isn\'t a performance problem — the code it\'s looking for simply isn\'t there.' },
            ],
          }, done);
        },
      },
      {
        id: 'transfer', kind: 'transfer', title: 'Work backwards',
        lead: 'Harder: you\'re handed the mod first. Build a world that fits it — you\'ll have to figure out which version to start from.',
        build: (host, done) => {
          worldBench(host, {
            slotLabel: 'Build a world for this mod',
            goal: { name: 'Notebot', note: 'built for Game 1.21', art: ART.box('#B48CF2', 'tab') },
            needVersion: 'v121',
            needLoaderNotNone: true,
            hint: 'The mod can\'t change. Everything else can. Work out what it needs.',
            win: () => 'Exactly. You read the mod\'s requirement, picked the version to match, then picked the loader to match that. That\'s the order it always goes in.',
          }, done);
          moreInfo(host, 'Working backwards from a requirement is what Agora does for you when you install something — it checks the chain before it stages anything.');
        },
      },
      {
        id: 'explain', kind: 'explain', title: 'Why',
        lead: 'Now the actual mechanism.',
        build: (host, done, _reduce, openGuide) => {
          explainStep(host,
            '<h4>Why loaders are tied to a version</h4>' +
            '<p>Minecraft doesn\'t have a plug-in system. A <b>mod loader</b> adds one by patching the game\'s own code as it starts. Those patches aim at specific spots inside a specific build — so when the game updates and the code moves, the patches miss.</p>' +
            '<h4>"Loader A" and "Loader B" aren\'t real</h4>' +
            '<p>They\'re stand-ins, so the bench can show you a loader that doesn\'t fit without pretending that\'s a fact about anybody\'s actual software. The real ones you\'ll meet are <b>Fabric</b>, <b>Forge</b>, <b>NeoForge</b> and <b>Quilt</b>. Which versions each one supports changes over time, so check the loader\'s own page rather than trusting a lesson.</p>' +
            '<h4>What the numbers mean</h4>' +
            '<ul><li><b>1.20.1</b> and <b>1.21</b> are different builds, not just bigger numbers.</li>' +
            '<li>A loader release is published for particular game versions.</li>' +
            '<li>A mod is published for a particular loader on a particular version. Three things that all have to line up.</li></ul>' +
            '<h4>What happens if they don\'t</h4>' +
            '<p>The game usually stops during startup and writes a <b>crash report</b> naming what it couldn\'t find. That\'s the bench next door.</p>' +
            '<h4>What Agora does about it</h4>' +
            '<p>It won\'t let you build the mismatched combination in the first place, and the health check runs before launch rather than after.</p>',
            done, GUIDE_TOPIC.build, openGuide);
        },
      },
    ],
  },
  {
    id: 'mod',
    title: 'Add stuff',
    why: 'Some add-ons need a friend to work, and some refuse to sit next to each other.',
    steps: [
      {
        id: 'do', kind: 'do', title: 'Try it',
        lead: 'Drag Better Caves in and watch what arrives with it. Then try Terrain Redo.',
        build: (host, done, reduce) => { modBench(host, { mode: 'basic' }, done, reduce); },
      },
      {
        id: 'predict', kind: 'predict', title: 'Guess first',
        lead: 'Better Caves is installed, and Core Helper came along with it. You take Core Helper back out.',
        build: (host, done) => {
          askStep(host, {
            options: [
              { t: 'Nothing — Better Caves is already installed, so it\'s fine.', why: 'Installing isn\'t copying. The helper has to be there every time the game runs, not just once.' },
              { t: 'Better Caves stops working — it needed that helper.', right: true, why: 'Better Caves calls code that lives inside Core Helper. Remove it and those calls hit nothing.' },
              { t: 'It finds another helper automatically.', why: 'Nothing goes looking. A mod names the exact helper it needs; there\'s no substitute.' },
            ],
          }, done);
        },
      },
      {
        id: 'transfer', kind: 'transfer', title: 'Two at once',
        lead: 'Now two different mods both need the same helper. Add both, then take them out one at a time and watch what the helper does.',
        build: (host, done, reduce) => { modBench(host, { mode: 'shared' }, done, reduce); },
      },
      {
        id: 'explain', kind: 'explain', title: 'Why',
        lead: 'The rules underneath.',
        build: (host, done, _reduce, openGuide) => {
          explainStep(host,
            '<h4>Dependencies</h4>' +
            '<p>A mod can say <i>"I need this other mod present"</i>. Shared code gets published once as a library instead of copied into every mod that wants it — smaller downloads, one place to fix bugs. The cost is that the library has to actually be there.</p>' +
            '<h4>Why the helper stayed</h4>' +
            '<p>When two mods need the same helper, it isn\'t removed until the last one that needs it is gone. Agora counts what still depends on it rather than removing it the moment you delete one thing.</p>' +
            '<h4>Version ranges</h4>' +
            '<p>Dependencies usually name a range — <i>"Core Helper 2.x"</i> — not one exact build. So an update inside the range is fine and a jump outside it is not, which is why an update can break a mod that you didn\'t touch.</p>' +
            '<h4>Conflicts</h4>' +
            '<p>The opposite problem. Two mods that rewrite the <i>same</i> part of the game can\'t both win. Terrain Redo and Better Caves both rewrite cave generation, so one would silently overwrite the other — usually as corrupted terrain rather than a clean error. Refusing the pair is the safer answer.</p>',
            done, GUIDE_TOPIC.mod, openGuide);
        },
      },
    ],
  },
  {
    id: 'fix',
    title: 'Something broke',
    why: 'You don\'t guess. You try one thing at a time, and you can always undo it.',
    steps: [
      {
        id: 'do', kind: 'do', title: 'Try it',
        lead: 'Something crashed. Read every clue before you guess, then test your guess safely.',
        build: (host, done, reduce) => { fixBench(host, { mode: 'basic' }, done, reduce); },
      },
      {
        id: 'predict', kind: 'predict', title: 'Guess first',
        lead: 'You think a particular mod is the cause. You\'re about to turn it off and launch.',
        build: (host, done) => {
          askStep(host, {
            options: [
              { t: 'If it starts up fine, that proves I was right.', why: 'It\'s evidence, but it isn\'t proof — turning off a mod can hide a problem instead of fixing it.' },
              { t: 'If it still crashes, my guess was wrong.', right: true, why: 'That\'s the useful half. A test worth running is one that can come back and tell you no — and ruling a suspect out is real progress, not a wasted go.' },
              { t: 'Either way I\'ve learned nothing until it works.', why: 'A failed test narrows the list. That\'s exactly what you\'re doing here.' },
            ],
          }, done);
        },
      },
      {
        id: 'transfer', kind: 'transfer', title: 'A trickier one',
        lead: 'New crash. This time the message names a mod — but read the clues carefully before you trust it.',
        build: (host, done, reduce) => { fixBench(host, { mode: 'messenger' }, done, reduce); },
      },
      {
        id: 'explain', kind: 'explain', title: 'Why',
        lead: 'How this works when it\'s real.',
        build: (host, done, _reduce, openGuide) => {
          explainStep(host,
            '<h4>The name in the error isn\'t always the cause</h4>' +
            '<p>A crash report names the code that was running when things stopped. That\'s often the mod that <i>noticed</i> the problem, not the one that caused it — a mod falls over because the helper it expected was missing, and its name is the one you see.</p>' +
            '<h4>Change one thing</h4>' +
            '<p>Change two and a fix tells you nothing: you can\'t tell which one did it, or whether one fixed it while the other broke something new. One change, one launch, one answer. It feels slower and is much faster.</p>' +
            '<h4>What a return point actually saves</h4>' +
            '<p>A <b>snapshot</b> records which mods were present, their versions, and your settings — so undoing an experiment restores the exact combination, not an approximation. Agora takes one before it runs an experiment, which is what makes it safe to be wrong.</p>' +
            '<h4>Confidence, not certainty</h4>' +
            '<p>The bars say likely, maybe, unlikely — never a percentage. A number like "80%" claims a precision nobody has. And one good run isn\'t proof: intermittent crashes are the hard ones precisely because a single success looks like a fix.</p>',
            done, GUIDE_TOPIC.fix, openGuide);
        },
      },
    ],
  },
  {
    id: 'heal',
    title: 'Health check',
    why: 'Catching it early is easier than fixing it later.',
    steps: [
      {
        id: 'do', kind: 'do', title: 'Try it',
        lead: 'Run the scan, read what it finds, and pick a loader that fits.',
        build: (host, done, reduce) => { healthBench(host, done, reduce); },
      },
      {
        id: 'predict', kind: 'predict', title: 'Guess first',
        lead: 'The scan finds a blocker: the current loader does not fit two installed mods.',
        build: (host, done) => {
          askStep(host, {
            options: [
              { t: 'Any loader will do — the health check is just a suggestion.', why: 'A blocker stops launch. The check is the gate, not advice.' },
              { t: 'Only a proven-compatible loader clears it; an unknown one needs review first.', right: true, why: 'Proven-compatible is different from needs-review. An indeterminate loader can\'t clear a blocker until someone checks which loaders each mod supports.' },
              { t: 'Keep the current loader and hope it works.', why: 'The check already knows it doesn\'t fit — "hope" is not a compatibility answer.' },
            ],
          }, done);
        },
      },
      {
        id: 'transfer', kind: 'transfer', title: 'The warning',
        lead: 'The blocker is gone. Now a warning appears: Java 8 is too old for this Minecraft version.',
        build: (host, done) => {
          const out = sayEl(host);
          tell(out, 'A warning needs a choice — it does not stop launch. Keep Java 8, or let Agora manage Java 17.');
          const row = document.createElement('div');
          row.className = 'ws-row';
          host.appendChild(row);
          const keep = document.createElement('button');
          keep.className = 'ws-btn';
          keep.textContent = 'Keep Java 8';
          const manage = document.createElement('button');
          manage.className = 'ws-btn p';
          manage.textContent = 'Let Agora manage Java 17';
          row.appendChild(keep);
          row.appendChild(manage);
          manage.onclick = () => {
            blip(880, 0.14);
            tell(out, 'Warning resolved — Java 17 is managed. Launch is green.', 'good');
            done();
          };
          keep.onclick = () => {
            blip(440, 0.1);
            tell(out, 'Java 8 stays — the warning remains. You can proceed, but launch may fail. A kept warning stays until you resolve it.', 'bad');
            manage.disabled = false;
            done();
          };
          moreInfo(host, 'Blockers stop launch. Warnings need a decision. Recommendations are advice. The bars say likely, maybe, unlikely — never a percentage.');
        },
      },
      {
        id: 'explain', kind: 'explain', title: 'Why',
        lead: 'What the scan is actually doing.',
        build: (host, done, _reduce, openGuide) => {
          explainStep(host,
            '<h4>Three kinds of finding</h4>' +
            '<p>A <b>blocker</b> stops launch until it is resolved. A <b>warning</b> needs your decision — keeping it is a valid choice, but it stays. A <b>recommendation</b> never blocks; it is advice.</p>' +
            '<h4>Proven vs needs-review</h4>' +
            '<p>A loader is <b>proven-compatible</b> when every installed mod\'s own metadata agrees. It is <b>indeterminate</b> when some mods have not declared support — which is not a "no", but it is not a yes either. A blocker cannot be cleared by an indeterminate answer.</p>' +
            '<h4>Why the check runs before launch</h4>' +
            '<p>Every mismatch this finds is one that would otherwise surface as a crash at startup — with a report to read and a bench to sit at. The check is the earlier, cheaper place to catch it.</p>',
            done, GUIDE_TOPIC.heal, openGuide);
        },
      },
    ],
  },
  {
    id: 'offline',
    title: 'Going offline',
    why: 'Know what still works when the wifi drops.',
    steps: [
      {
        id: 'do', kind: 'do', title: 'Try it',
        lead: 'The wifi is about to drop. Tap each item to decide whether it belongs in your offline pack.',
        build: (host, done, reduce) => { offlineBench(host, done, reduce); },
      },
      {
        id: 'predict', kind: 'predict', title: 'Guess first',
        lead: 'You saw a mod in the catalog yesterday, but you never downloaded it. The wifi drops now.',
        build: (host, done) => {
          askStep(host, {
            options: [
              { t: 'It\'s fine — it was in the catalog, so it\'s basically installed.', why: 'A cached catalog is a list, not a copy. Showing you something exists is not the same as having its files.' },
              { t: 'It won\'t launch offline — the files were never downloaded.', right: true, why: 'Offline readiness means the exact files you need are already on disk. If it was never downloaded, the wifi drop is the difference between "see it" and "run it".' },
              { t: 'It will launch, just without updates.', why: 'Launch needs the files themselves. "Without updates" only matters when the files are already there.' },
            ],
          }, done);
        },
      },
      {
        id: 'transfer', kind: 'transfer', title: 'Pack a trip',
        lead: 'You\'re going somewhere with no internet for a week. Pick what makes sense to take — and what will have to wait.',
        build: (host, done) => {
          const out = sayEl(host);
          tell(out, 'Sort the list: tap what works offline. One wrong pick and the pack is lying to you.');
          const tray = document.createElement('div');
          tray.className = 'ws-tray';
          tray.innerHTML = '<h4>What you\'re carrying</h4>';
          host.appendChild(tray);
          const items = [
            { id: 'a', name: 'Saved worlds', note: 'on this computer', ok: true },
            { id: 'b', name: 'A mod you downloaded', note: 'files are here', ok: true },
            { id: 'c', name: 'A mod you only saw in the catalog', note: 'not downloaded', ok: false },
            { id: 'd', name: 'Account sign-in', note: 'needs a server', ok: false },
          ];
          const state: Record<string, boolean> = {};
          items.forEach((it) => {
            const el = document.createElement('button');
            el.className = 'ws-piece';
            el.style.width = '112px';
            el.innerHTML = '<canvas></canvas><b></b><small></small>';
            el.querySelector('b')!.textContent = it.name;
            el.querySelector('small')!.textContent = it.note;
            tray.appendChild(el);
            requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, ART.box(it.ok ? '#8BE24F' : '#FF7A6B', it.ok ? 'tab' : undefined)));
            el.onclick = () => {
              if (state[it.id]) return;
              if (it.ok) {
                state[it.id] = true;
                el.classList.add('ok');
                blip(880, 0.12);
              } else {
                nope();
                tell(out, it.name + ' ' + it.note + ' — it waits for wifi.', 'bad');
              }
              const doneAll = items.filter((x) => x.ok).every((x) => state[x.id]);
              if (doneAll) {
                tell(out, 'The offline pack is exactly what is already on this computer. Anything the internet had to bring is left behind.', 'good');
                done();
              }
            };
          });
        },
      },
      {
        id: 'explain', kind: 'explain', title: 'Why',
        lead: 'What "offline-ready" actually means.',
        build: (host, done, _reduce, openGuide) => {
          explainStep(host,
            '<h4>Catalog vs files</h4>' +
            '<p>A <b>cached catalog</b> is a list of what exists. It is not a copy of the files. Offline readiness is about the files: if the exact artifact is on disk, it runs; if only its name is in a list, it waits.</p>' +
            '<h4>What still works</h4>' +
            '<p>Launching, playing, and managing what is already installed all work with no connection. Signing in, downloading, checking for updates, and fetching anything new do not.</p>' +
            '<h4>Why Agora separates them</h4>' +
            '<p>Conflating "I can see it" with "I have it" is exactly how an offline trip disappoints. The readiness check asks the same question the pack does: are the files here?</p>',
            done, GUIDE_TOPIC.offline, openGuide);
        },
      },
    ],
  },
  {
    id: 'undo',
    title: 'Undo it',
    why: 'A save point means nothing is ever permanent.',
    steps: [
      {
        id: 'do', kind: 'do', title: 'Try it',
        lead: 'You changed two things and now it broke. Pick the return point that takes you back to before the trouble.',
        build: (host, done, reduce) => { undoBench(host, done, reduce); },
      },
      {
        id: 'predict', kind: 'predict', title: 'Guess first',
        lead: 'A restore point is about to put your instance back. What does it restore?',
        build: (host, done) => {
          askStep(host, {
            options: [
              { t: 'Everything, including your worlds.', why: 'A snapshot of the instance covers the mods, versions, and settings — your worlds are protected separately, not inside it.' },
              { t: 'The mods, versions, and settings — not your worlds.', right: true, why: 'That is exactly the boundary. Restoring an experiment should never touch the worlds you built in it.' },
              { t: 'Just the launcher itself.', why: 'The launcher is not the part that changes when you add or remove mods.' },
            ],
          }, done);
        },
      },
      {
        id: 'transfer', kind: 'transfer', title: 'Choose the distance',
        lead: 'Three return points exist. One is after the break, one is right before it, one is much older. Which restores your work without losing it?',
        build: (host, done) => {
          const out = sayEl(host);
          tell(out, 'Tap a return point. The right one is the smallest step that fixes it.');
          const tray = document.createElement('div');
          tray.className = 'ws-tray';
          tray.innerHTML = '<h4>Return points</h4>';
          host.appendChild(tray);
          const points = [
            { id: 'after', name: 'After the break', note: 'restores the problem', ok: false },
            { id: 'before', name: 'Right before', note: 'keeps your work', ok: true },
            { id: 'ancient', name: 'Much older', note: 'throws away more than needed', ok: true, tooFar: true },
          ];
          const state: Record<string, boolean> = {};
          points.forEach((p) => {
            const el = document.createElement('button');
            el.className = 'ws-piece';
            el.style.width = '118px';
            el.innerHTML = '<canvas></canvas><b></b><small></small>';
            el.querySelector('b')!.textContent = p.name;
            el.querySelector('small')!.textContent = p.note;
            tray.appendChild(el);
            requestAnimationFrame(() => paint(el.querySelector('canvas') as HTMLCanvasElement, ART.undo));
            el.onclick = () => {
              if (state[p.id]) return;
              if (!p.ok) {
                nope();
                tell(out, 'That point is after the break — it brings the problem back.', 'bad');
                return;
              }
              state[p.id] = true;
              el.classList.add('ok');
              blip(880, 0.14);
              if (p.tooFar) {
                tell(out, 'It works, but you threw away more than you needed. "Right before" was the smallest fix. Either way, nothing is permanent.', 'good');
              } else {
                tell(out, 'Restored to right before the change — the break is gone and the work stays. The smallest step that fixes it is the right one.', 'good');
              }
              done();
            };
          });
        },
      },
      {
        id: 'explain', kind: 'explain', title: 'Why',
        lead: 'What a save point is — and is not.',
        build: (host, done, _reduce, openGuide) => {
          explainStep(host,
            '<h4>What a snapshot covers</h4>' +
            '<p>A <b>snapshot</b> records which mods were present, their versions, and your settings. Restoring it puts the instance back to that exact combination.</p>' +
            '<h4>What it does not cover</h4>' +
            '<p>Your worlds are protected separately — a snapshot is not a backup of them. That separation is deliberate: experiments should be safe to undo without ever touching what you built inside them.</p>' +
            '<h4>The right distance</h4>' +
            '<p>The best restore point is the smallest step that fixes the problem. Going further back than you need to throws away work for no reason; going back to after the break fixes nothing.</p>',
            done, GUIDE_TOPIC.undo, openGuide);
        },
      },
    ],
  },
];

type BenchStepBuild = (host: HTMLElement, done: () => void, reduce: boolean, openGuide?: (t: string) => void) => void;

/* The bench steps carry the prototype's imperative `build` on top of the model's
   pure step data. WorkshopBenchWithBuild is structurally compatible with the
   model's WorkshopBench, so progress helpers accept it unchanged. */
type BenchStepWithBuild = {
  id: string;
  kind: 'do' | 'predict' | 'transfer' | 'explain';
  title: string;
  lead: string;
  build: BenchStepBuild;
};
type WorkshopBenchWithBuild = Omit<WorkshopBench, 'steps'> & { steps: BenchStepWithBuild[] };

export function Workshop({ onOpenGuide, onNavigateStandard, onAmbienceChange, reducedMotion = false, soundOn = true }: WorkshopProps) {
  void onNavigateStandard;
  const reduce = reducedMotion;
  const [progress, setProgress] = useState<ProgressMap>(() => loadProgress());
  const [activeBench, setActiveBench] = useState<string | null>(null);
  const [curStep, setCurStep] = useState(0);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [badge, setBadge] = useState<string | null>(null);
  const [sayText, setSayText] = useState('');
  const sayTimer = useRef<number | null>(null);

  useEffect(() => { syncWorkshopSound(); if (soundOn) syncWorkshopSound(); }, [soundOn]);

  useEffect(() => {
    if (!badge) return;
    if (sayTimer.current !== null) window.clearTimeout(sayTimer.current);
    sayTimer.current = window.setTimeout(() => setBadge(null), 3200);
    return () => { if (sayTimer.current !== null) window.clearTimeout(sayTimer.current); };
  }, [badge]);

  const bench: WorkshopBenchWithBuild | null = activeBench ? BENCHES.find((b) => b.id === activeBench) ?? null : null;

  const openBench = useCallback((id: string, step?: number) => {
    const b = BENCHES.find((x) => x.id === id);
    if (!b) return;
    setActiveBench(id);
    setCurStep(step !== undefined ? step : firstUndone(loadProgress(), b));
    blip(620, 0.1);
    onAmbienceChange?.('calm');
  }, [onAmbienceChange]);

  const closeBench = useCallback(() => {
    setActiveBench(null);
    setSayText('');
    onAmbienceChange?.(null);
  }, [onAmbienceChange]);

  const markDone = useCallback((benchId: string, stepIndex: number) => {
    const b = BENCHES.find((x) => x.id === benchId);
    if (!b) return;
    setProgress((cur) => {
      const { next, completed } = markStepDone(cur, b, stepIndex);
      saveProgress(next);
      if (completed) {
        const station = STATIONS.find((s) => s.id === benchId);
        setBadge((station?.badge ?? '🎖️') + ' ' + b.title);
        yay();
        burst(window.innerWidth / 2, window.innerHeight * 0.35, '#8BE24F', 44, 12);
        burst(window.innerWidth / 2, window.innerHeight * 0.35, '#FFD34E', 26, 15);
      }
      return next;
    });
    // Completing a step never moves you on by itself. The reveal, the snap, the
    // explanation of why the other answers were wrong — those are the teaching,
    // and yanking the page out from under someone mid-read destroys them. The
    // learner presses Next when they are done reading.
    // The exception is the final step, whose own button is the exit.
    if (stepIndex >= b.steps.length - 1) {
      setTimeout(() => closeBench(), reduce ? 0 : 900);
    }
  }, [reduce, closeBench]);

  // Build the active step's body imperatively (the prototype's exact approach).
  useEffect(() => {
    const host = bodyRef.current;
    if (!host || !bench) return;
    host.innerHTML = '';
    const step = bench.steps[curStep] as BenchStepWithBuild;
    const isLast = curStep >= bench.steps.length - 1;
    const done = () => {
      markDone(bench.id, curStep);
      // The last step supplies its own exit ("Got it — back to the workshop"),
      // so only the earlier ones get a Next.
      if (isLast || host.querySelector('.ws-next')) return;
      const row = document.createElement('div');
      row.className = 'ws-acts ws-next';
      const b = document.createElement('button');
      b.className = 'ws-btn p';
      b.textContent = 'Next: ' + bench.steps[curStep + 1].title;
      b.onclick = () => setCurStep(curStep + 1);
      row.appendChild(b);
      host.appendChild(row);
      b.focus();
    };
    const openGuide = (topic: string) => { if (onOpenGuide) onOpenGuide(topic); };
    step.build(host, done, reduce, openGuide);
    // sync the say line with the aria-live region
    const observer = new MutationObserver(() => {
      const say = host.querySelector('.ws-say');
      if (say) setSayText(say.textContent ?? '');
    });
    observer.observe(host, { childList: true, subtree: true });
    setSayText('');
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bench, curStep]);

  const benchById = (id: string) => BENCHES.find((b) => b.id === id) ?? null;

  // Paint the station-map art canvases (they were declared but never painted —
  // the icons are part of the tiles) and re-paint as the grid resizes so the
  // art scales with the tile instead of staying a fixed 96px strip. Painted
  // synchronously (paint() reads getBoundingClientRect) — never inside a
  // requestAnimationFrame, which pauses when the page is hidden and would
  // leave the tiles blank.
  useEffect(() => {
    const paintStations = () => {
      document.querySelectorAll<HTMLCanvasElement>('.ws-station-art[data-art]').forEach((c) => {
        const station = STATIONS.find((s) => s.id === c.dataset.art);
        if (!station) return;
        paint(c, ART_BY_STATION[station.id]);
      });
    };
    paintStations();
    const t = window.setTimeout(paintStations, 60);
    // A window resize is not the only thing that changes a tile's width — the
    // sidebar collapsing, a scrollbar appearing, or the grid reflowing all do it
    // without firing `resize`, which left the art at its old bitmap size and
    // visibly out of scale. Observe the elements themselves instead.
    let ro: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver(() => paintStations());
      document.querySelectorAll('.ws-station-art[data-art]').forEach((c) => ro!.observe(c));
    }
    window.addEventListener('resize', paintStations);
    return () => {
      window.clearTimeout(t);
      ro?.disconnect();
      window.removeEventListener('resize', paintStations);
    };
  }, [progress, activeBench]);

  return (
    <div className="ws-wrap" data-testid="workshop">
      <canvas className="ws-fx" aria-hidden="true" // `100vw` INCLUDES the vertical scrollbar's width, so as soon as the page
      // scrolled this fixed canvas was wider than the content box and produced a
      // horizontal scrollbar of its own. `inset: 0` already sizes it correctly.
      style={{ position: 'fixed', inset: 0, zIndex: 60, pointerEvents: 'none', width: '100%', height: '100%' }} />

      <header className="ws-header agora-hero compact">
        <h1>The Workshop</h1>
        <span className="ws-sub">Six benches. Pick things up, try them, see what happens.</span>
        <div className="ws-stamps" aria-label="Badges earned">
          {STATIONS.map((st) => {
            const b = benchById(st.id);
            const full = b ? benchComplete(progress, b) : false;
            return (
              <div key={st.id} className={`ws-stamp ${full ? 'on' : ''}`} title={st.title + (b ? ` (${stepsDone(progress, b)}/${stepCount(b)})` : '')}>
                {st.badge}
              </div>
            );
          })}
        </div>
      </header>

      <div className="ws-map" data-testid="ws-map">
        {STATIONS.map((st) => {
          const b = benchById(st.id);
          const n = b ? stepsDone(progress, b) : 0;
          const tot = b ? stepCount(b) : 0;
          const full = b ? benchComplete(progress, b) : false;
          return (
            <button
              key={st.id}
              type="button"
              className="ws-station"
              onClick={() => openBench(st.id)}
              onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openBench(st.id); } }}
            >
              <canvas className="ws-station-art" data-art={st.id} aria-hidden="true" />
              <h3>{st.title}</h3>
              <p>{st.blurb}</p>
              <span className="ws-go">{full ? 'Do it again' : n ? 'Continue' : 'Start'}</span>
              <span className="ws-prog" title={`${n} of ${tot} steps done`}>
                {Array.from({ length: tot }).map((_, i) => (
                  <span key={i} className={`ws-dot ${i < n ? 'on' : ''}`} />
                ))}
              </span>
              {full ? <span className="ws-done">{st.badge}</span> : null}
            </button>
          );
        })}
      </div>

      <p className="ws-sub" style={{ marginTop: 18 }}>
        Nothing here touches your real game. Everything is a pretend copy you can break on purpose.
      </p>

      {/* bench modal */}
      {/* controller-exempt: click-outside backdrop; the card has its own Close button. */}
      <div className={`ws-bench ${bench ? 'show' : ''}`} role="dialog" aria-modal="true" aria-label={bench?.title} onClick={(e) => { if (e.target === e.currentTarget) closeBench(); }}>
        {bench ? (
          <div className="ws-card">
            <button type="button" className="ws-close" onClick={closeBench}>Close</button>
            <h2>{bench.title}</h2>
            <p className="why">{bench.why}</p>
            <div className="ws-rail">
              {bench.steps.map((s, i) => {
                const open = i === 0 || !!progress[bench.id]?.[bench.steps[i - 1].id];
                const done = !!progress[bench.id]?.[s.id];
                return (
                  <button
                    key={s.id}
                    type="button"
                    className={`ws-pip ${i === curStep ? 'now' : ''} ${done ? 'ok' : ''} ${open ? '' : 'lock'}`}
                    disabled={!open}
                    onClick={() => setCurStep(i)}
                  >
                    <u>{i + 1}</u>
                    {s.title}
                    {done ? ' ✓' : open ? '' : ' 🔒'}
                  </button>
                );
              })}
            </div>
            <p className="ws-lead">{bench.steps[curStep].lead}</p>
            <div ref={bodyRef} className="ws-bench-body" />
          </div>
        ) : null}
      </div>

      {/* aria-live for the say line */}
      <div aria-live="polite" style={{ position: 'absolute', width: 1, height: 1, overflow: 'hidden', clip: 'rect(0 0 0 0)', whiteSpace: 'nowrap' }}>
        {sayText}
      </div>

      {/* badge toast */}
      {badge ? (
        <div className="ws-toast" role="status" data-testid="ws-badge">
          <span className="ws-ach-ico">🎖️</span>
          <span><span className="t">Badge earned</span><br /><span className="d">{badge}</span></span>
        </div>
      ) : null}
    </div>
  );
}
