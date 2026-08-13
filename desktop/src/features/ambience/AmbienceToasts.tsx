/**
 * Ambience toasts + carry tag.
 *
 * The engine emits events; this component presents them as the prototype's
 * achievement/discovery toasts and the carry tag ("Picked up: …"). Purely
 * presentational — the engine never touches the DOM for these.
 */

import { useEffect, useRef, useState } from 'react';
import { useAmbience } from './AmbienceProvider';
import type { AmbienceEvent } from './engine/state';

interface ToastItem {
  id: number;
  icon: string;
  title: string;
  detail: string;
}

let toastId = 0;

export function AmbienceToasts() {
  const { lastEvent } = useAmbience();
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [carry, setCarry] = useState<string | null>(null);
  const carryTimer = useRef<number | null>(null);
  const lastShown = useRef<number | null>(null);

  useEffect(() => {
    if (!lastEvent) return;
    const ev: AmbienceEvent = lastEvent;
    if (ev.type === 'carry') {
      setCarry(ev.name);
      if (carryTimer.current !== null) window.clearTimeout(carryTimer.current);
      carryTimer.current = window.setTimeout(() => setCarry(null), ev.firstTime ? 2200 : 900);
      return;
    }
    if (ev.type === 'drop-carry') { setCarry(null); return; }
    if (ev.type === 'discovery') {
      // the discovery count comes from the journal
      setToasts((cur) => [...cur.slice(-3), { id: ++toastId, icon: '🔎', title: 'Discovery', detail: `${ev.eggName}` }]);
    } else if (ev.type === 'achievement') {
      setToasts((cur) => [...cur.slice(-3), { id: ++toastId, icon: ev.icon, title: 'Achievement', detail: ev.name }]);
    } else if (ev.type === 'completion') {
      setToasts((cur) => [...cur.slice(-3), { id: ++toastId, icon: '👑', title: 'Complete!', detail: 'Every discovery found.' }]);
    }
  }, [lastEvent]);

  // dismiss each toast after 4.2s (prototype timing)
  useEffect(() => {
    if (!toasts.length) return;
    const last = toasts[toasts.length - 1];
    if (lastShown.current === last.id) return;
    lastShown.current = last.id;
    const t = window.setTimeout(() => {
      setToasts((cur) => cur.filter((x) => x.id !== last.id));
    }, 4200);
    return () => window.clearTimeout(t);
  }, [toasts]);

  return (
    <>
      <div className="ambience-toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className="ambience-toast" role="status">
            <span className="ambience-toast-ico">{t.icon}</span>
            <span>
              <span className="ambience-toast-t">{t.title}</span>
              <span className="ambience-toast-d">{t.detail}</span>
            </span>
          </div>
        ))}
      </div>
      {carry ? (
        <div className="ambience-carry-tag" role="status">Picked up: {carry}</div>
      ) : null}
    </>
  );
}
