/**
 * Guided walkthrough — the spotlight and the instruction card.
 *
 * The overlay never blocks the app: the dim layer is `pointer-events: none`,
 * so the user drives the real UI while the tour follows along. It measures its
 * anchors on every frame because the things it points at move — dialogs
 * animate in, lists grow, pages scroll — and a highlight that lags behind the
 * button it is describing is worse than none.
 */

import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { acceptsContinue, watchedAnchors, type TourStep } from './tourModel';
import { findAnchor, isAnchorPresent, prefersReducedMotion } from './tourDom';
import { useTour, type TourContextValue } from './TourProvider';
import './tour.css';

interface Spot {
  anchor: string;
  top: number;
  left: number;
  width: number;
  height: number;
}

interface Layout {
  spots: Spot[];
  /** Which side of the screen the card sits on. */
  side: 'left' | 'right';
  cardTop: number;
  gateMet: boolean;
}

const CARD_MARGIN = 20;
const SPOT_PADDING = 8;
const DEFAULT_CARD_HEIGHT = 260;
/** How often to re-measure when animation frames are not being served. */
const BACKSTOP_INTERVAL_MS = 250;

function roundedSpot(anchor: string, rect: DOMRect): Spot {
  return {
    anchor,
    top: Math.round(rect.top - SPOT_PADDING),
    left: Math.round(rect.left - SPOT_PADDING),
    width: Math.round(rect.width + SPOT_PADDING * 2),
    height: Math.round(rect.height + SPOT_PADDING * 2),
  };
}

function sameSpots(a: Spot[], b: Spot[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((spot, index) => {
    const other = b[index];
    return spot.anchor === other.anchor
      && spot.top === other.top
      && spot.left === other.left
      && spot.width === other.width
      && spot.height === other.height;
  });
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

export function TourOverlay() {
  const tour = useTour();
  if (!tour || !tour.running || !tour.step) return null;
  if (typeof document === 'undefined') return null;
  return createPortal(<TourLayer tour={tour} step={tour.step} />, document.body);
}

function TourLayer({ tour, step }: { tour: TourContextValue; step: TourStep }) {
  const cardRef = useRef<HTMLDivElement>(null);
  const reportRef = useRef(tour.reportPresent);
  reportRef.current = tour.reportPresent;

  const [layout, setLayout] = useState<Layout>({
    spots: [],
    side: 'right',
    cardTop: CARD_MARGIN,
    gateMet: true,
  });

  // One measurement drives everything on screen: where the spotlight goes,
  // which side the card sits on, whether the step's surface is still open, and
  // whether the anchor this step is waiting for has turned up. It runs once
  // synchronously — so the very first paint of a step is already aligned — and
  // then on every frame, because the things it points at move: dialogs animate
  // in, lists grow, pages scroll.
  useLayoutEffect(() => {
    let frame = 0;
    const watched = watchedAnchors(step.advance);
    const anchors = step.anchors ?? [];

    const measure = () => {
      for (const anchor of watched) {
        if (isAnchorPresent(anchor)) reportRef.current(anchor);
      }

      const spots: Spot[] = [];
      for (const anchor of anchors) {
        const element = findAnchor(anchor);
        if (!element) continue;
        const rect = element.getBoundingClientRect();
        if (rect.width <= 0 && rect.height <= 0) continue;
        spots.push(roundedSpot(anchor, rect));
        if (!step.highlightAll) break;
      }

      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;
      const primary = spots[0];
      const cardHeight = cardRef.current?.offsetHeight ?? DEFAULT_CARD_HEIGHT;
      const side: Layout['side'] = primary && primary.left + primary.width / 2 > viewportWidth / 2
        ? 'left'
        : 'right';
      const cardTop = primary
        ? clamp(
          primary.top + primary.height / 2 - cardHeight / 2,
          CARD_MARGIN,
          Math.max(CARD_MARGIN, viewportHeight - cardHeight - CARD_MARGIN),
        )
        : clamp(
          viewportHeight - cardHeight - CARD_MARGIN * 2,
          CARD_MARGIN,
          Math.max(CARD_MARGIN, viewportHeight - cardHeight - CARD_MARGIN),
        );
      const gateMet = !step.gate || isAnchorPresent(step.gate);

      setLayout((previous) => (
        previous.gateMet === gateMet
          && previous.side === side
          && Math.abs(previous.cardTop - cardTop) < 1
          && sameSpots(previous.spots, spots)
          ? previous
          : { spots, side, cardTop: Math.round(cardTop), gateMet }
      ));
    };

    const loop = () => {
      measure();
      frame = requestAnimationFrame(loop);
    };
    measure();
    frame = requestAnimationFrame(loop);
    // Frames stop entirely while the window is hidden or fully occluded, and a
    // tour that cannot notice the user opened the page it asked for reads as
    // broken. This slow backstop keeps it progressing regardless.
    const backstop = window.setInterval(measure, BACKSTOP_INTERVAL_MS);
    return () => {
      cancelAnimationFrame(frame);
      window.clearInterval(backstop);
    };
  }, [step]);

  // Bring the target into view once per step, and again when the element the
  // step is waiting for finally mounts. Never while it is already in view, so
  // this cannot fight the user's own scrolling.
  const primaryAnchor = layout.spots[0]?.anchor;
  useEffect(() => {
    if (!primaryAnchor) return;
    const element = findAnchor(primaryAnchor);
    if (!element) return;
    const rect = element.getBoundingClientRect();
    if (rect.top >= 0 && rect.bottom <= window.innerHeight) return;
    element.scrollIntoView({
      block: 'center',
      behavior: prefersReducedMotion() ? 'auto' : 'smooth',
    });
  }, [step.id, primaryAnchor]);

  // Radix's dialogs treat any pointerdown outside their content as "dismiss",
  // and the tour card is by definition outside. Without this, pressing
  // Continue while the create-instance or install-review dialog is open would
  // close the very dialog the step is explaining. Stopping propagation at the
  // card keeps those events away from the document-level listeners while the
  // card's own buttons still work normally.
  useEffect(() => {
    const element = cardRef.current;
    if (!element) return;
    const stop = (event: Event) => event.stopPropagation();
    const types = ['pointerdown', 'mousedown', 'focusin'] as const;
    types.forEach((type) => element.addEventListener(type, stop));
    return () => types.forEach((type) => element.removeEventListener(type, stop));
  }, []);

  const offTrack = !layout.gateMet;
  const canContinue = acceptsContinue(step) && !offTrack;
  const hint = offTrack ? step.offTrackHint ?? 'Head back to that page to continue.' : step.waitingHint;
  const progress = Math.round((tour.stepNumber / tour.totalSteps) * 100);

  return (
    <div className="tour-root" data-tour-ui="overlay">
      <svg className="tour-scrim" aria-hidden="true">
        <defs>
          <mask id="tour-spotlight-mask">
            <rect x="0" y="0" width="100%" height="100%" fill="white" />
            {layout.spots.map((spot) => (
              <rect
                key={spot.anchor}
                x={spot.left}
                y={spot.top}
                width={spot.width}
                height={spot.height}
                rx="12"
                fill="black"
              />
            ))}
          </mask>
        </defs>
        <rect
          x="0"
          y="0"
          width="100%"
          height="100%"
          className="tour-scrim-fill"
          mask="url(#tour-spotlight-mask)"
        />
      </svg>

      {layout.spots.map((spot) => (
        <div
          key={spot.anchor}
          className="tour-ring"
          style={{ top: spot.top, left: spot.left, width: spot.width, height: spot.height }}
          aria-hidden="true"
        />
      ))}

      <div
        ref={cardRef}
        className={`tour-card tour-card-${layout.side}`}
        style={{ top: layout.cardTop }}
        role="dialog"
        aria-label="Guided tour"
        aria-live="polite"
      >
        <div className="flex items-center justify-between gap-3">
          <span className="text-[11px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
            Guided tour
          </span>
          <span className="text-[11px] font-medium text-muted-foreground">
            Step {tour.stepNumber} of {tour.totalSteps}
          </span>
        </div>
        <div
          className="mt-2 h-1 overflow-hidden rounded-full bg-muted"
          role="progressbar"
          aria-label="Tour progress"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={progress}
        >
          <div className="h-full rounded-full bg-primary transition-[width]" style={{ width: `${progress}%` }} />
        </div>

        <h3 className="mt-3 text-base font-bold text-foreground">{step.title}</h3>
        <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{step.body}</p>

        {hint && (
          <p
            className={`mt-3 rounded-lg px-3 py-2 text-xs font-medium ${
              offTrack
                ? 'bg-destructive/10 text-destructive'
                : 'bg-accent text-accent-foreground'
            }`}
            role="status"
          >
            {hint}
          </p>
        )}

        <div className="mt-4 flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={tour.end}
              className="rounded-lg px-2.5 py-1.5 text-xs font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            >
              End tour
            </button>
            {tour.stepNumber > 1 && (
              <button
                type="button"
                onClick={tour.back}
                title="Alt + Left arrow"
                className="rounded-lg px-2.5 py-1.5 text-xs font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              >
                Back
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            {!canContinue && (
              <button
                type="button"
                onClick={tour.skip}
                className="rounded-lg border border-border px-3 py-1.5 text-xs font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
              >
                Skip step
              </button>
            )}
            {canContinue && (
              <button
                type="button"
                onClick={tour.next}
                title="Alt + Right arrow"
                className="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-primary-foreground hover:bg-primary/90"
              >
                {step.continueLabel ?? 'Continue'}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
