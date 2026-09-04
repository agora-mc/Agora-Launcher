import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { ArrowLeft, ArrowRight, Gamepad2, LogOut, Play, RefreshCw } from 'lucide-react';
import { InstanceIcon } from '../InstanceIcon';
import {
  listInstances,
  type InstanceRow,
} from '../../lib/tauri';
import { useController } from '../../features/controller/ControllerProvider';
import type { ControllerDirection, ControllerIntent } from '../../features/controller/intents';
import { useControllerLayer } from '../../features/controller/useControllerLayer';

export interface HandheldShellProps {
  active: boolean;
  onActiveChange: (active: boolean) => void;
  onLaunch: (instanceId: string) => Promise<boolean>;
  launchBusy?: boolean;
}

export function handheldGridColumns(width: number): number {
  if (width >= 1024) return 3;
  if (width >= 640) return 2;
  return 1;
}

/** Move within the visible card grid, wrapping at the edge of each axis. */
export function moveHandheldSelection(
  index: number,
  direction: ControllerDirection,
  count: number,
  columns: number,
): number {
  if (count <= 0) return 0;
  const columnCount = Math.max(1, Math.min(columns, count));
  const safeIndex = Math.max(0, Math.min(index, count - 1));

  if (direction === 'left') {
    return safeIndex % columnCount === 0 ? count - 1 : safeIndex - 1;
  }
  if (direction === 'right') {
    return safeIndex === count - 1 || (safeIndex + 1) % columnCount === 0 ? 0 : safeIndex + 1;
  }

  if (direction === 'up') {
    if (safeIndex >= columnCount) return safeIndex - columnCount;
    let last = safeIndex;
    while (last + columnCount < count) last += columnCount;
    return last;
  }

  const below = safeIndex + columnCount;
  return below < count ? below : safeIndex % columnCount;
}

function formatLoader(instance: InstanceRow): string {
  const loader = instance.loader.trim();
  if (!loader || loader.toLowerCase() === 'vanilla') return 'Vanilla';
  return `${loader} ${instance.loader_version}`.trim();
}

export function HandheldShell({
  active,
  onActiveChange,
  onLaunch,
  launchBusy = false,
}: HandheldShellProps) {
  const [instances, setInstances] = useState<InstanceRow[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const shellRef = useRef<HTMLDivElement>(null);
  const cardRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const { connected } = useController();

  const handleInactiveControllerIntent = useCallback((intent: ControllerIntent) => {
    // The shell stays registered while hidden so Start can re-enter after B or
    // Escape closed handheld mode without requiring a second connection event.
    //
    // Claim *only* that intent. Returning true unconditionally would make a
    // closed handheld shell swallow every input in the app, which costs nothing
    // today because nothing else is controller-aware yet, and would silently
    // break every page the moment one is.
    if (intent.type !== 'menu') return undefined;
    onActiveChange(true);
    return true;
  }, [onActiveChange]);

  const handleControllerIntent = useCallback((intent: ControllerIntent) => {
    if (intent.type === 'menu') {
      // Start is the way back in after leaving with B or Escape, so it toggles
      // whenever a pad is being used.
      onActiveChange(!active);
      return true;
    }
    if (!active) return true;

    if (intent.type === 'navigate') {
      setSelectedIndex((current) => moveHandheldSelection(
        current,
        intent.direction,
        instances.length,
        handheldGridColumns(typeof window === 'undefined' ? 1024 : window.innerWidth),
      ));
      return true;
    }
    if (intent.type === 'accept') {
      const instance = instances[selectedIndex];
      if (instance && !launchBusy) void onLaunch(instance.instance_id);
      return true;
    }
    return undefined;
  }, [active, instances, launchBusy, onActiveChange, onLaunch, selectedIndex]);

  const handleControllerCancel = useCallback(() => {
    if (active) onActiveChange(false);
  }, [active, onActiveChange]);

  useControllerLayer({
    active: !active,
    rootRef: shellRef,
    onIntent: handleInactiveControllerIntent,
    // Listening for one button is all a closed shell wants. Left opaque, it
    // would sit on top of the whole app for as long as handheld mode is shut —
    // which is nearly always — and silently absorb every other intent before
    // the application shell beneath could act on it.
    transparent: true,
  });
  useControllerLayer({
    active,
    rootRef: shellRef,
    onIntent: handleControllerIntent,
    onCancel: handleControllerCancel,
  });

  const loadInstances = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const rows = await listInstances();
      setInstances(rows);
      setSelectedIndex((current) => Math.max(0, Math.min(current, rows.length - 1)));
    } catch (cause) {
      setInstances([]);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (active) void loadInstances();
  }, [active, loadInstances]);

  // Escape is deliberately global: focus can be on a card, a browser-managed
  // control, or the shell itself, and none of those should trap the user.
  useEffect(() => {
    if (!active) return;
    const handleKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      event.preventDefault();
      onActiveChange(false);
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [active, onActiveChange]);

  useEffect(() => {
    if (!active) return;
    const frame = requestAnimationFrame(() => {
      const card = cardRefs.current[selectedIndex];
      if (card) card.focus();
      else shellRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [active, selectedIndex, instances.length, loading]);

  const handleKeyboardNavigation = (event: KeyboardEvent<HTMLDivElement>) => {
    const direction: ControllerDirection | null =
      event.key === 'ArrowUp' ? 'up'
        : event.key === 'ArrowDown' ? 'down'
          : event.key === 'ArrowLeft' ? 'left'
            : event.key === 'ArrowRight' ? 'right'
              : null;
    if (!direction) return;
    event.preventDefault();
    setSelectedIndex((current) => moveHandheldSelection(
      current,
      direction,
      instances.length,
      handheldGridColumns(typeof window === 'undefined' ? 1024 : window.innerWidth),
    ));
  };

  if (!active) return null;

  return (
    <div
      ref={shellRef}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-label="Handheld mode"
      onKeyDown={handleKeyboardNavigation}
      className="fixed inset-0 z-[50] flex min-h-0 flex-col overflow-hidden bg-slate-950 text-slate-100 outline-none"
    >
      <header className="flex shrink-0 items-center justify-between gap-4 border-b border-white/10 px-6 py-5 sm:px-10">
        <div className="flex min-w-0 items-center gap-3">
          <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-cyan-400/15 text-cyan-300">
            <Gamepad2 className="h-6 w-6" aria-hidden="true" />
          </span>
          <div className="min-w-0">
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-cyan-300/80">Agora</p>
            <h1 className="truncate text-xl font-bold sm:text-2xl">Handheld mode</h1>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-3 text-xs text-slate-300">
          <span className={connected ? 'text-emerald-300' : 'text-amber-300'}>
            {connected ? 'Controller connected' : 'Controller disconnected'}
          </span>
          <button
            type="button"
            onClick={() => onActiveChange(false)}
            className="inline-flex items-center gap-2 rounded-xl border border-white/15 px-3 py-2 font-semibold text-slate-100 hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-cyan-300"
          >
            <LogOut className="h-4 w-4" aria-hidden="true" />
            Exit
          </button>
        </div>
      </header>

      <main className="min-h-0 flex-1 overflow-y-auto px-6 py-8 sm:px-10 lg:px-16">
        <div className="mx-auto max-w-6xl">
          <div className="mb-6 flex flex-wrap items-end justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-slate-400">Choose a world to play</p>
              <p className="mt-1 text-xs text-slate-500">Use the D-pad or left stick to move, then press A.</p>
            </div>
            {connected && <span className="rounded-full bg-emerald-400/10 px-3 py-1 text-xs font-medium text-emerald-300">Ready to play</span>}
          </div>

          {loading ? (
            <div className="flex min-h-56 items-center justify-center rounded-3xl border border-white/10 bg-white/[0.04] text-sm text-slate-400" aria-live="polite">
              <RefreshCw className="mr-2 h-4 w-4 animate-spin" aria-hidden="true" />
              Loading instances…
            </div>
          ) : error ? (
            <div className="rounded-3xl border border-rose-400/30 bg-rose-400/10 p-8 text-center" role="alert">
              <p className="font-semibold text-rose-200">Couldn’t load your instances</p>
              <p className="mt-2 text-sm text-rose-100/70">{error}</p>
              <button
                type="button"
                onClick={() => void loadInstances()}
                className="mt-5 rounded-xl border border-white/15 px-4 py-2 text-sm font-semibold hover:bg-white/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-cyan-300"
              >
                Try again
              </button>
            </div>
          ) : instances.length === 0 ? (
            <div className="flex min-h-56 flex-col items-center justify-center rounded-3xl border border-dashed border-white/15 bg-white/[0.04] p-8 text-center">
              <Gamepad2 className="h-10 w-10 text-slate-500" aria-hidden="true" />
              <p className="mt-4 text-lg font-semibold">No instances yet</p>
              <p className="mt-2 max-w-md text-sm text-slate-400">Create or import an instance in the regular Agora view, then come back here to launch it from the couch.</p>
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-5 sm:grid-cols-2 lg:grid-cols-3" role="list" aria-label="Minecraft instances">
              {instances.map((instance, index) => {
                const selected = index === selectedIndex;
                return (
                  <div key={instance.instance_id} role="listitem">
                    <button
                      ref={(element) => { cardRefs.current[index] = element; }}
                      type="button"
                      tabIndex={selected ? 0 : -1}
                      aria-label={`Launch ${instance.name}`}
                      disabled={launchBusy}
                      onClick={() => { if (!launchBusy) void onLaunch(instance.instance_id); }}
                      className={`group min-h-52 w-full rounded-3xl border p-6 text-left transition focus-visible:outline focus-visible:outline-4 focus-visible:outline-cyan-300 disabled:cursor-wait disabled:opacity-60 ${selected ? 'border-cyan-300 bg-cyan-300/15 shadow-[0_0_0_2px_rgba(103,232,249,0.35),0_18px_60px_rgba(8,145,178,0.2)]' : 'border-white/10 bg-white/[0.05] hover:border-white/25 hover:bg-white/[0.09]'}`}
                    >
                      <div className="flex items-start justify-between gap-4">
                        <InstanceIcon
                          name={instance.name}
                          seed={instance.instance_id}
                          loader={instance.loader}
                          size={76}
                          className="rounded-2xl"
                        />
                        {selected && <span className="rounded-full bg-cyan-300 px-2.5 py-1 text-[10px] font-bold uppercase tracking-wider text-slate-950">Selected</span>}
                      </div>
                      <h2 className="mt-6 truncate text-xl font-bold text-white">{instance.name}</h2>
                      <p className="mt-2 text-sm text-slate-400">{instance.minecraft_version} · {formatLoader(instance)}</p>
                      <span className="mt-6 inline-flex items-center gap-2 text-sm font-semibold text-cyan-300 group-hover:text-cyan-200">
                        <Play className="h-4 w-4 fill-current" aria-hidden="true" />
                        {launchBusy && selected ? 'Starting…' : 'Launch'}
                      </span>
                    </button>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </main>

      <footer className="flex shrink-0 flex-wrap items-center justify-center gap-x-6 gap-y-2 border-t border-white/10 px-6 py-4 text-xs text-slate-400 sm:text-sm">
        <span><strong className="text-slate-200">D-pad / stick</strong> Move</span>
        <span><strong className="text-slate-200">A</strong> Launch</span>
        <span><strong className="text-slate-200">B</strong> Back</span>
        <span><strong className="text-slate-200">Start</strong> Toggle</span>
        <span className="inline-flex items-center gap-1 text-slate-300"><ArrowLeft className="h-3.5 w-3.5" aria-hidden="true" /><ArrowRight className="h-3.5 w-3.5" aria-hidden="true" /> Keyboard works too</span>
        <span className="text-amber-300/80">Esc exits</span>
      </footer>
    </div>
  );
}
