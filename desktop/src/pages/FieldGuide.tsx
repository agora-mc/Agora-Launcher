/**
 * Field Guide — the ambience layer's journal + achievements as their own
 * sidebar page (moved out of the High Interaction instance editor).
 *
 * The journal is a keyboard-accessible record of every discovery in the
 * living background. Achievements are unlocked by the ambience engine and
 * persisted locally under the same storage key, so completion is permanent
 * as long as the local config is kept — even when the background is off this
 * page still reads the saved state and shows what you've earned.
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { BookOpen, RotateCcw } from 'lucide-react';
import { useAmbience } from '../features/ambience/AmbienceProvider';
import {
  journalFromSaved,
  milestoneAchievements,
  parseSavedJournal,
  type AchievementDef,
  type JournalData,
} from '../features/ambience/engine/eggs';
import { FieldJournalView } from '../features/ambience/FieldJournal';
import { getCachedJournalRawSync, loadJournalRaw } from '../features/ambience/journalStorage';
import {
  INTERACTION_ACHIEVEMENTS,
  loadEarnedInteraction,
} from '../features/interactive/live/interactionAchievements';

const ACHIEVEMENTS: AchievementDef[] = [
  { key: 'first', icon: '🌟', name: 'First Discovery' },
  { key: 'nat', icon: '🦉', name: 'Naturalist' },
  { key: 'zoo', icon: '🦁', name: 'Zoologist' },
  { key: 'ranger', icon: '🎖️', name: 'Ranger' },
  { key: 'complete', icon: '👑', name: 'Completionist' },
  { key: 'puzzle', icon: '🧠', name: 'Puzzle Master' },
  { key: 'night', icon: '🌙', name: 'Night Owl' },
  { key: 'quick', icon: '⏱️', name: 'Quick Study' },
];

interface FieldGuideData {
  journal: JournalData;
  unlocked: Set<string>;
}

/** Read the persisted journal + achievements from Agora app data with fallback. */
function loadSavedGuideSync(): FieldGuideData {
  let raw: string | null = null;
  try {
    raw = getCachedJournalRawSync();
  } catch {
    /* ignore */
  }
  const saved = parseSavedJournal(raw);
  return {
    journal: journalFromSaved(saved),
    unlocked: new Set(saved?.unlocked ?? []),
  };
}

async function loadSavedGuideAsync(): Promise<FieldGuideData> {
  let raw: string | null = null;
  try {
    raw = await loadJournalRaw();
  } catch {
    /* ignore */
  }
  const saved = parseSavedJournal(raw);
  return {
    journal: journalFromSaved(saved),
    unlocked: new Set(saved?.unlocked ?? []),
  };
}

/** Compute which achievements WOULD be earned from the found count (even if
 * they were never recorded — e.g. a fresh read of an old journal). */
function computeEarned(data: FieldGuideData): Set<string> {
  const state = {
    world: { found: Object.fromEntries(data.journal.tiers.flatMap((t) => t.entries.map((e) => [e.id, e.found]))) },
    unlocked: Object.fromEntries(Array.from(data.unlocked).map((k) => [k, true])),
    firstLoad: 0,
  } as never;
  const earned = milestoneAchievements(state, []);
  return new Set([...data.unlocked, ...earned.map((a) => a.key)]);
}

export function FieldGuide() {
  const { journal: liveJournal, enabled } = useAmbience();
  const [saved, setSaved] = useState<FieldGuideData>(() => loadSavedGuideSync());

  // Hydrate from Agora app data (local_state.db) — migrates legacy WebView
  // storage once and ensures deleting the Agora data folder resets progress.
  useEffect(() => {
    void loadSavedGuideAsync().then(setSaved);
  }, []);

  // When the engine is running, prefer its live journal (it is fresher); the
  // saved snapshot stays the fallback and the source of persistence.
  useEffect(() => {
    if (liveJournal) {
      setSaved((cur) => ({ ...cur, journal: liveJournal }));
    }
  }, [liveJournal]);

  const refresh = useCallback(() => { void loadSavedGuideAsync().then(setSaved); }, []);

  // Re-read on visibility/focus so closing the app and reopening is reflected.
  useEffect(() => {
    const onShow = () => { void loadSavedGuideAsync().then(setSaved); };
    window.addEventListener('focus', onShow);
    document.addEventListener('visibilitychange', onShow);
    return () => {
      window.removeEventListener('focus', onShow);
      document.removeEventListener('visibilitychange', onShow);
    };
  }, []);

  const earned = useMemo(() => computeEarned(saved), [saved]);
  const earnedCount = ACHIEVEMENTS.filter((a) => earned.has(a.key)).length;

  // Interaction achievements (the High Interaction instance editor's toasts):
  // same persisted store the WorldEditor writes, so each is earned exactly once.
  const [interactionEarned, setInteractionEarned] = useState<Set<string>>(() => loadEarnedInteraction());
  useEffect(() => { setInteractionEarned(loadEarnedInteraction()); }, [saved]);
  const interactionCount = INTERACTION_ACHIEVEMENTS.filter((a) => interactionEarned.has(a.key)).length;

  return (
    <div className="space-y-6" data-testid="field-guide-page">
      <header className="relative overflow-hidden agora-hero">
        <div className="absolute -right-16 -top-20 h-56 w-56 rounded-full border border-white/10" aria-hidden="true" />
        <div className="absolute -bottom-28 right-20 h-64 w-64 rounded-full border border-white/10" aria-hidden="true" />
        <div className="relative grid gap-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end">
          <div>
            <div className="mb-3 flex items-center gap-2">
              <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-white/10 ring-1 ring-white/20">
                <BookOpen className="h-4 w-4" aria-hidden="true" />
              </div>
              <p className="text-xs font-bold uppercase tracking-[0.2em] text-white/65">Agora field guide</p>
            </div>
            <h2 className="text-2xl font-bold">Field Guide</h2>
            <p className="mt-1 max-w-2xl text-sm text-white/75">
              Every discovery hidden in the living background, plus the achievements you've earned.
              {enabled ? ' The background is running — keep an eye out.' : ' Turn the living background on to find more.'}
            </p>
          </div>
          <div className="flex items-center gap-3">
            <div className="rounded-xl bg-white/10 px-4 py-2 text-center ring-1 ring-white/15">
              <div className="text-xl font-bold leading-none">{earnedCount}<span className="text-sm font-normal text-white/60">/{ACHIEVEMENTS.length}</span></div>
              <div className="mt-0.5 text-[11px] uppercase tracking-wide text-white/55">achievements</div>
            </div>
            <button
              type="button"
              onClick={refresh}
              className="inline-flex items-center gap-1.5 rounded-lg bg-white/10 px-3 py-2 text-sm font-semibold text-white ring-1 ring-white/15 hover:bg-white/20"
              aria-label="Refresh field guide"
            >
              <RotateCcw className="h-4 w-4" aria-hidden="true" />
              Refresh
            </button>
          </div>
        </div>
      </header>

      {/* Achievements — completion is persisted locally (agora-world-journal). */}
      <section className="rounded-xl border border-border bg-card p-4">
        <div className="mb-3 flex items-center justify-between gap-3">
          <h3 className="text-lg font-bold">Discoveries</h3>
          <p className="text-xs text-muted-foreground">Saved locally — they stay as long as your config does.</p>
        </div>
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
          {ACHIEVEMENTS.map((a) => {
            const isEarned = earned.has(a.key);
            return (
              <div
                key={a.key}
                data-testid={`achievement-${a.key}`}
                className={`flex items-center gap-3 rounded-xl border px-3 py-3 ${isEarned ? 'border-primary/50 bg-primary/10' : 'border-border bg-muted/40 opacity-70'}`}
              >
                <span className={`grid h-10 w-10 place-items-center rounded-lg text-xl ${isEarned ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'}`} aria-hidden="true">
                  {isEarned ? a.icon : '🔒'}
                </span>
                <div>
                  <div className="text-sm font-bold">{a.name}</div>
                  <div className="text-xs text-muted-foreground">{isEarned ? 'Earned' : 'Locked'}</div>
                </div>
              </div>
            );
          })}
        </div>
      </section>

      {/* Interaction achievements — earned once in the High Interaction
          instance editor (Searcher, Curious, Rearrange, …), persisted in the
          same local store, listed here so nothing re-announces forever. */}
      <section className="rounded-xl border border-border bg-card p-4">
        <div className="mb-3 flex items-center justify-between gap-3">
          <h3 className="text-lg font-bold">Workbench moments</h3>
          <p className="text-xs text-muted-foreground">Earned once in the High Interaction instance view — {interactionCount}/{INTERACTION_ACHIEVEMENTS.length}.</p>
        </div>
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {INTERACTION_ACHIEVEMENTS.map((a) => {
            const isEarned = interactionEarned.has(a.key);
            return (
              <div
                key={a.key}
                data-testid={`interaction-achievement-${a.key}`}
                className={`flex items-center gap-3 rounded-xl border px-3 py-2.5 ${isEarned ? 'border-primary/50 bg-primary/10' : 'border-border bg-muted/40 opacity-70'}`}
              >
                <span className={`grid h-9 w-9 place-items-center rounded-lg text-lg ${isEarned ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'}`} aria-hidden="true">
                  {isEarned ? a.icon : '🔒'}
                </span>
                <div>
                  <div className="text-sm font-bold">{a.name}</div>
                  <div className="text-xs text-muted-foreground">{a.detail}</div>
                </div>
              </div>
            );
          })}
        </div>
      </section>

      {/* The discovery journal (keyboard-accessible equivalent of the world). */}
      <section className="rounded-xl border border-border bg-card p-4">
        <FieldJournalView data={saved.journal} />
      </section>
    </div>
  );
}
