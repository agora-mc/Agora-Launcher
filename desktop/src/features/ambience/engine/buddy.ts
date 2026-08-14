/**
 * The cursor companion ("buddy").
 *
 * Drawn on the FX canvas by the engine rather than in the React tree, because
 * it moves every frame.
 *
 * It is deliberately more than a sprite that lerps toward the pointer. A thing
 * reads as *alive* when its motion has consequences: it leans into a chase,
 * squashes when it lands, overshoots and settles, looks where it is going, and
 * does something of its own when ignored. All of that is cheap here — a handful
 * of scalars integrated per frame — and it is the difference between a cursor
 * decoration and a companion.
 */

export interface BuddyState {
  x: number;
  y: number;
  tx: number;
  ty: number;
  on: boolean;
  reduced: boolean;
  /** Horizontal facing: 1 right, -1 left. Eased, so turns are not instant. */
  face: number;
  /** Smoothed speed, drives lean/stretch and how hard it hops. */
  speed: number;
  /** Hop phase in radians; advances faster the quicker it travels. */
  hop: number;
  /** Vertical offset from the hop, in px (0 = on the ground). */
  bob: number;
  /** Squash (<1 = flattened, >1 = stretched). Springs back to 1. */
  squash: number;
  /** Seconds the pointer has been still. Drives the idle behaviours. */
  idle: number;
  /** Where the eyes are looking, -1..1 on each axis. */
  lookX: number;
  lookY: number;
  /** Countdown to the next idle flourish. */
  nextPerk: number;
  /** Active flourish, if any. */
  perk: 'none' | 'jump' | 'spin';
  perkT: number;
  lastMs: number;
}

export function createBuddy(reduced: boolean, w: number, h: number): BuddyState {
  return {
    x: w / 2, y: h / 2, tx: w / 2, ty: h / 2,
    on: true, reduced,
    face: 1, speed: 0, hop: 0, bob: 0, squash: 1,
    idle: 0, lookX: 0, lookY: 0,
    nextPerk: 4 + Math.random() * 4, perk: 'none', perkT: 0,
    lastMs: 0,
  };
}

/**
 * Teleport the companion to the cursor instead of easing toward it.
 *
 * Used when the pointer has been away — off-window, or on another surface. The
 * buddy eases toward its target, which is right for following a moving cursor
 * but wrong after a gap: it would crawl across the whole screen from wherever it
 * was stranded, reading as broken rather than lively.
 */
export function buddySnap(s: BuddyState, cx: number, cy: number): void {
  s.tx = cx + 26;
  s.ty = cy + 26;
  s.x = s.tx;
  s.y = s.ty;
  s.speed = 0;
  s.idle = 0;
  s.perk = 'none';
}

export function buddyTarget(s: BuddyState, cx: number, cy: number): void {
  // Trailing behind and below reads as "following", not "attached to".
  const nx = cx + 26;
  const ny = cy + 26;
  if (Math.abs(nx - s.tx) > 0.5 || Math.abs(ny - s.ty) > 0.5) {
    s.idle = 0;
    s.perk = 'none';
  }
  s.tx = nx;
  s.ty = ny;
}

export function buddyVisible(s: BuddyState, visible: boolean): void {
  s.on = visible;
}

/** Advance one frame. Returns true when it should be drawn. */
export function buddyStep(s: BuddyState, now: number): boolean {
  if (!s.on || s.reduced) return false;

  const dt = s.lastMs ? Math.min(0.05, (now - s.lastMs) / 1000) : 1 / 60;
  s.lastMs = now;

  const dx = s.tx - s.x;
  const dy = s.ty - s.y;
  const dist = Math.hypot(dx, dy);

  // Chase harder the further behind it is, so it scurries to catch up on a
  // fast flick but drifts gently on a small move.
  const pull = 0.06 + Math.min(0.12, dist / 900);
  s.x += dx * pull;
  s.y += dy * pull;

  const vx = dx * pull;
  const inst = dist * pull;
  s.speed += (inst - s.speed) * 0.18;

  // Turn to face travel, but only when actually moving — otherwise it twitches
  // in place from sub-pixel jitter.
  if (Math.abs(vx) > 0.35) {
    const want = vx > 0 ? 1 : -1;
    s.face += (want - s.face) * 0.18;
  }

  // Idle timer: the pointer being still is what makes it idle, not the buddy
  // having arrived.
  s.idle += dt;

  // Hop: pace scales with speed so it trots when chasing and bobs when resting.
  // Halved from the first pass — at the original rate it read as jittery rather
  // than alive, and a slower bounce also makes the squash on landing legible.
  const pace = 2.75 + Math.min(5.5, s.speed * 0.75);
  const prevBob = s.bob;
  s.hop += dt * pace;
  const amp = 3 + Math.min(6, s.speed * 0.7);
  s.bob = Math.abs(Math.sin(s.hop)) * amp;

  // Landing squash: detect the downward zero-crossing of the hop.
  if (prevBob > 0.6 && s.bob <= 0.6) {
    s.squash = 0.74 - Math.min(0.12, s.speed * 0.012);
  }
  // Spring back toward neutral, with a little stretch at the top of the arc.
  const restingSquash = 1 + Math.min(0.16, s.speed * 0.014) * Math.sin(s.hop);
  s.squash += (restingSquash - s.squash) * 0.22;

  // Eyes lead the movement, and drift when idle so it looks around.
  const wantLookX = dist > 3 ? Math.max(-1, Math.min(1, dx / 60)) : Math.sin(now / 1400) * 0.7;
  const wantLookY = dist > 3 ? Math.max(-1, Math.min(1, dy / 60)) : Math.sin(now / 2300) * 0.5;
  s.lookX += (wantLookX - s.lookX) * 0.12;
  s.lookY += (wantLookY - s.lookY) * 0.12;

  // Idle flourishes: only once genuinely left alone, and never while chasing.
  if (s.idle > 2.4) {
    s.nextPerk -= dt;
    if (s.nextPerk <= 0 && s.perk === 'none') {
      s.perk = Math.random() < 0.55 ? 'jump' : 'spin';
      s.perkT = 0;
      s.nextPerk = 5 + Math.random() * 6;
    }
  }
  if (s.perk !== 'none') {
    s.perkT += dt;
    const dur = s.perk === 'jump' ? 0.55 : 0.7;
    if (s.perk === 'jump') {
      // one clean arc on top of the ambient bob
      const k = Math.min(1, s.perkT / dur);
      s.bob += Math.sin(k * Math.PI) * 16;
      s.squash += Math.sin(k * Math.PI) * 0.14;
    }
    if (s.perkT >= dur) { s.perk = 'none'; s.perkT = 0; }
  }

  return true;
}

/** Draw the buddy as blocky rects, matching the world's fillRect-only look. */
export function drawBuddy(ctx: CanvasRenderingContext2D, s: BuddyState, blink: boolean): void {
  const W = 26, H = 22;
  // squash preserves volume: wider when flatter
  const sy = s.squash;
  const sx = 1 / Math.max(0.5, sy);
  const cx = s.x + W / 2;
  const baseY = s.y + H - s.bob;      // feet stay on the "ground" the bob defines

  ctx.save();
  ctx.translate(cx, baseY);
  // Lean into travel. Small — enough to read, not enough to look broken.
  const lean = Math.max(-0.22, Math.min(0.22, s.speed * 0.016 * (s.face >= 0 ? 1 : -1)));
  if (s.perk === 'spin') {
    ctx.rotate(Math.sin((s.perkT / 0.7) * Math.PI * 2) * 0.5);
  } else {
    ctx.rotate(lean);
  }
  ctx.scale(sx * (s.face >= 0 ? 1 : -1), sy);

  const w = W, h = H;
  const left = -w / 2, top = -h;

  // shadow on the ground, tightening as it rises
  ctx.globalAlpha = 0.22 * Math.max(0.25, 1 - s.bob / 22);
  ctx.fillStyle = '#02171a';
  const shW = w * (0.9 - Math.min(0.3, s.bob / 40));
  ctx.fillRect(Math.round(-shW / 2), 1, Math.round(shW), 3);
  ctx.globalAlpha = 1;

  // feet — alternate as it hops so it reads as stepping
  const step = Math.sin(s.hop) * Math.min(3, s.speed * 0.5);
  ctx.fillStyle = '#2FB39B';
  ctx.fillRect(Math.round(left + 5 + step), Math.round(top + h - 1), 6, 4);
  ctx.fillRect(Math.round(left + 15 - step), Math.round(top + h - 1), 6, 4);

  // body
  ctx.fillStyle = '#45D9BE';
  ctx.fillRect(Math.round(left), Math.round(top), w, h);
  // lighter cap
  ctx.fillStyle = '#6FF0D8';
  ctx.fillRect(Math.round(left), Math.round(top), w, 8);

  // antenna, trailing behind the direction of travel
  ctx.fillStyle = '#2FB39B';
  const antX = left + w / 2 - 1 - s.speed * 0.35;
  ctx.fillRect(Math.round(antX), Math.round(top - 5), 2, 5);
  ctx.fillStyle = '#B8FFF0';
  ctx.fillRect(Math.round(antX - 1), Math.round(top - 8), 4, 4);

  // eyes: blink flattens them; otherwise they look where it is going
  ctx.fillStyle = '#062622';
  const ox = Math.round(s.lookX * 2);
  const oy = Math.round(s.lookY * 1.5);
  if (blink) {
    ctx.fillRect(Math.round(left + 6), Math.round(top + 12), 6, 2);
    ctx.fillRect(Math.round(left + 16), Math.round(top + 12), 6, 2);
  } else {
    ctx.fillRect(Math.round(left + 6 + ox), Math.round(top + 10 + oy), 5, 5);
    ctx.fillRect(Math.round(left + 16 + ox), Math.round(top + 10 + oy), 5, 5);
    // catchlight
    ctx.fillStyle = '#CFFFF6';
    ctx.fillRect(Math.round(left + 7 + ox), Math.round(top + 11 + oy), 2, 2);
    ctx.fillRect(Math.round(left + 17 + ox), Math.round(top + 11 + oy), 2, 2);
  }

  ctx.restore();
}
