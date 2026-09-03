import { useEffect, useRef, type KeyboardEvent } from 'react';
import { Gamepad2, X } from 'lucide-react';
import type { ControlifyOffer } from '../../lib/tauri';

export function canInstallControlify(offer: ControlifyOffer): offer is ControlifyOffer & { decision: 'offer'; modrinth_slug: string } {
  return offer.decision === 'offer'
    && typeof offer.modrinth_slug === 'string'
    && offer.modrinth_slug.trim().length > 0;
}

export interface ControlifyOfferDialogProps {
  offer: ControlifyOffer;
  onAccept: (slug: string) => void;
  onDecline: () => void;
}

export function ControlifyOfferDialog({ offer, onAccept, onDecline }: ControlifyOfferDialogProps) {
  const acceptRef = useRef<HTMLButtonElement>(null);
  const declineRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const installable = canInstallControlify(offer);

  useEffect(() => {
    (installable ? acceptRef.current : declineRef.current)?.focus();
  }, [installable]);

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      onDecline();
    }
  };

  return (
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-slate-950/80 p-4 backdrop-blur-sm"
      role="presentation"
      onKeyDown={handleKeyDown}
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="controlify-offer-title"
        className="w-full max-w-lg rounded-3xl border border-cyan-300/30 bg-slate-900 p-6 text-slate-100 shadow-2xl sm:p-8"
      >
        <div className="flex items-start justify-between gap-4">
          <div className="flex items-center gap-3">
            <span className="flex h-11 w-11 items-center justify-center rounded-2xl bg-cyan-300/15 text-cyan-300">
              <Gamepad2 className="h-6 w-6" aria-hidden="true" />
            </span>
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.2em] text-cyan-300/80">Controller support</p>
              <h2 id="controlify-offer-title" className="mt-1 text-xl font-bold">Add Controlify?</h2>
            </div>
          </div>
          <button
            ref={closeRef}
            type="button"
            onClick={onDecline}
            aria-label="Close Controlify offer"
            className="rounded-xl p-2 text-slate-400 hover:bg-white/10 hover:text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-cyan-300"
          >
            <X className="h-5 w-5" aria-hidden="true" />
          </button>
        </div>

        <p className="mt-6 text-base leading-7 text-slate-200">{offer.reason}</p>
        {installable ? (
          <p className="mt-3 text-sm leading-6 text-slate-400">
            Agora will install the verified Modrinth project <span className="font-medium text-slate-200">{offer.modrinth_slug}</span> into this instance before launching it. You can review the normal install flow or cancel it.
          </p>
        ) : (
          <p className="mt-3 text-sm leading-6 text-amber-200/80">Controlify is not available to install for this decision.</p>
        )}

        <div className="mt-7 flex flex-wrap justify-end gap-3">
          <button
            ref={declineRef}
            type="button"
            onClick={onDecline}
            className="rounded-xl border border-white/15 px-4 py-2.5 text-sm font-semibold text-slate-200 hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-cyan-300"
          >
            Not now
          </button>
          {installable && (
            <button
              ref={acceptRef}
              type="button"
              onClick={() => {
                if (!canInstallControlify(offer)) return;
                onAccept(offer.modrinth_slug);
              }}
              className="rounded-xl bg-cyan-300 px-4 py-2.5 text-sm font-bold text-slate-950 hover:bg-cyan-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-cyan-100"
            >
              Install Controlify
            </button>
          )}
        </div>
      </section>
    </div>
  );
}
