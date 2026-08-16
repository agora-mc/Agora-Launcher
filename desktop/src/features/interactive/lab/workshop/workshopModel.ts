/**
 * Workshop model — stations, per-step progress, and the bench step contract.
 *
 * Progress is per STEP (not per bench), versioned under `agora-lab-v5b`, so a
 * later bench reopening resumes exactly where the player stopped. This is pure
 * presentation/state logic — no Tauri, no live/, no operations.
 */

export interface WorkshopStation {
  id: string;
  title: string;
  badge: string;
  blurb: string;
  why: string;
}

export type BenchStepKind = 'do' | 'predict' | 'transfer' | 'explain';

export interface BenchStep {
  id: string;
  kind: BenchStepKind;
  title: string;
  lead: string;
}

export interface WorkshopBench {
  id: string;
  title: string;
  why: string;
  steps: BenchStep[];
}

export const STATIONS: WorkshopStation[] = [
  { id: 'build', title: 'Build it', badge: '🔨', blurb: "Make a world from scratch. Some pieces fit together, some don't.", why: 'Two pieces have to agree before anything works. You can see it when they snap.' },
  { id: 'mod', title: 'Add stuff', badge: '🧩', blurb: 'Drop in a new thing. Watch what it drags along with it.', why: 'Some add-ons need a friend to work, and some refuse to sit next to each other.' },
  { id: 'fix', title: 'Something broke', badge: '🔍', blurb: 'Read the clues, pick a suspect, test it safely.', why: "You don't guess. You try one thing at a time, and you can always undo it." },
  { id: 'heal', title: 'Health check', badge: '💚', blurb: 'Scan for problems before they bite.', why: 'Catching it early is easier than fixing it later.' },
  { id: 'offline', title: 'Going offline', badge: '🎒', blurb: 'Pack everything you need for no-internet.', why: 'Know what still works when the wifi drops.' },
  { id: 'undo', title: 'Undo it', badge: '⏪', blurb: 'Go back in time to before it broke.', why: 'A save point means nothing is ever permanent.' },
];

export const STORAGE_KEY = 'agora-lab-v5b';

export type ProgressMap = Record<string, Record<string, boolean>>;

export function loadProgress(): ProgressMap {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? '{}') as ProgressMap;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

export function saveProgress(progress: ProgressMap): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(progress));
  } catch {
    // best effort
  }
}

export function stepCount(bench: WorkshopBench): number {
  return bench.steps.length;
}

export function stepsDone(progress: ProgressMap, bench: WorkshopBench): number {
  const d = progress[bench.id];
  if (!d) return 0;
  return bench.steps.filter((s) => d[s.id]).length;
}

export function benchComplete(progress: ProgressMap, bench: WorkshopBench): boolean {
  return stepCount(bench) > 0 && stepsDone(progress, bench) === stepCount(bench);
}

export function firstUndone(progress: ProgressMap, bench: WorkshopBench): number {
  for (let i = 0; i < bench.steps.length; i++) {
    if (!progress[bench.id]?.[bench.steps[i].id]) return i;
  }
  return 0; // all done — reopen at the start
}

/** Mark a step done and persist. Returns true when the whole bench just completed. */
export function markStepDone(
  progress: ProgressMap,
  bench: WorkshopBench,
  stepIndex: number,
): { next: ProgressMap; completed: boolean } {
  const stepId = bench.steps[stepIndex]?.id;
  if (!stepId) return { next: progress, completed: false };
  if (progress[bench.id]?.[stepId]) return { next: progress, completed: benchComplete(progress, bench) };
  const next = {
    ...progress,
    [bench.id]: { ...(progress[bench.id] ?? {}), [stepId]: true },
  };
  return { next, completed: benchComplete(next, bench) };
}
