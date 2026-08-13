/**
 * BrowseBazaar — the High Interaction presentation for Browse (v5-browse.html
 * port, V5-PORT-PLAN §11).
 *
 * Market stalls as the category switch, a legible five-axis taste model with
 * 👍/👎 on every tile, vibe bars that explain the reorder, a taste-weighted
 * "Surprise me" machine that never returns something owned, creature-art
 * tiles (real icon_url preferred, deterministic critter fallback), and a fit
 * line that asks the same version-compatibility question the pre-flight does.
 *
 * Adding an item does NOT open a side channel — it goes through the parent's
 * reviewed install flow (`onAdd`). "Put it in my bag" is staged locally for
 * the shelf ordering / gacha exclusion only, exactly as the plan requires.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getSetting } from '../../lib/tauri';
import {
  STALLS,
  VIBES,
  VIBE_LABEL,
  crank,
  fitFor,
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
  vote,
  type BazaarItem,
  type BazaarState,
  type VoteDirection,
} from './bazaar-model';
import './bazaar.css';

export interface BrowseBazaarProps {
  items: BazaarItem[];
  /** The target instance's Minecraft version (for the fit line). */
  instanceVersion: string | null;
  /** Items already installed in the target instance. */
  ownedIds: Set<string>;
  /** Opens the reviewed install path (the Standard ModDetail flow). */
  onAdd: (item: BazaarItem) => void;
}

/* Tiny feedback sound, respecting the global ambience sound setting. */
let soundOn = false;
export async function syncBazaarSound(): Promise<void> {
  try {
    soundOn = (await getSetting('ambience.sound')) === true;
  } catch {
    soundOn = false;
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
function fanfare(): void { [523, 659, 784, 1047].forEach((f, i) => setTimeout(() => blip(f, 0.16, 'triangle', 0.07), i * 80)); }

/** A critter-art tile (fallback when the real icon_url is missing). */
function CritterArt({ name, size, hue }: { name: string; size: number; hue?: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    c.width = size; c.height = size;
    const ctx = c.getContext('2d');
    if (ctx) paintCritter(ctx, name, size, size, hue);
  }, [name, size, hue]);
  return <canvas ref={ref} style={{ width: size, height: size }} aria-hidden="true" />;
}

function ItemTile({
  item,
  state,
  fit,
  onVote,
  onOpen,
  onAdd,
}: {
  item: BazaarItem;
  state: BazaarState;
  fit: boolean | null;
  onVote: (item: BazaarItem, d: VoteDirection) => void;
  onOpen: (item: BazaarItem) => void;
  onAdd: (item: BazaarItem) => void;
}) {
  const owned = isOwned(state, item.id);
  const score = scoreOf(state, item);
  const voted = state.votes[item.id] ?? 0;
  return (
    <div className="bazaar-tile">
      <button type="button" className="bazaar-tile-art" onClick={() => onOpen(item)} aria-label={`Open ${item.name}`}>
        {item.iconUrl ? (
          <img src={item.iconUrl} alt="" loading="lazy" className="bazaar-tile-img" />
        ) : (
          <CritterArt name={item.name} size={88} />
        )}
        {item.popular ? <span className="bazaar-ribbon">Popular</span> : null}
        {owned ? <span className="bazaar-inbag" aria-label="Already in your world">✓</span> : null}
      </button>
      <button type="button" className="bazaar-tile-name" onClick={() => onOpen(item)}>{item.name}</button>
      <span className="bazaar-tile-by">{item.author ? `by ${item.author}` : ''}</span>
      <span className={`bazaar-fit ${fit === false ? 'bad' : fit === true ? 'ok' : ''}`}>
        {fit === false
          ? '⚠️ Needs a different game version'
          : fit === true
            ? '✅ Fits your world'
            : 'Choose an instance to check the fit'}
      </span>
      <div className="bazaar-vote">
        <button
          type="button"
          aria-label={`I like ${item.name}`}
          aria-pressed={voted === 1}
          className={`bazaar-vote-btn yes ${voted === 1 ? 'on' : ''}`}
          onClick={(e) => { e.stopPropagation(); onVote(item, 1); }}
        >
          👍
        </button>
        <button
          type="button"
          aria-label={`I don't like ${item.name}`}
          aria-pressed={voted === -1}
          className={`bazaar-vote-btn no ${voted === -1 ? 'on' : ''}`}
          onClick={(e) => { e.stopPropagation(); onVote(item, -1); }}
        >
          👎
        </button>
        {score >= 1 && !owned ? <span className="bazaar-score">+{Math.round(score * 10) / 10}</span> : null}
        <button
          type="button"
          className="bazaar-add"
          onClick={(e) => { e.stopPropagation(); onAdd(item); }}
          disabled={owned}
        >
          {owned ? 'In your world' : 'Put in my bag'}
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
      <p className="bazaar-vibe-note">
        {top
          ? `Looks like you're after ${VIBE_LABEL[top].toLowerCase()} things. The shelf reorders to match.`
          : 'Vote 👍 or 👎 on anything and the shelf reorders to match your taste.'}
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

function SurpriseMe({ state, pool, onPick }: { state: BazaarState; pool: BazaarItem[]; onPick: (item: BazaarItem) => void }) {
  const artRef = useRef<HTMLCanvasElement>(null);
  const [spinning, setSpinning] = useState(false);
  const [hint, setHint] = useState('Give it a crank');
  const timerRef = useRef<number | null>(null);
  const rafRef = useRef<number>(0);

  const stopAnim = () => {
    if (timerRef.current !== null) { clearInterval(timerRef.current); timerRef.current = null; }
    if (rafRef.current) { cancelAnimationFrame(rafRef.current); rafRef.current = 0; }
  };

  useEffect(() => () => stopAnim(), []);

  const drawCritter = (name: string) => {
    const c = artRef.current;
    if (!c) return;
    c.width = 88; c.height = 88;
    const ctx = c.getContext('2d');
    if (ctx) paintCritter(ctx, name, 88, 88);
  };

  const spin = useCallback(() => {
    if (spinning) return;
    setSpinning(true);
    setHint('Cranking…');
    blip(300, 0.1, 'square', 0.05);
    const poolNames = pool.map((p) => p.name);
    const sample = () => poolNames[Math.floor(Math.random() * poolNames.length)] ?? 'Agora';
    let frames = 0;
    const t0 = performance.now();
    const step = (now: number) => {
      const el = now - t0;
      drawCritter(sample());
      if (el < 1500 && frames < 30) {
        frames++;
        blip(420 + frames * 40, 0.05, 'square', 0.04);
        rafRef.current = requestAnimationFrame(step);
      } else {
        const pick = crank(state, pool);
        if (pick) {
          drawCritter(pick.name);
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
    void stopAnim;
  }, [spinning, pool, state, onPick]);

  return (
    <div className="bazaar-gacha">
      <canvas ref={artRef} className="bazaar-gacha-art" aria-hidden="true" />
      <div className="bazaar-gacha-body">
        <strong>Surprise me</strong>
        <span className="bazaar-gacha-hint">{hint}</span>
        <button type="button" onClick={spin} disabled={spinning} className="bazaar-gacha-btn">
          {spinning ? 'Cranking…' : '🎰 Crank'}
        </button>
      </div>
    </div>
  );
}

export function BrowseBazaar({ items, instanceVersion, ownedIds, onAdd }: BrowseBazaarProps) {
  const [state, setState] = useState<BazaarState>(() => {
    const loaded = loadBazaarState();
    return { ...loaded, owned: Object.fromEntries(Array.from(ownedIds).map((id) => [id, true])) };
  });
  const [stall, setStall] = useState('all');
  const [open, setOpen] = useState<BazaarItem | null>(null);

  useEffect(() => { void syncBazaarSound(); }, []);
  useEffect(() => {
    saveBazaarState(state);
  }, [state]);

  const filtered = useMemo(() => items.filter((it) => matchesStall(it, stall)), [items, stall]);
  const shelf = useMemo(() => sortedShelf(state, filtered), [state, filtered]);

  const handleVote = useCallback((item: BazaarItem, d: VoteDirection) => {
    setState((cur) => vote(cur, item, d));
    if (d > 0) blip(820, 0.1);
    else if (d < 0) blip(300, 0.1);
  }, []);

  const handleAdd = useCallback((item: BazaarItem) => {
    setState((cur) => (isOwned(cur, item.id) ? cur : stageItem(cur, item)));
    onAdd(item);
  }, [onAdd]);

  const handleUnstage = useCallback((id: string) => {
    setState((cur) => unstageItem(cur, id));
  }, []);

  const picked = (item: BazaarItem) => setOpen(item);

  return (
    <div className="bazaar" data-testid="browse-bazaar">
      <div className="bazaar-head">
        <h3 className="bazaar-title">The Bazaar</h3>
        <p className="bazaar-sub">Wander the stalls. The shelf learns what you like — and tells you why.</p>
      </div>

      <div className="bazaar-stalls" role="tablist" aria-label="Market stalls">
        {STALLS.map((s) => (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={stall === s.id}
            className={`bazaar-stall ${stall === s.id ? 'on' : ''}`}
            onClick={() => { setStall(s.id); blip(600, 0.07); }}
          >
            <span className="bazaar-stall-emoji">{s.emoji}</span>
            <span>{s.label}</span>
          </button>
        ))}
      </div>

      <div className="bazaar-row">
        <VibeBars state={state} />
        <SurpriseMe state={state} pool={items} onPick={picked} />
      </div>

      <div className="bazaar-shelf">
        {shelf.map((item) => (
          <ItemTile
            key={item.id}
            item={item}
            state={state}
            fit={fitFor(item, instanceVersion)}
            onVote={handleVote}
            onOpen={setOpen}
            onAdd={handleAdd}
          />
        ))}
        {shelf.length === 0 && (
          <p className="bazaar-empty">Nothing in this stall yet. Try another one.</p>
        )}
      </div>

      {open && (
        <div className="bazaar-detail-scrim" onClick={() => setOpen(null)}>
          <div className="bazaar-detail" role="dialog" aria-modal="true" aria-label={open.name} onClick={(e) => e.stopPropagation()}>
            <button type="button" className="bazaar-close" onClick={() => setOpen(null)} aria-label="Close">×</button>
            <div className="bazaar-detail-art">
              {open.iconUrl ? <img src={open.iconUrl} alt="" /> : <CritterArt name={open.name} size={120} />}
            </div>
            <h4 className="bazaar-detail-name">{open.name}</h4>
            <p className="bazaar-detail-by">{open.author ? `made by ${open.author}` : ''}</p>
            {open.description ? <p className="bazaar-detail-desc">{open.description}</p> : null}
            <span className={`bazaar-fit ${fitFor(open, instanceVersion) === false ? 'bad' : 'ok'}`}>
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
                {isOwned(state, open.id) ? 'In your world' : 'Put it in my bag'}
              </button>
              {isOwned(state, open.id) && state.staged[open.id] ? (
                <button type="button" className="bazaar-add ghost" onClick={() => handleUnstage(open.id)}>
                  Take it back out
                </button>
              ) : null}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
