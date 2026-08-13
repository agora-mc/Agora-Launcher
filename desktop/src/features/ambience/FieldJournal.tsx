/**
 * Field Journal — the keyboard-accessible equivalent of the living world's
 * easter eggs (the prototype's `#journalPanel`). Every discovery is listed in
 * text; the canvas itself stays `aria-hidden` and unreachable by keyboard.
 */

import { useAmbience } from './AmbienceProvider';
import type { JournalData } from './engine/eggs';

export function FieldJournal() {
  const { journal } = useAmbience();
  if (!journal) {
    return (
      <div className="ambience-journal" data-testid="field-journal">
        <p className="text-sm text-muted-foreground">The Field Journal will appear once the living background is running.</p>
      </div>
    );
  }
  return <FieldJournalView data={journal} />;
}

export function FieldJournalView({ data }: { data: JournalData }) {
  return (
    <div className="ambience-journal" data-testid="field-journal">
      <p className="ambience-journal-note">
        The background world is optional, decorative content and isn't reachable by keyboard.
        This journal lists every discovery in text and is fully keyboard accessible — that's the
        accessible equivalent.
      </p>
      <div className="flex items-center gap-3">
        <div className="ambience-journal-ring" style={{ background: `conic-gradient(var(--ambience-rare) ${data.percent * 3.6}deg, rgba(255,255,255,.12) 0deg)` }}>
          <div className="ambience-journal-ring-inner">
            <span className="ambience-journal-pct">{data.percent}%</span>
          </div>
        </div>
        <div>
          <div className="text-lg font-bold leading-none">{data.foundCount} <span className="text-sm font-normal text-muted-foreground">/ {data.total}</span></div>
          <div className="text-xs text-muted-foreground">discovered</div>
        </div>
      </div>
      {data.tiers.map((tier) => (
        <div key={tier.tier} className="ambience-journal-sec">
          <h4 className="ambience-journal-h">{tier.name}</h4>
          <div className="ambience-journal-grid">
            {tier.entries.map((e) => (
              <div key={e.id} className={`ambience-journal-card ${e.found ? 'found' : 'unfound'}`}>
                <span className="ambience-journal-ico">{e.found ? '✓' : '?'}</span>
                <span>
                  <span className="ambience-journal-name">{e.found ? e.name : '???'}</span>
                  <span className="ambience-journal-hint">{e.found ? 'Discovered' : e.hint}</span>
                </span>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
