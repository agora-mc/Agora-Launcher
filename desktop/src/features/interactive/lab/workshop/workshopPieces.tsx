/**
 * Workshop pieces — the blocky 4px-tile icon painter and the drag helper.
 * Every icon is fillRect only (no images, no network); drag has a keyboard
 * equivalent (Enter/Space drops into the first slot) so drag is never the
 * only route in.
 */

import { useEffect, useRef } from 'react';

export type ArtFn = (c: CanvasRenderingContext2D, w: number, h: number) => void;

export function px(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, col: string): void {
  ctx.fillStyle = col;
  ctx.fillRect(x, y, w, h);
}

/** Paint a blocky icon onto a canvas element sized to its CSS box. */
export function paint(canvas: HTMLCanvasElement, fn: ArtFn): void {
  const c = canvas.getContext('2d');
  if (!c) return;
  const r = canvas.getBoundingClientRect();
  canvas.width = Math.max(40, Math.round(r.width));
  canvas.height = Math.max(30, Math.round(r.height));
  c.clearRect(0, 0, canvas.width, canvas.height);
  fn(c, canvas.width, canvas.height);
}

export interface PieceSpec {
  id: string;
  name: string;
  note?: string;
  art: ArtFn;
  kind?: string;
}

export function mkPiece(spec: PieceSpec): { el: HTMLDivElement } {
  const el = document.createElement('div');
  el.className = 'ws-piece';
  el.innerHTML = '<canvas></canvas><b></b><small></small>';
  el.querySelector('b')!.textContent = spec.name;
  el.querySelector('small')!.textContent = spec.note || '';
  el.dataset.id = spec.id;
  const canvas = el.querySelector('canvas') as HTMLCanvasElement;
  // paint after layout
  requestAnimationFrame(() => paint(canvas, spec.art));
  return { el };
}

/**
 * Pointer drag that works with mouse and touch: pick up, drop on a target,
 * everything else is a click. Keyboard equivalent: Enter/Space drops into the
 * first `.ws-slotbig` on the page.
 */
export function draggable(el: HTMLElement, onDrop: (target: HTMLElement | null) => void): void {
  el.addEventListener('pointerdown', (e) => {
    if (e.button) return;
    e.preventDefault();
    const r = el.getBoundingClientRect();
    const g = el.cloneNode(true) as HTMLElement;
    g.style.cssText = `position:fixed;z-index:80;pointer-events:none;width:${r.width}px;opacity:.9`;
    document.body.appendChild(g);
    const mv = (ev: PointerEvent) => {
      g.style.left = `${ev.clientX - r.width / 2}px`;
      g.style.top = `${ev.clientY - 22}px`;
      const t = document.elementFromPoint(ev.clientX, ev.clientY);
      document.querySelectorAll('.ws-slotbig').forEach((s) => s.classList.remove('hot'));
      const slot = t?.closest?.('.ws-slotbig');
      if (slot) slot.classList.add('hot');
    };
    mv(e);
    const up = (ev: PointerEvent) => {
      document.removeEventListener('pointermove', mv);
      document.removeEventListener('pointerup', up);
      g.remove();
      document.querySelectorAll('.ws-slotbig').forEach((s) => s.classList.remove('hot'));
      const t = document.elementFromPoint(ev.clientX, ev.clientY);
      const slot = t?.closest?.('.ws-slotbig') as HTMLElement | null;
      onDrop(slot);
    };
    document.addEventListener('pointermove', mv);
    document.addEventListener('pointerup', up);
  });
  el.tabIndex = 0;
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      const slot = document.querySelector('.ws-slotbig') as HTMLElement | null;
      onDrop(slot);
    }
  });
}

/** React component wrapper for a blocky-art canvas. */
export function ArtCanvas({ art, size, className }: { art: ArtFn; size: number; className?: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    c.style.width = `${size}px`;
    c.style.height = `${size * 0.46}px`;
    c.width = size;
    c.height = Math.max(30, Math.round(size * 0.46));
    const ctx = c.getContext('2d');
    if (ctx) {
      ctx.clearRect(0, 0, c.width, c.height);
      art(ctx, c.width, c.height);
    }
  }, [art, size]);
  return <canvas ref={ref} className={className} aria-hidden="true" />;
}

export interface ArtSpec { [k: string]: ArtFn | ((col: string) => ArtFn) }

/** The v5-lab ART catalogue (blocky icons for stations, pieces, doors). */
export const ART: {
  build: ArtFn;
  mod: ArtFn;
  fix: ArtFn;
  heal: ArtFn;
  offline: ArtFn;
  undo: ArtFn;
  crystal: (col: string) => ArtFn;
  gear: (col: string) => ArtFn;
  box: (col: string, notch?: 'tab' | 'hole') => ArtFn;
  door: (col: string, face?: '?') => ArtFn;
} = {
  build(c, w, h) {
    const b = Math.min(w, h) / 9, cx = w / 2, base = h - b * 1.2;
    px(c, cx - b * 3, base - b, b * 6, b, '#5C4630');
    [[0, 2], [1, 2], [2, 2], [0, 1], [1, 1], [0, 0]].forEach((p) => {
      px(c, cx - b * 3 + p[0] * b * 1.1, base - b * 2 - p[1] * b, b, b * 0.9, p[1] === 0 ? '#8BE24F' : '#4C8FBF');
    });
    px(c, cx + b * 1.4, base - b * 3.2, b * 1.6, b * 3.2, '#FFD34E');
  },
  mod(c, w, h) {
    const b = Math.min(w, h) / 9, cx = w / 2, cy = h / 2;
    px(c, cx - b * 3.2, cy - b * 1.4, b * 2.8, b * 2.8, '#6FD3E8');
    px(c, cx - b * 0.4, cy - b * 0.2, b, b, '#6FD3E8');
    px(c, cx + b * 0.6, cy - b * 1.4, b * 2.8, b * 2.8, '#B48CF2');
    px(c, cx + b * 0.6, cy - b * 0.2, b, b, '#0A1820');
  },
  fix(c, w, h) {
    const b = Math.min(w, h) / 9, cx = w / 2, cy = h / 2;
    px(c, cx - b * 2.6, cy - b * 2.6, b * 4.4, b * 4.4, '#1B3A48');
    px(c, cx - b * 1.9, cy - b * 1.9, b * 3, b * 3, '#9FE8FF');
    px(c, cx + b * 1.4, cy + b * 1.4, b * 2.4, b * 0.9, '#FFD34E');
  },
  heal(c, w, h) {
    const b = Math.min(w, h) / 9, cx = w / 2, cy = h / 2;
    px(c, cx - b * 0.7, cy - b * 2.6, b * 1.4, b * 5.2, '#FF7A6B');
    px(c, cx - b * 2.6, cy - b * 0.7, b * 5.2, b * 1.4, '#FF7A6B');
  },
  offline(c, w, h) {
    const b = Math.min(w, h) / 9, cx = w / 2, cy = h / 2;
    px(c, cx - b * 2.4, cy - b * 1.6, b * 4.8, b * 3.6, '#B08A4A');
    px(c, cx - b * 1, cy - b * 2.6, b * 2, b * 1, '#8A6A34');
    px(c, cx - b * 0.4, cy - b * 0.4, b * 0.8, b * 1.6, '#3A2A16');
  },
  undo(c, w, h) {
    const b = Math.min(w, h) / 9, cx = w / 2, cy = h / 2;
    for (let i = 0; i < 7; i++) {
      const a = Math.PI * 0.35 + i * (Math.PI * 1.25 / 6);
      px(c, cx + Math.cos(a) * b * 2.6 - b * 0.4, cy + Math.sin(a) * b * 2.6 - b * 0.4, b * 0.9, b * 0.9, '#6FD3E8');
    }
    px(c, cx - b * 3.4, cy - b * 2.2, b * 1.2, b * 1.2, '#8BE24F');
  },
  crystal(col) {
    return (c, w, h) => {
      const b = Math.min(w, h) / 6, cx = w / 2, cy = h / 2;
      px(c, cx - b, cy - b * 1.6, b * 2, b * 3.2, col);
      px(c, cx - b * 1.6, cy - b * 0.6, b * 3.2, b * 1.2, col);
      px(c, cx - b * 0.5, cy - b * 1.2, b * 0.6, b * 0.8, 'rgba(255,255,255,.55)');
    };
  },
  gear(col) {
    return (c, w, h) => {
      const b = Math.min(w, h) / 7, cx = w / 2, cy = h / 2;
      px(c, cx - b * 1.5, cy - b * 1.5, b * 3, b * 3, col);
      px(c, cx - b * 2.3, cy - b * 0.5, b * 0.8, b * 1, col);
      px(c, cx + b * 1.5, cy - b * 0.5, b * 0.8, b * 1, col);
      px(c, cx - b * 0.5, cy - b * 2.3, b * 1, b * 0.8, col);
      px(c, cx - b * 0.5, cy + b * 1.5, b * 1, b * 0.8, col);
      px(c, cx - b * 0.6, cy - b * 0.6, b * 1.2, b * 1.2, '#0A1820');
    };
  },
  box(col, notch) {
    return (c, w, h) => {
      const b = Math.min(w, h) / 6, cx = w / 2, cy = h / 2;
      px(c, cx - b * 2, cy - b * 1.5, b * 4, b * 3, col);
      if (notch === 'tab') px(c, cx + b * 2, cy - b * 0.5, b * 0.8, b * 1, col);
      if (notch === 'hole') px(c, cx - b * 2, cy - b * 0.5, b * 0.8, b * 1, '#0A1820');
    };
  },
  door(col, face) {
    return (c, w, h) => {
      const b = Math.min(w, h) / 7, cx = w / 2, base = h - b;
      px(c, cx - b * 2, base - b * 5, b * 4, b * 5, col);
      px(c, cx + b * 1.1, base - b * 2.6, b * 0.7, b * 0.7, '#0A1820');
      if (face === '?') {
        px(c, cx - b * 0.9, base - b * 4, b * 1.8, b * 0.6, '#0A1820');
        px(c, cx + b * 0.3, base - b * 3.4, b * 0.6, b * 0.8, '#0A1820');
        px(c, cx - b * 0.3, base - b * 2.6, b * 0.6, b * 0.6, '#0A1820');
      }
    };
  },
};

