/**
 * Guided walkthrough — React binding for the step machine.
 *
 * The provider owns three jobs and nothing else: it feeds browser events into
 * `tourReducer`, persists whatever comes back, and hands the current step to
 * the overlay. All decisions about *when* a step is done live in
 * `tourModel.ts`; all knowledge of *where* things are on screen lives in the
 * `data-tour` attributes.
 *
 * Click and change listeners are attached in the capture phase on `document`
 * so a step still completes when the app's own handler stops propagation, and
 * so watching costs nothing while no tour is running.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  TOUR_STEPS,
  tourReducer,
  type TourAnchor,
  type TourEvent,
  type TourState,
  type TourStep,
} from './tourModel';
import { anchorsFromNode } from './tourDom';
import { subscribeTourSignals } from './tourSignals';
import { loadTourRecord, saveTourRecord } from './tourStorage';

export interface TourContextValue {
  state: TourState;
  /** The step being shown, or null when no tour is running. */
  step: TourStep | null;
  stepNumber: number;
  totalSteps: number;
  running: boolean;
  /** True once the user has reached the end at least once. */
  completed: boolean;
  start: () => void;
  next: () => void;
  back: () => void;
  skip: () => void;
  end: () => void;
  /** Called by the overlay when a watched anchor turns up on screen. */
  reportPresent: (anchor: TourAnchor) => void;
}

const TourContext = createContext<TourContextValue | null>(null);

function initialState(): TourState {
  const record = loadTourRecord();
  // A tour interrupted by an app restart resumes where it stopped; anything
  // else (never run, ended, finished) starts idle.
  if (record.status === 'running') {
    return { status: 'running', index: Math.min(record.index, TOUR_STEPS.length - 1) };
  }
  return { status: 'idle', index: 0 };
}

interface TourProviderProps {
  children: ReactNode;
  /**
   * Called when a tour starts, so the app can put the user on the page the
   * first step talks about. The tour never navigates on its own after that —
   * every move is the user's.
   */
  onStart?: () => void;
  /** Overridable for tests. */
  steps?: readonly TourStep[];
}

export function TourProvider({ children, onStart, steps = TOUR_STEPS }: TourProviderProps) {
  const reduce = useCallback(
    (state: TourState, event: TourEvent) => tourReducer(state, event, steps),
    [steps],
  );
  const [state, dispatch] = useReducer(reduce, undefined, initialState);
  const [completed, setCompleted] = useState(() => loadTourRecord().completed);
  const running = state.status === 'running';

  // `onStart` fires from an effect rather than the click handler so a tour
  // resumed from storage lands on the right page too. Only from the first step
  // though: a tour interrupted halfway resumes where the user left it, and
  // yanking them back to Home would undo their own progress.
  const startedRef = useRef(false);
  useEffect(() => {
    if (!running) {
      startedRef.current = false;
      return;
    }
    if (startedRef.current) return;
    startedRef.current = true;
    if (state.index === 0) onStart?.();
  }, [running, state.index, onStart]);

  useEffect(() => {
    if (state.status === 'finished') setCompleted(true);
  }, [state.status]);

  useEffect(() => {
    saveTourRecord(state, completed || state.status === 'finished');
  }, [state, completed]);

  useEffect(() => {
    if (!running) return;
    const onClick = (event: MouseEvent) => {
      const anchors = anchorsFromNode(event.target);
      if (anchors.length > 0) dispatch({ type: 'dom-click', anchors });
    };
    const onChange = (event: Event) => {
      const anchors = anchorsFromNode(event.target);
      if (anchors.length > 0) dispatch({ type: 'dom-change', anchors });
    };
    document.addEventListener('click', onClick, true);
    document.addEventListener('change', onChange, true);
    return () => {
      document.removeEventListener('click', onClick, true);
      document.removeEventListener('change', onChange, true);
    };
  }, [running]);

  useEffect(() => {
    if (!running) return undefined;
    return subscribeTourSignals((signal) => dispatch({ type: 'signal', signal }));
  }, [running]);

  // Alt+arrows drive the card from the keyboard. Plain arrows and Escape are
  // left alone: while a modal dialog is open its focus trap owns them.
  useEffect(() => {
    if (!running) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.altKey) return;
      if (event.key === 'ArrowRight') {
        event.preventDefault();
        dispatch({ type: 'next' });
      } else if (event.key === 'ArrowLeft') {
        event.preventDefault();
        dispatch({ type: 'back' });
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [running]);

  const value = useMemo<TourContextValue>(() => ({
    state,
    step: running ? steps[state.index] ?? null : null,
    stepNumber: state.index + 1,
    totalSteps: steps.length,
    running,
    completed,
    start: () => dispatch({ type: 'start' }),
    next: () => dispatch({ type: 'next' }),
    back: () => dispatch({ type: 'back' }),
    skip: () => dispatch({ type: 'skip' }),
    end: () => dispatch({ type: 'end' }),
    reportPresent: (anchor: TourAnchor) => dispatch({ type: 'anchor-present', anchor }),
  }), [state, running, completed, steps]);

  return <TourContext.Provider value={value}>{children}</TourContext.Provider>;
}

/**
 * Tour controls. Returns null outside a provider so surfaces that may render
 * without one (tests, isolated stories) can degrade instead of crashing.
 */
export function useTour(): TourContextValue | null {
  return useContext(TourContext);
}
