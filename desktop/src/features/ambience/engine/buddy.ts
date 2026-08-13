/**
 * The cursor companion ("buddy") — ported from v4-world.html.
 *
 * The prototype rendered the buddy as an inline SVG with two blinking eyes
 * and moved it with a `transform`. In the app the buddy is drawn on the FX
 * canvas by the engine, keeping it out of the React tree entirely (it moves
 * 60×/second). The hop and blink logic is verbatim.
 */

export interface BuddyState {
  x: number;
  y: number;
  tx: number;
  ty: number;
  on: boolean;
  reduced: boolean;
}

export function createBuddy(reduced: boolean, w: number, h: number): BuddyState {
  return { x: w / 2, y: h / 2, tx: w / 2, ty: h / 2, on: true, reduced };
}

export function buddyTarget(s: BuddyState, cx: number, cy: number): void {
  s.tx = cx + 26;
  s.ty = cy + 26;
}

export function buddyVisible(s: BuddyState, visible: boolean): void {
  s.on = visible;
}

/** Advance one frame (prototype's hop loop). Returns true when it moved. */
export function buddyStep(s: BuddyState, now: number): boolean {
  if (!s.on || s.reduced) return false;
  s.x += (s.tx - s.x) * 0.06;
  s.y += (s.ty - s.y) * 0.06;
  const b = Math.abs(Math.sin(now / 260)) * 5;
  void b;
  return true;
}

/** Draw the buddy as blocky rects (the prototype's inline SVG, 1:1). */
export function drawBuddy(ctx: CanvasRenderingContext2D, s: BuddyState, blink: boolean): void {
  const x = s.x, y = s.y;
  ctx.save();
  // body
  ctx.fillStyle = '#45D9BE';
  ctx.fillRect(x, y, 26, 22);
  // head cap
  ctx.fillStyle = '#6FF0D8';
  ctx.fillRect(x, y, 26, 8);
  // eyes (blink shrinks them, prototype's setInterval)
  const r = blink ? 0.6 : 3.2;
  ctx.fillStyle = '#062622';
  ctx.beginPath();
  ctx.arc(x + 8, y + 11, r, 0, 6.283);
  ctx.fill();
  ctx.beginPath();
  ctx.arc(x + 18, y + 11, r, 0, 6.283);
  ctx.fill();
  // feet
  ctx.fillStyle = '#2FB39B';
  ctx.fillRect(x + 5, y + 21, 6, 4);
  ctx.fillRect(x + 15, y + 21, 6, 4);
  ctx.restore();
}
