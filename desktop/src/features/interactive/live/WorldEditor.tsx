/**
 * WorldEditor — the High Interaction instance view, and the Simple one.
 *
 * Hero + shelf + pre-flight health check + crash doctor + advanced drawer +
 * Field Journal. The background ambience is a separate global layer; this
 * file is the *foreground* only.
 *
 * `presentation === 'simple'` renders the SAME structure with every
 * decorative and gamified system removed — see `decor` below for the single
 * list of what that means. Simple is a strict subset: nothing it turns off can
 * change how High Interaction behaves, and no branch adds anything Simple-only.
 *
 * The safety spine is unchanged in both: selection is the only local state;
 * every mutation-worthy action emits a `VisualIntent` that the host routes to
 * the reviewed Standard surface (remove → InstallFlow, health → review, crash →
 * CrashInvestigator, advanced rows → Standard editor). Nothing here executes
 * an operation directly.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import type { CapabilityFlags, VisualCrashEvidence, VisualId } from '../domain/models';
import type { VisualIntent } from '../domain/intents';
import type { LiveHostData } from './LiveSceneView';
import { buildEditorData, KIND_LABEL, monoOf, tileBackground, type EditorItem } from './worldEditorData';
import { tryEarnInteraction } from './interactionAchievements';
import { EMPTY_CONTENT_DETAIL, type ContentDetail } from './readAdapters';
import './world-editor.css';

export interface WorldEditorProps {
  data: LiveHostData;
  capabilities: CapabilityFlags;
  selection: VisualId | null;
  onSelect: (id: VisualId | null) => void;
  onIntent: (intent: VisualIntent) => void;
  onUseStandardView: () => void;
  onLaunch?: () => Promise<void> | void;
  launchAvailable?: boolean;
  reducedMotion?: boolean;
  presentation?: 'standard' | 'simple' | 'high-interaction';
  /** Description / categories / page link for the selected item. */
  selectedDetail?: ContentDetail;
  /** The enrichment read (health, evidence, runtime) is still in flight. */
  pending?: boolean;
  /** High-interaction inline crash experiment (stays in this view). */
  onTrialSuspect?: (suspectName: string) => Promise<{ snapshotId: string | null; disabled: string[]; error?: string }>;
  onUndoTrial?: (snapshotId: string) => Promise<void>;
}

type Filter = 'all' | 'mod' | 'look' | 'world';

interface Toast {
  id: number;
  icon: string;
  title: string;
  detail: string;
}

let toastSeq = 0;

function TileArtwork({ name, iconUrl }: { name: string; iconUrl?: string }) {
  return (
    <span className="tile-art" aria-hidden="true">
      <span className="tile-mono">{monoOf(name)}</span>
      {iconUrl ? (
        <img
          src={iconUrl}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={(event) => event.currentTarget.remove()}
        />
      ) : null}
    </span>
  );
}

const STEPS = ['Looking at your files', 'Checking your mods', 'Checking Minecraft version'];

/**
 * The scale the collection meter fills against.
 *
 * Agora has no hard cap on installed content — the registry deliberately chunks
 * its lookups at 500 ids to stay under SQLite's bound-variable limit, precisely
 * so a pack can be arbitrarily large. So this is a *reference scale*, not a
 * ceiling: 500 is the number the storage layer is built around, and it makes the
 * meter mean something. It used to render `total / total` at a hardcoded 100%,
 * which is why it always looked full and never appeared to move.
 */
const COLLECTION_SCALE = 500;

/**
 * How many dependency curves to draw at once.
 *
 * A hub library (Fabric API, Create) is needed by most of a pack, and capping at
 * a dozen hid exactly the thing that makes it a hub. The panel's "Needed by"
 * list is now complete and scrollable; this bound only stops the *drawing* from
 * becoming an unreadable starburst, so it is generous rather than tight.
 */
const MAX_CURVES = 40;

function confOf(strength: VisualCrashEvidence['hypotheses'][number]['strength']): number {
  return strength === 'high' ? 0.82 : strength === 'medium' ? 0.45 : 0.2;
}
function gaugeLabel(conf: number): string {
  return conf > 0.66 ? 'likely' : conf > 0.35 ? 'maybe' : 'unlikely';
}
function gaugeColor(conf: number): string {
  return conf > 0.66 ? 'var(--we-danger)' : conf > 0.35 ? 'var(--we-attention)' : 'var(--we-rare)';
}

export function WorldEditor({
  data,
  capabilities,
  onSelect,
  onIntent,
  onLaunch,
  launchAvailable = true,
  reducedMotion = false,
  presentation = 'high-interaction',
  selectedDetail = EMPTY_CONTENT_DETAIL,
  pending = false,
  onTrialSuspect,
  onUndoTrial,
}: WorldEditorProps) {
  /**
   * What Simple mode drops, in one place.
   *
   * Every one of these is decoration or a score — none of it is information
   * you cannot get from the panel, the findings list or the pre-flight check.
   *
   *  - `achievements`: the earn-a-badge toast layer (Field Guide entries too);
   *  - `meter`: the collection / XP progress bar;
   *  - `diagram`: the dependency curves drawn over the shelf, plus the
   *    dim-everything-else focus effect (the panel still lists Needs and
   *    Needed by, in full, as text);
   *  - `peek`: the tooltip that follows the cursor across the shelf;
   *  - `drag`: drag-to-rearrange the shelf.
   *
   * Simple also implies reduced motion — the mode pins the app-wide setting,
   * and `reduce` follows it here so the JS-driven animations (the pre-flight
   * scan wave, the shake, smooth scrolling) go with the CSS ones.
   */
  const simple = presentation === 'simple';
  const decor = {
    achievements: !simple,
    meter: !simple,
    diagram: !simple,
    peek: !simple,
    drag: !simple,
  };
  const reduce = reducedMotion || simple;
  const { scene } = data;
  const crash = data.crashEvidence.status === 'ok' ? data.crashEvidence.value : null;
  const editor = useMemo(
    () => buildEditorData(scene, data.crashEvidence.status === 'ok' && data.crashEvidence.value !== null),
    [scene, data.crashEvidence],
  );
  /**
   * Header findings, minus the optional-dependency notes.
   *
   * Every `recommendation` the health check emits is a
   * `MissingOptionalDependency` — the only variant of that kind — and those get
   * a dedicated "Optional dependencies (n)" button with its own overlay. Listing
   * them in the header too said "3 things need a look" about things that are by
   * definition optional and need nothing, which made the status line cry wolf
   * and buried the findings that do matter.
   */
  const findings = useMemo(
    () => scene.findings.filter((f) => f.severity !== 'recommendation'),
    [scene.findings],
  );
  const blocker = findings.find((f) => f.severity === 'blocker') ?? null;

  // ---- shelf state ----
  const [filter, setFilter] = useState<Filter>('all');
  const [query, setQuery] = useState('');
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [focused, setFocused] = useState(false);
  const [peek, setPeek] = useState<{ item: EditorItem; x: number; y: number } | null>(null);
  const [removed, setRemoved] = useState<EditorItem | null>(null);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [preflightOpen, setPreflightOpen] = useState(false);
  const [preflightResult, setPreflightResult] = useState<{ ok: boolean; running: boolean }>({ ok: false, running: false });
  const [doctorOpen, setDoctorOpen] = useState(false);
  const [pickedSuspect, setPickedSuspect] = useState<number | null>(null);
  const [doctorTrial, setDoctorTrial] = useState<null | { phase: 'working' | 'done' | 'error'; message: string; snapshotId: string | null; disabled: string[] }>(null);
  const [optionalOpen, setOptionalOpen] = useState(false);

  // order is local and mutable (drag-to-rearrange), reset when the data changes
  const orderRef = useRef<string[]>([]);
  const [orderVersion, setOrderVersion] = useState(0);
  useEffect(() => {
    orderRef.current = editor.items.map((it) => it.id);
    setSelectedName((cur) => (cur && editor.byId.has(cur) ? cur : null));
  }, [editor]);

  const orderedItems = useMemo(() => {
    const order = orderRef.current;
    const byId = new Map(editor.items.map((it) => [it.id, it]));
    const ordered = order
      .map((id) => byId.get(id))
      .filter((it): it is EditorItem => Boolean(it))
      .filter((it) => !removedMatching(it));
    const rest = editor.items.filter((it) => ordered.indexOf(it) < 0 && !removedMatching(it));
    return ordered.concat(rest);
    // orderVersion is the cosmetic reorder signal; orderRef is mutated in place
    // during drag, so the memo must re-run when the version ticks.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, removed, orderVersion]);

  function removedMatching(it: EditorItem): boolean {
    return removed?.id === it.id;
  }

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return orderedItems.filter((it) => {
      if (removed && removed.id === it.id) return false;
      if (filter !== 'all' && it.kind !== filter) return false;
      if (q && it.name.toLowerCase().indexOf(q) < 0) return false;
      return true;
    });
  }, [orderedItems, filter, query, removed]);

  // ---- status pill ----
  const instance = scene.instance;
  const launchState = instance?.launchState ?? 'idle';
  const operationBusy = instance?.lockState === 'busy';
  const playerLocked = instance?.lockState === 'locked-by-player';
  const runningState = launchState === 'starting' || launchState === 'running' || launchState === 'stopping' || launchState === 'delegated';
  // Locking protects the files from edits but deliberately does not block play.
  // The Standard editor follows the same policy; only an active process or
  // operation, an unavailable Standard launch gate, or a missing handler blocks.
  const playDisabled = runningState || operationBusy || !launchAvailable || !onLaunch;

  // While the enrichment read is in flight the health fragment has no value
  // YET — which is not the same as a health scan that ran and failed. Saying
  // "could not be verified" for the first second of every visit would be a
  // false alarm, so the pending case gets its own honest wording.
  const healthUnverified = !pending && data.health.status !== 'ok';
  const statusText = pending
    ? 'Checking things over…'
    : editor.hasCrash
      ? 'Your game stopped last time — find out why'
      : healthUnverified
        ? 'Health could not be verified'
        : blocker
          ? 'One mod is missing its file'
          : findings.length > 0
            ? `${findings.length} thing${findings.length === 1 ? '' : 's'} need a look`
            : 'Everything looks ready';
  const statusOk = !pending && !editor.hasCrash && !healthUnverified && !blocker && findings.length === 0;


  const achievementsOn = decor.achievements;
  const achieve = useCallback((icon: string, title: string, detail: string, key: string) => {
    // Simple mode does not keep score: nothing is earned and nothing toasts,
    // so ordinary use never leaves a trail in the Field Guide either.
    if (!achievementsOn) return;
    // Persisted interaction achievement: only the FIRST earn toasts, so the
    // same action can never re-announce itself (Field Guide shows the rest).
    if (!tryEarnInteraction(key)) return;
    setToasts((cur) => [...cur.slice(-3), { id: ++toastSeq, icon, title, detail }]);
  }, [achievementsOn]);
  useEffect(() => {
    if (!toasts.length) return;
    const last = toasts[toasts.length - 1];
    const t = window.setTimeout(() => setToasts((cur) => cur.filter((x) => x.id !== last.id)), 4200);
    return () => window.clearTimeout(t);
  }, [toasts]);

  // ---- selection + detail ----
  const selectedItem = selectedName ? editor.byId.get(selectedName) ?? null : null;
  const selectedDetailId = selectedItem?.catalogIds?.registryId
    || selectedItem?.catalogIds?.modrinthId
    || selectedItem?.catalogIds?.modJarId
    || null;
  const relatedIds = useMemo(() => {
    if (!selectedItem) return new Set<string>();
    const rel = new Set<string>([selectedItem.id]);
    editor.items.forEach((it) => {
      if (it.needs.some((n) => n === selectedItem.name) || it.neededBy.some((n) => n === selectedItem.name)) rel.add(it.id);
    });
    return rel;
  }, [selectedItem, editor]);

  /**
   * Draw the neighbourhood curves for the current selection.
   *
   * Real quadratic curves between the selected tile and each tile it needs or
   * is needed by — the bounded diagram the plan asks for (§ "Bounded
   * neighbourhood diagram on selection (real curves, capped node count)").
   * Endpoints come from measured layout, so this runs after paint and re-runs
   * on scroll and resize.
   *
   * Direction is encoded, not decorative: the travelling dot always moves from
   * the dependent TO the thing it depends on, so the curve answers "which way
   * does this need point" at a glance.
   */
  const diagramOn = decor.diagram;
  const drawLinks = useCallback(() => {
    const svg = linksRef.current;
    const wrap = gridRef.current;
    if (!svg || !wrap) return;
    while (svg.firstChild) svg.removeChild(svg.firstChild);
    // Simple mode has no curves to draw. The clear still runs above, so a mode
    // switch never leaves a stale strand painted over the shelf.
    if (!diagramOn) return;
    if (!selectedItem) return;

    const box = wrap.getBoundingClientRect();
    svg.setAttribute('viewBox', `0 0 ${Math.round(box.width)} ${Math.round(box.height)}`);
    const centreOf = (name: string) => {
      const el = wrap.querySelector(`.we-slot[data-name="${CSS.escape(name)}"] .tile`) as HTMLElement | null;
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: r.left - box.left + r.width / 2 + wrap.scrollLeft, y: r.top - box.top + r.height / 2 + wrap.scrollTop };
    };
    const from = centreOf(selectedItem.name);
    if (!from) return;

    // Cap the drawn neighbourhood: past a dozen strands it reads as noise
    // rather than structure, and the tile highlight already conveys the rest.
    const partners = editor.items
      .filter((it) => it.id !== selectedItem.id
        && (it.needs.includes(selectedItem.name) || it.neededBy.includes(selectedItem.name)))
      .slice(0, MAX_CURVES);

    const NS = 'http://www.w3.org/2000/svg';
    partners.forEach((it, i) => {
      const to = centreOf(it.name);
      if (!to) return;
      // `it.needs` contains the selection => the arrow runs it -> selection.
      const dependentFirst = it.needs.includes(selectedItem.name);
      const a = dependentFirst ? to : from;
      const b = dependentFirst ? from : to;
      // Bow the curve perpendicular to the run so parallel strands stay legible.
      const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
      const dx = b.x - a.x, dy = b.y - a.y;
      const len = Math.hypot(dx, dy) || 1;
      const bow = Math.min(46, len * 0.22);
      const cx = mx + (-dy / len) * bow;
      const cy = my + (dx / len) * bow;
      const d = `M ${a.x} ${a.y} Q ${cx} ${cy} ${b.x} ${b.y}`;

      const path = document.createElementNS(NS, 'path');
      path.setAttribute('d', d);
      path.setAttribute('class', dependentFirst ? 'we-link needs' : 'we-link needed');
      svg.appendChild(path);

      if (!reduce) {
        const dot = document.createElementNS(NS, 'circle');
        dot.setAttribute('r', '3');
        dot.setAttribute('class', 'we-link-dot');
        const motion = document.createElementNS(NS, 'animateMotion');
        motion.setAttribute('dur', '1.9s');
        motion.setAttribute('repeatCount', 'indefinite');
        motion.setAttribute('path', d);
        motion.setAttribute('begin', `${(i * 0.16).toFixed(2)}s`);
        dot.appendChild(motion);
        svg.appendChild(dot);
      }
    });
  }, [selectedItem, editor, reduce, diagramOn]);

  useEffect(() => {
    // after paint, so the tiles have their final positions
    const id = requestAnimationFrame(drawLinks);
    return () => cancelAnimationFrame(id);
  }, [drawLinks, filter, query, visible]);

  useEffect(() => {
    const wrap = gridRef.current;
    // No curves in Simple mode, so no scroll/resize remeasuring either.
    if (!wrap || !diagramOn) return;
    const on = () => drawLinks();
    wrap.addEventListener('scroll', on, { passive: true });
    window.addEventListener('resize', on);
    return () => { wrap.removeEventListener('scroll', on); window.removeEventListener('resize', on); };
  }, [drawLinks, diagramOn]);

  const selectItem = useCallback((name: string | null) => {
    setSelectedName(name);
    // The dim-everything-else focus effect belongs to the diagram: without the
    // curves it just greys out the shelf for no stated reason.
    setFocused(diagramOn && name !== null);
    if (name) {
      const item = editor.byId.get(name);
      if (item) onSelect(item.id);
    } else {
      onSelect(null);
    }
  }, [editor, onSelect, diagramOn]);

  const removeItem = useCallback(() => {
    if (!selectedItem) return;
    setRemoved(selectedItem);
    achieve('🧹', 'Tidied up', 'Removed ' + selectedItem.name, 'tidied-up');
    selectItem(null);
    onIntent({ kind: 'propose-remove', contentId: selectedItem.id });
  }, [selectedItem, achieve, selectItem, onIntent]);

  const undoRemove = useCallback(() => {
    setRemoved(null);
    achieve('↩️', 'Second thoughts', 'Put it back', 'second-thoughts');
  }, [achieve]);

  // ---- hover peek ----
  const peekOn = decor.peek;
  const onTileMove = useCallback((e: React.MouseEvent, item: EditorItem, tileEl: HTMLElement) => {
    if (!peekOn) return;
    if (reduce) return;
    const r = tileEl.getBoundingClientRect();
    const px2 = (e.clientX - r.left) / r.width;
    const py2 = (e.clientY - r.top) / r.height;
    tileEl.style.transform = `translateY(-6px) rotateX(${(0.5 - py2) * 20}deg) rotateY(${(px2 - 0.5) * 20}deg) scale(1.09)`;
    tileEl.style.setProperty('--mx', `${px2 * 100}%`);
    tileEl.style.setProperty('--my', `${py2 * 100}%`);
    setPeek({ item, x: Math.min(e.clientX + 18, window.innerWidth - 266), y: Math.min(e.clientY + 18, window.innerHeight - 100) });
  }, [reduce, peekOn]);

  const onTileLeave = useCallback((tileEl: HTMLElement) => {
    tileEl.style.transform = '';
    setPeek(null);
  }, []);

  // ---- drag to rearrange (high-interaction only, purely cosmetic) ----
  // Root cause of the old "freeze": the tiles contain <img> mod icons, and a
  // press-and-move over an image starts a NATIVE HTML5 drag in WebView2. The
  // native drag cancels the pointer stream (pointercancel, no pointerup), so
  // the drag appeared frozen until a later click unstuck it. Cancelling
  // `dragstart` on the grid (below) plus pointercancel handling keeps the
  // pointer stream ours: hold to lift, move to reorder, release to drop.
  const gridRef = useRef<HTMLDivElement>(null);
  const linksRef = useRef<SVGSVGElement>(null);
  const dragState = useRef<{ id: string; el: HTMLElement } | null>(null);
  const justDragged = useRef(0);

  const beginDrag = useCallback((slot: HTMLElement, id: string) => {
    dragState.current = { id, el: slot };
    slot.classList.add('dragging');
    gridRef.current?.classList.add('arranging');
    achieve('🧩', 'Rearrange', 'Rearranged the shelf', 'rearrange');
  }, [achieve]);

  const onDragMove = useCallback((e: PointerEvent) => {
    const drag = dragState.current;
    if (!drag) return;
    const ghost = document.querySelector('.we-ghost') as HTMLElement | null;
    if (ghost) {
      ghost.style.left = `${e.clientX - 42}px`;
      ghost.style.top = `${e.clientY - 42}px`;
    }
    const over = document.elementFromPoint(e.clientX, e.clientY);
    const target = over?.closest?.('.we-slot') as HTMLElement | null;
    if (!target || target === drag.el) return;
    const targetName = target.dataset.name ?? '';
    const targetItem = editor.byId.get(targetName);
    const toId = targetItem?.id ?? targetName;
    const from = orderRef.current.indexOf(drag.id);
    const to = orderRef.current.indexOf(toId);
    if (from < 0 || to < 0 || from === to) return;
    // purely cosmetic: splice the local order and let React re-render
    orderRef.current.splice(to, 0, orderRef.current.splice(from, 1)[0]);
    setOrderVersion((v) => v + 1);
  }, [editor]);

  const endDrag = useCallback(() => {
    const drag = dragState.current;
    if (!drag) return;
    drag.el.classList.remove('dragging');
    gridRef.current?.classList.remove('arranging');
    const ghost = document.querySelector('.we-ghost') as HTMLElement | null;
    ghost?.classList.remove('on');
    dragState.current = null;
    justDragged.current = Date.now();
  }, []);

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0 || reduce || !decor.drag) return;
    const slot = (e.target as HTMLElement).closest?.('.we-slot') as HTMLElement | null;
    if (!slot) return;
    const name = slot.dataset.name ?? '';
    const item = editor.byId.get(name);
    if (!item) return;
    const id = item.id;
    const startX = e.clientX;
    const startY = e.clientY;
    let hasStarted = false;
    let timer: number | null = null;

    const ensureStarted = (clientX: number, clientY: number) => {
      if (hasStarted) return;
      hasStarted = true;
      if (timer !== null) {
        window.clearTimeout(timer);
        timer = null;
      }
      const ghost = document.querySelector('.we-ghost') as HTMLElement | null;
      if (ghost) {
        ghost.style.background = tileBackground(name);
        ghost.textContent = monoOf(name);
        ghost.classList.add('on');
        ghost.style.left = `${clientX - 42}px`;
        ghost.style.top = `${clientY - 42}px`;
      }
      beginDrag(slot, id);
    };

    // Hold-to-pickup: even without movement, a sustained press lifts the tile
    timer = window.setTimeout(() => {
      ensureStarted(startX, startY);
    }, 140);

    const onMove = (ev: PointerEvent) => {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      const moved = Math.hypot(dx, dy) > 8;
      if (!hasStarted && moved) {
        ensureStarted(ev.clientX, ev.clientY);
      }
      if (hasStarted) {
        onDragMove(ev);
      }
    };

    const onUp = (ev: PointerEvent) => {
      cleanup();
      if (!hasStarted) return;
      const ghost = document.querySelector('.we-ghost') as HTMLElement | null;
      if (ghost) {
        ghost.style.left = `${ev.clientX - 42}px`;
        ghost.style.top = `${ev.clientY - 42}px`;
      }
      endDrag();
    };

    // If the pointer stream is cancelled (native gesture, alt-tab, pen/touch
    // takeover), end the drag NOW instead of stranding it mid-shelf.
    const onCancel = () => {
      cleanup();
      if (hasStarted) endDrag();
    };

    const cleanup = () => {
      if (timer !== null) {
        window.clearTimeout(timer);
        timer = null;
      }
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onCancel);
    };

    // Window-level listeners: the pointer may travel off the grid entirely,
    // and element-level capture once swallowed pointerup and stranded the
    // drag until a second click.
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp, { once: true });
    window.addEventListener('pointercancel', onCancel, { once: true });
  }, [reduce, decor.drag, editor, beginDrag, onDragMove, endDrag]);

  const onSlotClick = useCallback((name: string) => {
    if (Date.now() - justDragged.current < 250) return;
    if (selectedName === name) {
      selectItem(null);
      return;
    }
    achieve('🔍', 'Curious', 'Looking closer', 'curious');
    selectItem(name);
  }, [selectedName, selectItem, achieve]);

  // ---- preflight ----
  const runPreflight = useCallback(() => {
    setPreflightOpen(true);
    setPreflightResult({ ok: false, running: true });
    const order = visible.slice(0, 72);
    const stepEls = STEPS.map((_, i) => document.querySelector(`[data-pf-step="${i}"]`));
    const scanAt = (i: number, k: number) => {
      const name = order[Math.floor(i * order.length / 3) + k]?.name;
      const slot = name ? document.querySelector(`.we-slot[data-name="${CSS.escape(name)}"]`) : null;
      if (!slot || reduce) return;
      slot.classList.add('scanned');
      window.setTimeout(() => slot.classList.remove('scanned'), 450);
    };
    const step = (i: number) => {
      if (i >= STEPS.length) return finish(true);
      stepEls[i]?.classList.add('active');
      const sliceStart = Math.floor(i * order.length / 3);
      const sliceEnd = Math.floor((i + 1) * order.length / 3);
      for (let k = sliceStart; k < sliceEnd; k++) {
        window.setTimeout(() => scanAt(i, k - sliceStart), (k - sliceStart) * 13);
      }
      window.setTimeout(() => {
        stepEls[i]?.classList.remove('active');
        if (i === 1 && blocker) {
          stepEls[i]?.classList.add('bad');
          const mark = stepEls[i]?.querySelector('.mark');
          if (mark) mark.textContent = '!';
          const missing = editor.missingItem;
          if (missing) {
            const slot = document.querySelector(`.we-slot[data-name="${CSS.escape(missing.name)}"]`);
            if (slot) {
              slot.classList.add('shake');
              slot.scrollIntoView({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
              window.setTimeout(() => slot.classList.remove('shake'), 600);
            }
          }
          return finish(false);
        }
        stepEls[i]?.classList.add('done');
        const mark = stepEls[i]?.querySelector('.mark');
        if (mark) mark.textContent = '✓';
        step(i + 1);
      }, reduce ? 250 : 950);
    };
    const finish = (ok: boolean) => {
      setPreflightResult({ ok, running: false });
      if (ok) achieve('✅', 'All clear', 'Ready to play', 'all-clear');
    };
    step(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, blocker, editor, reduce, achieve]);

  const launch = useCallback(() => {
    setPreflightOpen(false);
    if (onLaunch) void onLaunch();
  }, [onLaunch]);

  const showMe = useCallback(() => {
    setPreflightOpen(false);
    setFilter('all');
    setQuery('');
    if (editor.missingItem) {
      const name = editor.missingItem.name;
      const el = document.querySelector(`.we-slot[data-name="${CSS.escape(name)}"]`);
      el?.scrollIntoView({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
      selectItem(name);
    }
  }, [editor, selectItem, reduce]);

  // ---- crash doctor ----
  const suspects = useMemo(() => {
    if (!crash) return [];
    return crash.hypotheses.map((h) => ({
      name: h.title,
      conf: confOf(h.strength),
      why: h.supportingClues[0] ?? 'A possible cause, not a certainty.',
    }));
  }, [crash]);

  const openDoctor = useCallback(() => {
    setPickedSuspect(null);
    setDoctorTrial(null);
    setDoctorOpen(true);
    achieve('🩺', 'Called the doctor', 'Crash Doctor', 'called-doctor');
  }, [achieve]);

  const doctorTry = useCallback(async () => {
    if (pickedSuspect === null) return;
    const suspectName = suspects[pickedSuspect]?.name;
    if (!suspectName) return;
    // Prefer inline handling when the host provides it; otherwise fall back to
    // the reviewed Standard CrashInvestigator.
    if (onTrialSuspect) {
      setDoctorTrial({ phase: 'working', message: 'Saving a return point first…', snapshotId: null, disabled: [] });
      try {
        const result = await onTrialSuspect(suspectName);
        if (result.error) {
          setDoctorTrial({ phase: 'error', message: result.error, snapshotId: result.snapshotId ?? null, disabled: result.disabled });
        } else {
          setDoctorTrial({
            phase: 'done',
            message: result.disabled.length > 1
              ? `Turned off ${result.disabled.join(', ')} — press Play to see if it helps. You can undo.`
              : `Turned off ${suspectName} — press Play to see if it helps. You can undo.`,
            snapshotId: result.snapshotId ?? null,
            disabled: result.disabled,
          });
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setDoctorTrial({ phase: 'error', message: msg, snapshotId: null, disabled: [] });
      }
      return;
    }
    // Fallback: the reviewed experiment lives in the Standard CrashInvestigator.
    setDoctorOpen(false);
    onIntent({ kind: 'open-crash-doctor' });
  }, [pickedSuspect, suspects, onTrialSuspect, onIntent]);

  const statusClick = useCallback(() => {
    if (editor.hasCrash) { openDoctor(); return; }
    if (statusOk) return;
    if (blocker) { showMe(); return; }
    runPreflight();
  }, [editor.hasCrash, statusOk, blocker, showMe, runPreflight, openDoctor]);

  const playClick = useCallback(() => {
    if (playDisabled) return;
    const btn = reduce ? null : (document.querySelector('.we-play') as HTMLElement | null);
    if (btn) {
      btn.classList.remove('squash');
      void btn.offsetWidth;
      btn.classList.add('squash');
    }
    achieve('🎮', 'Let\'s go', 'Pressed the big button', 'lets-go');
    runPreflight();
  }, [playDisabled, runPreflight, achieve, reduce]);

  // ---- optional dependencies overlay ----
  const optionalRows = useMemo(() => {
    return editor.items
      .filter((it) => it.optional.length > 0)
      .map((it) => ({ owner: it, options: it.optional }));
  }, [editor]);
  const optionalCount = useMemo(() => {
    const ids = new Set<string>();
    editor.items.forEach((it) => it.optional.forEach((o) => ids.add(o)));
    return ids.size;
  }, [editor]);

  return (
    <div className="world-editor" data-testid="world-editor" data-source={scene.source.kind} data-presentation={presentation} data-launch-state={launchState} data-lock-state={instance?.lockState ?? 'editable'}>
      {/* ── hero ── */}
      <section className="we-panel we-hero">
        <div className="we-logo-hold">
          {decor.meter ? <div className="we-ring" aria-hidden="true" /> : null}
          <div
            className="we-logo"
            style={{ background: instance?.name ? tileBackground(instance.name) : 'linear-gradient(160deg,hsl(174 52% 48%),hsl(190 71% 28%))' }}
            aria-hidden="true"
          >
            {instance?.name ? monoOf(instance.name) : '?'}
          </div>
        </div>
        <div>
          <h1 className="we-title">{instance?.name ?? 'Untitled'}</h1>
          <div className="we-meta">
            <span className="we-chip">Minecraft {instance?.gameVersion && instance.gameVersion !== '—' ? instance.gameVersion : '—'}</span>
            <span className="we-chip tab-num">{editor.total} thing{editor.total === 1 ? '' : 's'} inside</span>
            <span className="we-chip">
              {instance?.loader.current.family && instance.loader.current.family !== '—' ? instance.loader.current.family : 'No loader'}
            </span>
            {operationBusy ? <span className="we-chip">Busy</span> : playerLocked ? <span className="we-chip">Locked</span> : null}
          </div>
          {/* The collection meter is a score, not a fact — the chip above already
              says how many things are inside — so Simple mode drops it. */}
          {decor.meter ? (
            <div className="we-xp">
              <div className="we-xp-top">
                <span>Collection · {editor.total} {editor.total === 1 ? 'item' : 'items'}</span>
                <span className="tab-num">{editor.total} / {COLLECTION_SCALE}</span>
              </div>
              <div className="we-xp-bar">
                <div
                  className="we-xp-fill"
                  style={{ width: `${Math.min(100, (editor.total / COLLECTION_SCALE) * 100).toFixed(1)}%` }}
                />
              </div>
            </div>
          ) : null}
          <button type="button" className={`we-status ${statusOk ? 'ok' : ''}`} onClick={statusClick}>
            <span className="orb" />
            <span>{statusText}</span>
          </button>
          {findings.length > 0 ? (
            <ul className="we-findings" data-testid="we-findings" style={{ listStyle: 'none', margin: '10px 0 0', padding: 0, display: 'grid', gap: 6, maxWidth: 420 }}>
              {findings.map((f) => (
                <li key={f.id} style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 'calc(12.5px * var(--font-scale))', color: 'hsl(var(--muted-foreground))' }}>
                  <span className={`we-finding-dot`} style={{ width: 8, height: 8, borderRadius: '50%', background: f.severity === 'blocker' ? 'var(--we-danger)' : f.severity === 'warning' ? 'var(--we-attention)' : 'hsl(174 52% 48%)', flex: 'none' }} aria-hidden="true" />
                  <span>{f.title}</span>
                  {f.reviewIntent ? (
                    <button
                      type="button"
                      className="we-mini"
                      onClick={() => onIntent(f.reviewIntent!)}
                      disabled={f.reviewIntent.kind === 'review-health' ? !capabilities.canReviewHealth : !capabilities.canReviewLoader}
                    >
                      Review
                    </button>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        <button type="button" className="we-play" onClick={playClick} disabled={playDisabled} aria-label="Play this world">
          {decor.meter ? <span className="shine" aria-hidden="true" /> : null}
          <span className="tri" aria-hidden="true" />
          {runningState ? (launchState === 'delegated' ? 'Delegated' : launchState === 'running' ? 'Running' : 'Starting…') : 'Play'}
        </button>
      </section>

      {/* ── shelf ── */}
      <section className="we-panel">
        <div className="we-shelf-head">
          <h2 className="we-shelf-title">What's inside</h2>
          <input
            className="we-find"
            type="search"
            placeholder="Find something…"
            aria-label="Find something inside this world"
            value={query}
            onChange={(e) => { setQuery(e.target.value); if (e.target.value) achieve('🔎', 'Searcher', 'Found it', 'searcher'); }}
          />
          {optionalCount > 0 ? (
            <button
              type="button"
              className="we-optional-btn"
              onClick={() => setOptionalOpen(true)}
              data-testid="we-optional-btn"
            >
              Optional dependencies ({optionalCount})
            </button>
          ) : (
            <button
              type="button"
              className="we-optional-btn"
              onClick={() => setOptionalOpen(true)}
              data-testid="we-optional-btn"
            >
              Optional dependencies
            </button>
          )}
          <div className="we-tabs" role="tablist" aria-label="Filter contents">
            {([['all', 'All'], ['mod', 'Mods'], ['look', 'Looks'], ['world', 'Worlds']] as Array<[Filter, string]>).map(([k, label]) => (
              <button
                key={k}
                type="button"
                role="tab"
                aria-selected={filter === k}
                className="we-tab"
                onClick={() => { setFilter(k); achieve('🗂️', 'Sorted it out', label, 'sorted-it-out'); selectItem(null); }}
              >
                {label}<span className="n">{editor.counts[k] ?? 0}</span>
              </button>
            ))}
          </div>
        </div>
        <div className={`we-gridwrap ${focused ? 'focusing' : ''}`} ref={gridRef} onPointerLeave={() => setPeek(null)}>
          {/* Dependency curves are drawn here, above the tiles but click-through.
              Populated imperatively by the effect below because the endpoints are
              measured from laid-out DOM, which React cannot express declaratively.
              Simple mode has no curves, so it has no canvas for them either. */}
          {decor.diagram ? <svg className="we-links" ref={linksRef} aria-hidden="true" /> : null}
          <div
            className="we-grid"
            onPointerDown={onPointerDown}
            onDragStart={(e) => e.preventDefault()}
            data-testid="we-grid"
          >
            {visible.map((it, idx) => {
              const lit = focused && relatedIds.has(it.id);
              const pressed = selectedName === it.name;
              return (
                <div
                  key={it.id}
                  role="button"
                  tabIndex={0}
                  className={`we-slot ${focused ? (lit ? 'lit' : '') : ''}`}
                  style={{ ['--i' as string]: Math.min(idx, 70) }}
                  aria-pressed={pressed}
                  aria-label={`${it.name}${it.missing ? ' — missing its file' : ''}`}
                  data-name={it.name}
                  data-testid="we-slot"
                  onClick={() => onSlotClick(it.name)}
                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSlotClick(it.name); } }}
                  onMouseMove={(e) => onTileMove(e, it, e.currentTarget.querySelector('.tile') as HTMLElement)}
                  onMouseLeave={(e) => onTileLeave(e.currentTarget.querySelector('.tile') as HTMLElement)}
                >
                  <span className="tile" style={{ background: tileBackground(it.name) }}>
                    <TileArtwork name={it.name} iconUrl={it.kind === 'mod' ? it.iconUrl : undefined} />
                  </span>
                  {it.missing ? <span className="flag" aria-hidden="true">!</span> : null}
                  <span className="cap">{it.name}</span>
                  {it.presence === 'not-installed' && capabilities.canProposeInstall ? (
                    <button
                      type="button"
                      className="we-stage"
                      onClick={(e) => { e.stopPropagation(); onIntent({ kind: 'propose-install', contentId: it.id }); }}
                    >
                      Stage install: {it.name}
                    </button>
                  ) : null}
                </div>
              );
            })}
            {visible.length === 0 && (
              <p style={{ gridColumn: '1 / -1', textAlign: 'center', color: 'hsl(var(--muted-foreground))', fontSize: 'calc(13px * var(--font-scale))', padding: '24px 0' }}>
                {query ? 'Nothing matches that search.' : 'Nothing here yet.'}
              </p>
            )}
          </div>
        </div>
      </section>

      {/* ── field journal moved to its own page (Field Guide) ── */}

      {/* ── hover peek ── */}
      <div className={`we-peek ${peek ? 'show' : ''}`} style={peek ? { left: peek.x, top: peek.y } : undefined}>
        {peek && (
          <>
            <div className="pn">{peek.item.name}</div>
            <div className="pk">{KIND_LABEL[peek.item.kind]}</div>
            <div className="pr">
              {peek.item.neededBy.length ? `${peek.item.neededBy.length} thing${peek.item.neededBy.length === 1 ? '' : 's'} need this` : peek.item.needs.length ? `needs ${peek.item.needs.length}` : ''}
            </div>
          </>
        )}
      </div>

      {/* ── detail drawer ──
          Rendered into <body> on purpose. `position: fixed` is measured against
          the nearest ancestor with a filter/transform, and `main` carries a
          backdrop-filter for the glass look over the living world — so in place
          this drawer behaved like an absolutely-positioned child of the scroll
          container and slid away with the page instead of staying in view. */}
      {selectedItem && createPortal(
        <aside className={`we-detail ${selectedItem ? 'open' : ''}`} aria-live="polite" data-testid="we-detail">
          <button className="we-closeX" aria-label="Close" onClick={() => selectItem(null)}>×</button>
          <div className="we-bigtile" style={{ background: tileBackground(selectedItem.name) }}>
            <TileArtwork name={selectedItem.name} iconUrl={selectedItem.kind === 'mod' ? selectedItem.iconUrl : undefined} />
          </div>
          <h3>{selectedItem.name}</h3>
          {/* Categories first: what KIND of thing this is answers the question a
              player actually opened the panel with. */}
          {selectedDetail.categories.length > 0 ? (
            <div className="we-cats">
              {selectedDetail.categories.map((c) => (
                <span key={c} className="we-cat">{c.replace(/[-_]/g, ' ')}</span>
              ))}
            </div>
          ) : null}
          <p className="kind">{KIND_LABEL[selectedItem.kind]}</p>
          {selectedDetail.description ? (
            <p className="we-desc">
              {selectedDetail.description}
              <span className="we-desc-src">
                {selectedDetail.source === 'agora' ? 'from Agora' : 'from Modrinth'}
              </span>
            </p>
          ) : null}
          {selectedItem.missing ? (
            <div className="we-warnbox">This mod's file is missing, so it will not load. Reinstalling the pack puts it back.</div>
          ) : null}
          {selectedItem.needs.length > 0 ? (
            <div className="we-rel">
              <h4>Needs</h4>
              <ul>{selectedItem.needs.map((n) => <li key={n}>{n}</li>)}</ul>
            </div>
          ) : null}
          {selectedItem.neededBy.length > 0 ? (
            <div className="we-rel">
              <h4>Needed by <span className="we-rel-count">{selectedItem.neededBy.length}</span></h4>
              {/* A library like Fabric API is needed by most of the pack; showing
                  8 of 40 hides exactly the fact that makes it important. The list
                  scrolls instead of truncating. */}
              <ul className="we-rel-scroll">
                {selectedItem.neededBy.map((n) => <li key={n}>{n}</li>)}
              </ul>
            </div>
          ) : null}
          {selectedItem.neededBy.length >= 10 ? <span style={{ display: 'none' }} data-testid="keystone" /> : null}
          <div className="spacer" />
          <button
            type="button"
            className="we-btn"
            onClick={() => {
              if (selectedDetailId) onIntent({ kind: 'open-standard', destination: { type: 'mod-detail', itemId: selectedDetailId } });
            }}
            disabled={!selectedDetailId}
          >
            Open full details
          </button>
          <button type="button" className="we-btn danger" onClick={removeItem} disabled={!capabilities.canProposeRemove}>
            {capabilities.canProposeRemove ? 'Remove from this world' : 'Removal opens the Standard screen'}
          </button>
        </aside>,
        document.body,
      )}

      {/* ── optional dependencies overlay ──
          Portalled for the same reason as the drawer: a fixed overlay inside a
          backdrop-filtered scroll container is not actually fixed. */}
      {createPortal(
      <div
        className={`we-scrim ${optionalOpen ? 'show' : ''}`}
        // controller-exempt: click-outside backdrop, not a control; the dialog
        // carries its own dismiss control.
        onClick={(e) => { if (e.target === e.currentTarget) setOptionalOpen(false); }}
      >
        <div className="we-doc we-optional" role="dialog" aria-modal="true" aria-label="Optional dependencies">
          <h3>Optional dependencies</h3>
          <p className="sub">A few things in here can use extra add-ons. They're never required — add the ones you want.</p>
          {optionalRows.length === 0 ? (
            <p className="sub">Nothing optional right now.</p>
          ) : (
            <div className="we-optional-list">
              {optionalRows.map(({ owner, options }) => (
                <div key={owner.id} className="we-optional-group">
                  <h4>{owner.name} <span className="sub">recommends</span></h4>
                  <ul>
                    {options.map((name) => {
                      const opt = editor.byId.get(name);
                      const already = opt && opt.presence === 'installed';
                      return (
                        <li key={name} className="we-opt-row">
                          <span>{name}{already ? ' ✓ in this world' : ''}</span>
                          {opt && opt.presence === 'not-installed' && capabilities.canProposeInstall ? (
                            <button
                              type="button"
                              className="we-opt-add"
                              onClick={() => onIntent({ kind: 'propose-install', contentId: opt.id })}
                            >
                              Add to list
                            </button>
                          ) : null}
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ))}
            </div>
          )}
          <div className="we-pf-actions">
            <button type="button" className="we-btn" onClick={() => setOptionalOpen(false)}>Done</button>
          </div>
        </div>
      </div>,
      document.body,
      )}

      {/* ── preflight ── */}
      <div
        className={`we-scrim ${preflightOpen ? 'show' : ''}`}
        // controller-exempt: click-outside backdrop, not a control; the dialog
        // carries its own dismiss control.
        onClick={(e) => { if (e.target === e.currentTarget) setPreflightOpen(false); }}
      >
        <div className="we-preflight" role="dialog" aria-modal="true" aria-label="Getting ready">
          <h3>{preflightResult.ok ? 'Ready to play!' : 'Getting ready…'}</h3>
          <p className="sub">A quick look before you play.</p>
          <ul className="we-steps">
            {STEPS.map((s, i) => (
              <li key={s} className="we-step" data-pf-step={i}>
                <span className="mark">{i + 1}</span>
                {s}
              </li>
            ))}
          </ul>
          <div className="we-pf-actions">
            {preflightResult.running ? null : preflightResult.ok ? (
              <button type="button" className="we-btn go" onClick={launch}>Launch Minecraft</button>
            ) : (
              <>
                <button type="button" className="we-btn" onClick={showMe}>Show me</button>
                <button type="button" className="we-btn go" onClick={launch}>Play anyway</button>
              </>
            )}
          </div>
        </div>
      </div>

      {/* ── crash doctor ── */}
      <div
        className={`we-scrim ${doctorOpen ? 'show' : ''}`}
        // controller-exempt: click-outside backdrop, not a control; the dialog
        // carries its own dismiss control.
        onClick={(e) => { if (e.target === e.currentTarget) setDoctorOpen(false); }}
      >
        <div className="we-doc" role="dialog" aria-modal="true" aria-label="Crash Doctor">
          <h3>Your game stopped</h3>
          <p className="sub">
            {doctorTrial
              ? (doctorTrial.phase === 'working' ? 'Trying one thing at a time…' : doctorTrial.phase === 'done' ? 'We turned it off — you can test it.' : 'Something didn’t work.')
              : 'Let’s work out which mod did it. Pick the one you want to test first.'}
          </p>
          {!doctorTrial && suspects.length === 0 ? (
            <p className="sub" style={{ marginTop: 8 }}>We couldn’t tell which mod caused this. It might not be a mod issue — try the Standard view for the full logs.</p>
          ) : null}
          {!doctorTrial && suspects.length > 0 ? (
            <>
              <div className="we-suspects">
                {suspects.map((s, i) => {
                  const c = 2 * Math.PI * 20;
                  return (
                    <button
                      key={s.name}
                      type="button"
                      className="we-susp"
                      aria-pressed={pickedSuspect === i}
                      onClick={() => { setPickedSuspect(i); achieve('🕵️', 'Detective', 'Picked a crash suspect', 'suspect'); }}
                    >
                      <span className="st" style={{ background: tileBackground(s.name) }}>{monoOf(s.name)}</span>
                      <span>
                        <span className="sn">{s.name}</span><br />
                        <span className="sw">{s.why}</span>
                      </span>
                      <span className="we-gauge">
                        <svg width="52" height="52" aria-hidden="true">
                          <circle cx="26" cy="26" r="20" fill="none" stroke="hsl(var(--border))" strokeWidth="5" />
                          <circle cx="26" cy="26" r="20" fill="none" stroke={gaugeColor(s.conf)} strokeWidth="5" strokeLinecap="round" strokeDasharray={c} strokeDashoffset={c * (1 - s.conf)} />
                        </svg>
                        <span className="gv" style={{ color: gaugeColor(s.conf) }}>{gaugeLabel(s.conf)}</span>
                      </span>
                    </button>
                  );
                })}
              </div>
              <p className="sub" style={{ fontSize: 'calc(11px * var(--font-scale))', marginBottom: 12 }}>A return point is saved first — you can always undo.</p>
              <div className="we-pf-actions">
                {pickedSuspect === null ? null : (
                  <>
                    <button type="button" className="we-btn" onClick={() => setPickedSuspect(null)}>Pick another</button>
                    <button type="button" className="we-btn go" onClick={doctorTry}>Turn it off and try</button>
                  </>
                )}
                {!onTrialSuspect ? (
                  <button type="button" className="we-btn" onClick={() => { setDoctorOpen(false); onIntent({ kind: 'open-crash-doctor' }); }} style={{ marginLeft: 'auto', fontSize: 'calc(11px * var(--font-scale))', padding: '6px 10px' }}>Open full doctor</button>
                ) : null}
              </div>
            </>
          ) : null}
          {doctorTrial?.phase === 'working' ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '12px 0', color: 'hsl(var(--muted-foreground))', fontSize: 'calc(13px * var(--font-scale))' }}>
              <span className="we-step">
                <span className="mark" style={{ borderColor: 'var(--we-accent)', borderRightColor: 'transparent', animation: 'we-spin 0.7s linear infinite', width: 18, height: 18, display: 'inline-grid' }} />
              </span>
              {doctorTrial.message}
            </div>
          ) : null}
          {doctorTrial?.phase === 'done' ? (
            <div style={{ marginTop: 8 }}>
              <p style={{ fontSize: 'calc(13.5px * var(--font-scale))', fontWeight: 600, marginBottom: 12 }}>{doctorTrial.message}</p>
              <div className="we-pf-actions">
                <button type="button" className="we-btn" onClick={() => setDoctorTrial(null)}>Pick another</button>
                {doctorTrial.snapshotId && onUndoTrial ? (
                  <button type="button" className="we-btn" onClick={async () => { if (!doctorTrial.snapshotId) return; await onUndoTrial(doctorTrial.snapshotId); setDoctorTrial(null); setDoctorOpen(false); }}>Undo and put it back</button>
                ) : null}
                {onLaunch ? (
                  <button type="button" className="we-btn go" onClick={() => { setDoctorOpen(false); void onLaunch(); }}>Play to test</button>
                ) : null}
                <button type="button" className="we-btn" onClick={() => setDoctorOpen(false)}>Close</button>
              </div>
            </div>
          ) : null}
          {doctorTrial?.phase === 'error' ? (
            <div style={{ marginTop: 8 }}>
              <p style={{ fontSize: 'calc(13px * var(--font-scale))', color: 'hsl(var(--destructive))', marginBottom: 12 }}>{doctorTrial.message}</p>
              <div className="we-pf-actions">
                <button type="button" className="we-btn" onClick={() => setDoctorTrial(null)}>Try again</button>
                <button type="button" className="we-btn" onClick={() => setDoctorOpen(false)}>Close</button>
                {!onTrialSuspect ? (
                  <button type="button" className="we-btn go" onClick={() => { setDoctorOpen(false); onIntent({ kind: 'open-crash-doctor' }); }}>Open full doctor</button>
                ) : null}
              </div>
            </div>
          ) : null}
        </div>
      </div>

      {/* ── toasts + undo ──
          The achievement stack is empty in Simple mode (nothing is ever
          earned); the remove/undo toast below is not decoration and stays. */}
      <div className="we-toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className="we-ach" role="status">
            <span className="ico">{t.icon}</span>
            <span>
              <span className="t">{t.title}</span>
              <br />
              <span className="d">{t.detail}</span>
            </span>
          </div>
        ))}
      </div>
      <div className={`we-toast ${removed ? 'show' : ''}`} data-testid="we-remove-toast">
        <span>Removed {removed?.name}</span>
        <button type="button" onClick={undoRemove}>Undo</button>
      </div>
      <div className="we-ghost" aria-hidden="true" />
    </div>
  );
}
