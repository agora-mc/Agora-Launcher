/**
 * Click-burst particles + cursor trail — ported from v4-world.html.
 * The engine owns the frame loop; `frame` renders exactly one frame.
 */

interface P { x: number; y: number; vx: number; vy: number; s: number; life: number; c: string; rot: number; }
interface T { x: number; y: number; life: number; s: number; }

export class ParticleLayer {
  parts: P[] = [];
  trail: T[] = [];

  /** No particles under reduced motion (prototype `if(reduce)return`). */
  reduced = false;

  burst(x: number, y: number, color: string, n?: number, spread?: number): void {
    if (this.reduced) return;
    for (let i = 0; i < (n || 20); i++) {
      this.parts.push({
        x: x, y: y, vx: (Math.random() - 0.5) * (spread || 8),
        vy: (Math.random() - 0.95) * (spread || 8), s: 3 + Math.random() * 5, life: 1, c: color, rot: Math.random() * 6,
      });
    }
    if (this.parts.length > 200) this.parts.splice(0, this.parts.length - 200);
  }

  /** One cursor-trail sample from a pointermove (prototype's mousemove handler). */
  pointerMoved(x: number, y: number): void {
    if (this.reduced) return;
    if (Math.random() < 0.5) this.trail.push({ x: x, y: y, life: 1, s: 2 + Math.random() * 3 });
  }

  frame(fc: CanvasRenderingContext2D): void {
    fc.clearRect(0, 0, fc.canvas.width, fc.canvas.height);
    for (let i = this.trail.length - 1; i >= 0; i--) {
      const t = this.trail[i];
      t.life -= 0.045;
      if (t.life <= 0) { this.trail.splice(i, 1); continue; }
      fc.globalAlpha = t.life * 0.55;
      fc.fillStyle = '#9FF5E2';
      fc.fillRect(t.x, t.y, t.s, t.s);
    }
    for (let i = this.parts.length - 1; i >= 0; i--) {
      const p = this.parts[i];
      p.vy += 0.3;
      p.x += p.vx;
      p.y += p.vy;
      p.life -= 0.015;
      p.rot += 0.2;
      if (p.life <= 0) { this.parts.splice(i, 1); continue; }
      fc.globalAlpha = Math.max(p.life, 0);
      fc.fillStyle = p.c;
      fc.save();
      fc.translate(p.x, p.y);
      fc.rotate(p.rot);
      fc.fillRect(-p.s / 2, -p.s / 2, p.s, p.s);
      fc.restore();
    }
    fc.globalAlpha = 1;
  }

  clear(): void {
    this.parts.length = 0;
    this.trail.length = 0;
  }
}

/** Center of a DOM element (used by the React shell for bursts at UI points). */
export function centerOf(el: Element): [number, number] {
  const r = el.getBoundingClientRect();
  return [r.left + r.width / 2, r.top + r.height / 2];
}
