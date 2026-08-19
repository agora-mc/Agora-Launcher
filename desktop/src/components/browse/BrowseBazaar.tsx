/**
 * BrowseBazaar — the High Interaction presentation for Browse, its own page:
 * the standard browse chrome is hidden while the Bazaar owns the view, with a
 * switch back at the top of the normal Browse page.
 *
 * Kept faithfully:
 *  - the drifting market background (gradient, receding stall rooftops with
 *    parallax, floating lanterns) and click particle bursts;
 *  - blocky stall icons and the gacha machine with its spin animation;
 *  - the legible taste model with ❤️ / 💔 (never 👍/👎 — see the note by the
 *    buttons), vibe bars that explain the
 *    reorder, a taste-weighted "Surprise me" machine that never returns
 *    something owned;
 *  - creature-art tiles (real icon_url preferred, deterministic critter
 *    fallback), POPULAR ribbons, the rotating rare glow, the bag counter;
 *  - the fit line that asks the same version-compatibility question the
 *    pre-flight does;
 *  - "Show me more like this" (dSimilar) and the toast.
 *
 * Adding an item does NOT open a side channel — it goes through the parent's
 * reviewed install flow (`onAdd`). "Put it in my bag" is staged locally for
 * the shelf ordering / gacha exclusion only, exactly as the plan requires.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getSetting } from '../../lib/tauri';
import {
  STALL_ICONS,
  STALLS,
  VIBES,
  VIBE_LABEL,
  categoryTags,
  crank,
  fitFor,
  gachaMachineArt,
  isOwned,
  loadBazaarState,
  matchesStall,
  paintCritter,
  saveBazaarState,
  scoreOf,
  sortedShelf,
  stageItem,
  topVibe,
  unstageItem,
  vibeBarWidths,
  vibesFor,
  vote,
  type BazaarItem,
  type BazaarState,
  type StallIcon,
  type Vibe,
  type VoteDirection,
} from './bazaar-model';
import './bazaar.css';

export interface BrowseBazaarProps {
  /** Shelf order already shown before navigating away, so returning does not
   *  reshuffle cards the user has already scrolled past. */
  initialSettledOrder?: string[];
  onSettledOrderChange?: (order: string[]) => void;
  items: BazaarItem[];
  /** The target instance's Minecraft version (for the fit line). */
  instanceVersion: string | null;
  /** Items already installed in the target instance. */
  ownedIds: Set<string>;
  /** Opens the reviewed install path (the Standard ModDetail flow). */
  onAdd: (item: BazaarItem) => void;
  /** Opens the full mod details page (Standard ModDetail) — the tile's
   * "View details" button routes here instead of the in-bazaar modal. */
  onOpenMod: (item: BazaarItem) => void;
  /** Returns to the standard Browse page. */
  onExit: () => void;
  /** The standard Browse load-more machinery (infinite scroll in the shelf). */
  hasMore?: boolean;
  loadMoreLoading?: boolean;
  onLoadMore?: () => void;
  /** Bulk-installs the staged bag through the normal browse install flow. */
  onInstallBag?: (staged: BazaarItem[]) => void;
  /**
   * Which stall is open, and the request to change it.
   *
   * Controlled by Browse because a stall is not a client-side filter: the page
   * only ever FETCHED one content type, so filtering the loaded list left every
   * stall except Add-ons permanently empty. Changing stalls has to change the
   * query, reusing the same fetch the Browse list uses.
   */
  stall?: string;
  onStallChange?: (stallId: string) => void;
}

/* Tiny feedback sound, respecting the global ambience sound setting. */
let soundOn = false;
export async function syncBazaarSound(): Promise<void> {
  try {
    soundOn = (await getSetting('ambience.sound')) === true;
  } catch {
    try {
      soundOn = window.localStorage.getItem('ambience.sound') === 'true';
    } catch {
      soundOn = false;
    }
  }
}
function blip(f: number, dur = 0.12, type: OscillatorType = 'triangle', vol = 0.06): void {
  if (!soundOn) return;
  try {
    const AC = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AC) return;
    const ctx = new AC();
    const o = ctx.createOscillator(), g = ctx.createGain();
    o.type = type; o.frequency.value = f;
    g.gain.setValueAtTime(0.0001, ctx.currentTime);
    g.gain.exponentialRampToValueAtTime(vol, ctx.currentTime + 0.012);
    g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + dur);
    o.connect(g); g.connect(ctx.destination);
    o.start(); o.stop(ctx.currentTime + dur + 0.03);
    void ctx.close().catch(() => undefined);
  } catch {
    // never let sound break the bazaar
  }
}
function fanfare(): void { [523, 659, 784, 1047].forEach((f, i) => setTimeout(() => blip(f, 0.18), i * 80)); }

/** A critter-art tile (fallback when the real icon_url is missing). */
function CritterArt({ name, size }: { name: string; size: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    c.width = size; c.height = size;
    const ctx = c.getContext('2d');
    if (ctx) paintCritter(ctx, name, size, size);
  }, [name, size]);
  return <canvas ref={ref} style={{ width: size, height: size }} aria-hidden="true" />;
}

/** A blocky icon painted onto a small canvas (stalls, gacha machine). */
function IconCanvas({ art, size = 26 }: { art: StallIcon; size?: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    c.width = size; c.height = size;
    const ctx = c.getContext('2d');
    if (ctx) art(ctx, size, size);
  }, [art, size]);
  return <canvas ref={ref} style={{ width: size, height: size }} aria-hidden="true" />;
}

/* Click particles are NOT here.
   The ambience layer draws them for the whole app on its own fx canvas, so a
   second document-level click listener painting a second full-screen canvas
   meant every click in the Bazaar burst twice and cost two rAF loops. */

/* ── item tile ── */
function ItemTile({
  item,
  state,
  fit,
  onVote,
  onOpenMod,
  onPeek,
  rare,
}: {
  item: BazaarItem;
  state: BazaarState;
  fit: boolean | null;
  onVote: (item: BazaarItem, d: VoteDirection) => void;
  onOpenMod: (item: BazaarItem) => void;
  /** Tile body → the quick look. Browsing should stay on the shelf; leaving for
   *  a full page on every glance is what makes a shelf tiring to browse. */
  onPeek: (item: BazaarItem) => void;
  rare: boolean;
}) {
  const owned = isOwned(state, item.id);
  const score = scoreOf(state, item);
  const voted = state.votes[item.id] ?? 0;
  return (
    <div className={`bazaar-tile ${rare ? 'rare' : ''}`}>
      <button type="button" className="bazaar-tile-art" onClick={() => onPeek(item)} aria-label={`Quick look at ${item.name}`}>
        {item.iconUrl ? (
          <img src={item.iconUrl} alt="" loading="lazy" className="bazaar-tile-img" />
        ) : (
          <CritterArt name={item.name} size={88} />
        )}
        {item.curated ? <span className="bazaar-ribbon">CURATED</span> : null}
        {owned ? <span className="bazaar-inbag" aria-label="Already in your bag">🎒</span> : null}
      </button>
      <button type="button" className="bazaar-tile-name" onClick={() => onPeek(item)}>{item.name}</button>
      <span className="bazaar-tile-by">{item.author ? `by ${item.author}` : ''}</span>
      <span className="bazaar-tags">
        {categoryTags(item).map((c) => <span key={c} className="bazaar-tag">{c}</span>)}
      </span>
      <span className={`bazaar-fit ${fit === false ? 'bad' : fit === true ? 'ok' : ''}`}>
        {fit === false
          ? '⚠️ Needs a different game version'
          : fit === true
            ? '✅ Fits your world'
            : 'Choose an instance to check the fit'}
      </span>
      {/* Hearts, NOT thumbs.
          👍/👎 on a tile is visually identical to Agora's real upvote/downvote,
          and those are governance: they feed curation and the vote-surge anomaly
          detection. Someone reshaping their own shelf must not look like they are
          casting a public verdict on a mod — least of all this audience, who are
          the least likely to notice the distinction. A heart is unambiguously
          personal, and the labels say plainly what the buttons actually do:
          reorder YOUR shelf. */}
      <div className="bazaar-vote">
        <button
          type="button"
          aria-label={`More like ${item.name} on my shelf`}
          aria-pressed={voted === 1}
          className={`bazaar-vote-btn yes ${voted === 1 ? 'on' : ''}`}
          onClick={(e) => { e.stopPropagation(); onVote(item, 1); }}
        >
          <span aria-hidden="true">❤️</span> More like this
        </button>
        <button
          type="button"
          aria-label={`Not for me: ${item.name}`}
          aria-pressed={voted === -1}
          className={`bazaar-vote-btn no ${voted === -1 ? 'on' : ''}`}
          onClick={(e) => { e.stopPropagation(); onVote(item, -1); }}
        >
          <span aria-hidden="true">💔</span> Not for me
        </button>
        {score >= 1 && !owned ? <span className="bazaar-score">+{Math.round(score * 10) / 10}</span> : null}
        <button
          type="button"
          className="bazaar-add"
          onClick={(e) => { e.stopPropagation(); onOpenMod(item); }}
        >
          View details
        </button>
      </div>
    </div>
  );
}

function VibeBars({ state }: { state: BazaarState }) {
  const widths = vibeBarWidths(state);
  const top = topVibe(state);
  return (
    <div className="bazaar-vibes">
      <h4 className="bazaar-vibe-h">What you seem to like</h4>
      <p className="bazaar-vibe-note">
        {top
          ? `Looks like you're after ${VIBE_LABEL[top].toLowerCase()} things. The shelf reorders to match.`
          : 'Tap ❤️ or 💔 on anything. The shelf quietly rearranges itself.'}
      </p>
      <div className="bazaar-vibe-list">
        {VIBES.map((v) => (
          <div key={v} className="bazaar-vibe">
            <span className="bazaar-vibe-label">{VIBE_LABEL[v]}</span>
            <div className="bazaar-vibe-track">
              <div
                className={`bazaar-vibe-fill ${state.taste[v] >= 0 ? 'pos' : 'neg'}`}
                style={{ width: `${widths[v]}%` }}
              />
            </div>
            <span className="bazaar-vibe-val">{state.taste[v] > 0 ? `+${state.taste[v]}` : state.taste[v]}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function GachaMachine({ state, pool, onPick }: { state: BazaarState; pool: BazaarItem[]; onPick: (item: BazaarItem) => void }) {
  const artRef = useRef<HTMLCanvasElement>(null);
  const [spinning, setSpinning] = useState(false);
  const [hint, setHint] = useState('Give it a crank');
  const rafRef = useRef(0);
  const reduced = useMemo(() => typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches, []);

  const draw = useCallback((fn: (c: CanvasRenderingContext2D, w: number, h: number) => void) => {
    const c = artRef.current;
    if (!c) return;
    c.width = 150; c.height = 110;
    const ctx = c.getContext('2d');
    if (ctx) fn(ctx, 150, 110);
  }, []);

  useEffect(() => {
    draw(gachaMachineArt);
    return () => cancelAnimationFrame(rafRef.current);
  }, [draw]);

  const spin = useCallback(() => {
    if (spinning) return;
    setSpinning(true);
    setHint('Cranking…');
    blip(300, 0.1, 'square', 0.05);
    const poolNames = pool.map((p) => p.name);
    const sample = () => poolNames[Math.floor(Math.random() * poolNames.length)] ?? 'Agora';
    const t0 = performance.now();
    let frames = 0;
    const step = (now: number) => {
      const el = now - t0;
      draw((c, w, h) => paintCritter(c, sample(), w, h));
      if (el < 1500 && frames < 30) {
        frames++;
        blip(420 + frames * 40, 0.05, 'square', 0.04);
        rafRef.current = requestAnimationFrame(step);
      } else {
        const pick = crank(state, pool);
        if (pick) {
          draw((c, w, h) => paintCritter(c, pick.name, w, h));
          setHint(`You got: ${pick.name}`);
          fanfare();
          onPick(pick);
        } else {
          setHint("You've got everything!");
        }
        setSpinning(false);
      }
    };
    rafRef.current = requestAnimationFrame(step);
  }, [spinning, pool, state, onPick, draw, reduced]);

  return (
    <div
      className="bazaar-gacha"
      tabIndex={0}
      role="button"
      aria-label="Surprise me"
      onClick={spin}
      onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); spin(); } }}
    >
      <canvas ref={artRef} className="bazaar-gacha-art" aria-hidden="true" />
      <b>Surprise me</b>
      <small className="bazaar-gacha-hint">{hint}</small>
      <button type="button" className="bazaar-gacha-btn" disabled={spinning} onClick={(e) => { e.stopPropagation(); spin(); }}>
        {spinning ? 'Cranking…' : '🎰 Crank'}
      </button>
    </div>
  );
}

export function BrowseBazaar({ items, instanceVersion, ownedIds, onAdd, onOpenMod, onExit, hasMore = false, loadMoreLoading = false, onLoadMore, onInstallBag, stall: stallProp, onStallChange, initialSettledOrder, onSettledOrderChange }: BrowseBazaarProps) {
  const [state, setState] = useState<BazaarState>(() => {
    const loaded = loadBazaarState();
    return { ...loaded, owned: Object.fromEntries(Array.from(ownedIds).map((id) => [id, true])) };
  });
  const [stallLocal, setStallLocal] = useState('all');
  const stall = stallProp ?? stallLocal;
  const setStall = (id: string) => { setStallLocal(id); onStallChange?.(id); };
  const [open, setOpen] = useState<BazaarItem | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<number | null>(null);

  useEffect(() => { void syncBazaarSound(); }, []);
  useEffect(() => { saveBazaarState(state); }, [state]);
  useEffect(() => {
    if (!toast) return;
    if (toastTimer.current !== null) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 2600);
    return () => { if (toastTimer.current !== null) window.clearTimeout(toastTimer.current); };
  }, [toast]);

  const filtered = useMemo(() => items.filter((it) => matchesStall(it, stall)), [items, stall]);
  // Order already shown to the user, frozen so taste changes and newly loaded
  // pages append below instead of reshuffling what has been scrolled past.
  const settledOrderRef = useRef<string[]>(initialSettledOrder ?? []);
  const shelf = useMemo(
    () => sortedShelf(state, filtered, settledOrderRef.current),
    [state, filtered],
  );
  useEffect(() => {
    settledOrderRef.current = shelf.map((item) => item.id);
    onSettledOrderChange?.(settledOrderRef.current);
  }, [shelf, onSettledOrderChange]);
  const bagCount = useMemo(() => Object.keys(state.owned).length + Object.keys(state.staged).length, [state]);
  const stagedItems = useMemo(
    () => items.filter((it) => state.staged[it.id]),
    [items, state.staged],
  );

  // Infinite scroll: the shelf reuses the standard Browse load-more so the
  // Bazaar keeps filling past the first page (browse_search returns 20/page).
  const sentinelRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (!sentinel || !hasMore || loadMoreLoading) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting && hasMore && !loadMoreLoading) onLoadMore?.();
      },
      { rootMargin: '600px' },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, loadMoreLoading, onLoadMore]);

  const handleVote = useCallback((item: BazaarItem, d: VoteDirection) => {
    setState((cur) => vote(cur, item, d));
    if (d > 0) blip(820, 0.1);
    else if (d < 0) blip(300, 0.1);
  }, []);

  const showToast = useCallback((msg: string) => setToast(msg), []);

  const handleAdd = useCallback((item: BazaarItem) => {
    setState((cur) => {
      if (isOwned(cur, item.id)) return cur;
      const next = stageItem(cur, item);
      // soft "you liked it" nudge, exactly like the prototype
      const taste = { ...next.taste };
      vibesFor(item).forEach((v) => {
        const key = v as Vibe;
        taste[key] = (taste[key] ?? 0) + 0.5;
      });
      return { ...next, taste };
    });
    showToast(`Added ${item.name} to your bag.`);
    fanfare();
    onAdd(item);
  }, [onAdd, showToast]);

  // The bag button IS the bulk-install button: it hands the staged items to
  // the normal Browse batch install flow (same reviewed pipeline as the
  // standard selection bar).
  const handleInstallBag = useCallback(() => {
    if (!onInstallBag) {
      showToast(stagedItems.length ? `In your bag: ${stagedItems.length} item${stagedItems.length === 1 ? '' : 's'}` : 'Your bag is empty. Go poke something.');
      return;
    }
    if (stagedItems.length === 0) {
      showToast('Your bag is empty. Open something and put it in first.');
      return;
    }
    onInstallBag(stagedItems);
  }, [onInstallBag, stagedItems, showToast]);

  const handleSimilar = useCallback(() => {
    if (!open) return;
    setState((cur) => {
      const taste = { ...cur.taste };
      vibesFor(open).forEach((v) => {
        const key = v as Vibe;
        taste[key] = (taste[key] ?? 0) + 1;
      });
      return { ...cur, taste };
    });
    showToast(`Shelf reordered around ${categoryTags(open).join(' + ')}.`);
    blip(760, 0.12);
    setOpen(null);
  }, [open, showToast]);

  // The shiny tint + CURATED tag belong to curated registry picks only —
  // everything else stays plain (no popularity glow on the whole shelf).
  const curatedIds = useMemo(() => {
    const ids = new Set<string>();
    items.forEach((it) => { if (it.curated) ids.add(it.id); });
    return ids;
  }, [items]);

  return (
    <div className="bazaar" data-testid="browse-bazaar">

      <div className="bazaar-scroll">
        <header className="bazaar-header agora-hero compact">
          <h1>The Bazaar</h1>
          <span className="bazaar-sub">Wander. Poke things. Take what you like.</span>
          <div className="bazaar-header-right">
            <button type="button" className="bazaar-back" onClick={onExit} data-testid="bazaar-exit">
              ← Back to Browse
            </button>
            <button
              type="button"
              className="bazaar-pack"
              title="Install everything you picked"
              onClick={handleInstallBag}
              data-testid="bazaar-install-bag"
            >
              <span style={{ fontSize: 17 }}>🎒</span>
              <b>{bagCount}</b>
              <span className="bazaar-sub">picked</span>
            </button>
          </div>
        </header>

        <div className="bazaar-stalls" role="tablist" aria-label="Market stalls">
          {STALLS.map((s) => (
            <button
              key={s.id}
              type="button"
              role="tab"
              aria-selected={stall === s.id}
              className={`bazaar-stall ${stall === s.id ? 'on' : ''}`}
              onClick={() => { setStall(s.id); blip(600, 0.09); }}
            >
              <IconCanvas art={STALL_ICONS[s.id]} />
              <span>{s.label}</span>
            </button>
          ))}
        </div>

        <div className="bazaar-row">
          <GachaMachine state={state} pool={items} onPick={setOpen} />
          <VibeBars state={state} />
        </div>

        <div className="bazaar-shelf" data-tour="browse-results">
          {shelf.map((item) => (
            <ItemTile
              key={item.id}
              item={item}
              state={state}
              fit={fitFor(item, instanceVersion)}
              onVote={handleVote}
              onOpenMod={onOpenMod}
              onPeek={setOpen}
              rare={curatedIds.has(item.id)}
            />
          ))}
          {shelf.length === 0 && (
            <p className="bazaar-empty">Nothing on this stall yet. Try another one.</p>
          )}
        </div>

        {hasMore && (
          <div
            ref={sentinelRef}
            className="bazaar-loadmore"
            data-testid="bazaar-load-sentinel"
          >
            {loadMoreLoading ? 'Wandering further into the bazaar…' : ''}
          </div>
        )}
      </div>

      {/* detail modal */}
      {open && (
        <div className="bazaar-detail-scrim" onClick={() => setOpen(null)}>
          <div className="bazaar-detail" role="dialog" aria-modal="true" aria-label={open.name} onClick={(e) => e.stopPropagation()}>
            <button type="button" className="bazaar-close" onClick={() => setOpen(null)} aria-label="Close">×</button>
            <div className="bazaar-detail-art">
              {open.iconUrl ? <img src={open.iconUrl} alt="" /> : <CritterArt name={open.name} size={120} />}
            </div>
            <h4 className="bazaar-detail-name">{open.name}</h4>
            <p className="bazaar-detail-by">
              {open.author ? `made by ${open.author}${open.curated ? ' · curated' : ''}` : open.curated ? 'curated pick' : ''}
            </p>
            {open.description ? <p className="bazaar-detail-desc">{open.description}</p> : null}
            {categoryTags(open).length > 0 ? (
              <span className="bazaar-tags" style={{ marginTop: 8 }}>
                {categoryTags(open).map((c) => <span key={c} className="bazaar-tag">{c}</span>)}
              </span>
            ) : null}
            <span className={`bazaar-fit-line ${fitFor(open, instanceVersion) === false ? 'bad' : 'ok'}`}>
              {fitFor(open, instanceVersion) === false
                ? '⚠️ This one won’t fit your world as it is — it needs a different game version.'
                : '✅ This fits your world. Nothing else needs changing.'}
            </span>
            <div className="bazaar-detail-actions">
              <button
                type="button"
                className="bazaar-add big"
                disabled={isOwned(state, open.id)}
                onClick={() => { handleAdd(open); setOpen(null); }}
              >
                {isOwned(state, open.id) ? 'In your bag' : 'Put it in my bag'}
              </button>
              {isOwned(state, open.id) && state.staged[open.id] ? (
                <button type="button" className="bazaar-add ghost" onClick={() => { setState((cur) => unstageItem(cur, open.id)); setOpen(null); }}>
                  Take it back out
                </button>
              ) : null}
              <button type="button" className="bazaar-add ghost" onClick={handleSimilar}>
                Show me more like this
              </button>
              {/* The quick look is a glance, not a dead end — the full page is
                  still one press away. */}
              <button
                type="button"
                className="bazaar-add ghost"
                onClick={() => { const it = open; setOpen(null); onOpenMod(it); }}
              >
                View details
              </button>
            </div>
          </div>
        </div>
      )}

      {/* toast */}
      <div className={`bazaar-toast ${toast ? 'show' : ''}`} role="status" data-testid="bazaar-toast">
        {toast}
      </div>

      <div aria-live="polite" style={{ position: 'absolute', width: 1, height: 1, overflow: 'hidden', clip: 'rect(0 0 0 0)', whiteSpace: 'nowrap' }}>
        {toast ?? ''}
      </div>
    </div>
  );
}
