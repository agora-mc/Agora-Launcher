/**
 * Agora Lab shell.
 *
 * Sol-0 contract: `docs/interactive/MASTER_ARCHITECTURE.md` §5.1. The Lab is a
 * new top-level destination beside the Field Guide. Its shell owns adventure
 * selection and resume, a persistent Simulation label, lesson stage, local
 * scenario state, local progress, pause/reset/exit, Field Guide handoffs, and
 * a non-spatial action list.
 *
 * The Lab never receives a live instance ID. A link to a real destination
 * ends the simulation first and navigates through callbacks provided by App;
 * it never carries a simulated plan into live state.
 *
 * Lab code must not import Tauri, `lib/tauri`, `live/`, or current operation
 * components — enforced by `scripts/check-interactive-boundaries.mjs`.
 */

import { useMemo, useRef, useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import type { VisualId, VisualScene } from '../domain/models';
import type { VisualIntent, StandardDestination } from '../domain/intents';
import { isGuideTopicId } from '../domain/intents';
import type { FeedbackEvent, LabDecision, LabEvent, LabLessonState } from './scenarioTypes';
import { getScenario, listScenarios } from './simulationAdapter';
import { initialLessonState, isLessonComplete, reduceLesson } from './lessonEngine';
import { resolveDecisionGate } from './decisionGate';
import { loadAdventureProgress, recordCheckpoint } from './progressStore';
import { ScenarioView } from './ScenarioView';
import { useReducedMotion } from '../visual/primitives/useReducedMotion';
import { Announcement } from '../visual/primitives/announce';

export interface LabShellProps {
  onOpenGuide: (topicId: string) => void;
  onNavigateStandard: (dest: StandardDestination) => void;
  /** Friendly labels for Field Guide topic ids (from GUIDE_TOPICS). */
  guideTopicLabels?: Record<string, string>;
}

const DESTINATION_LABEL: Record<string, string> = {
  'tab:home': 'Home',
  'tab:browse': 'Browse',
  'tab:instances': 'My Instances',
  'tab:governance': 'Community Governance',
  'tab:guide': 'Help & Guide',
  'tab:about': 'The Agora Difference',
  'tab:settings': 'Settings',
  'instance-detail': 'Open the instance',
  'mod-detail': 'Open the item',
};

function destinationLabel(dest: StandardDestination): string {
  if (dest.type === 'tab') return DESTINATION_LABEL[`tab:${dest.tab}`] ?? 'Open destination';
  return DESTINATION_LABEL[dest.type] ?? 'Open destination';
}

export function LabShell({
  onOpenGuide,
  onNavigateStandard,
  guideTopicLabels,
}: LabShellProps) {
  const reducedMotion = useReducedMotion();
  const scenarios = useMemo(() => listScenarios(), []);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [lesson, setLesson] = useState<LabLessonState<VisualScene> | null>(null);
  const [selection, setSelection] = useState<VisualId | null>(null);
  const [pendingConfirm, setPendingConfirm] = useState<LabDecision | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const invokeOriginRef = useRef<HTMLElement | null>(null);
  const [announcement, setAnnouncement] = useState<string | null>(null);
  /** Last decision that was blocked/rejected, for visible inline feedback. */
  const [lastAttempt, setLastAttempt] = useState<{ decisionId: string; feedback: FeedbackEvent } | null>(null);

  const scenario = activeId ? getScenario(activeId) ?? null : null;

  const startAdventure = (id: string, resumeCheckpoint = 0) => {
    const candidate = getScenario(id);
    if (!candidate) return;
    setActiveId(id);
    setLesson(initialLessonState(candidate, resumeCheckpoint));
    setSelection(null);
    setPendingConfirm(null);
    setConfirmOpen(false);
    setAnnouncement(null);
    setLastAttempt(null);
  };

  const leaveAdventure = () => {
    setActiveId(null);
    setLesson(null);
    setSelection(null);
    setPendingConfirm(null);
    setConfirmOpen(false);
    setAnnouncement(null);
    setLastAttempt(null);
  };

  const dispatch = (event: LabEvent) => {
    if (!scenario || !lesson) return;
    const reduction = reduceLesson(scenario, lesson, event);
    setLesson(reduction.state);
    if (reduction.feedback) {
      setAnnouncement(reduction.feedback.message);
      if (event.kind === 'decision') {
        const tone = reduction.feedback.tone;
        if (tone === 'blocked' || tone === 'caution') {
          setLastAttempt({ decisionId: event.decisionId, feedback: reduction.feedback });
        } else {
          setLastAttempt((current) => (current && current.decisionId === event.decisionId ? null : current));
        }
      }
    }
    const done = isLessonComplete(scenario, reduction.state);
    const completed = done ? scenario.checkpoints.length : reduction.state.checkpoint;
    recordCheckpoint(scenario.id, scenario.version, completed, completed, done);
  };

  /**
   * Single decision gate shared by the action list and visual-intent routes
   * (SOL-1 BLOCKER 2). Resolves the id against the current checkpoint,
   * rejects unavailable/disabled ids, opens confirmation for dangerous
   * decisions, and refuses new requests while confirmation is open (so the
   * background cannot be activated behind a modal).
   */
  const requestDecision = (decisionId: string) => {
    if (!scenario || !lesson) return;
    if (confirmOpen) return; // modal is exclusive; ignore background requests
    const gate = resolveDecisionGate(scenario, lesson, decisionId);
    if (gate.status === 'rejected') {
      setAnnouncement(gate.reason);
      return;
    }
    if (gate.status === 'confirm') {
      invokeOriginRef.current = (document.activeElement as HTMLElement) ?? null;
      setPendingConfirm(gate.decision);
      setConfirmOpen(true);
      return;
    }
    dispatch({ kind: 'decision', decisionId });
  };

  const handleDecision = (decision: LabDecision) => {
    requestDecision(decision.id);
  };

  const closeConfirmation = () => {
    setPendingConfirm(null);
    setConfirmOpen(false);
  };

  /**
   * Revalidates the confirmed decision against the CURRENT lesson state before
   * dispatch (SOL-1 BLOCKER 3). If the decision is no longer present or became
   * disabled, the old confirmation cannot dispatch.
   */
  const confirmPendingDecision = () => {
    if (!pendingConfirm || !scenario || !lesson) return;
    const checkpoint = scenario.checkpoints[lesson.checkpoint];
    const current = checkpoint?.decisionsFor(lesson).find((candidate) => candidate.id === pendingConfirm.id);
    if (!current || current.disabledReason) {
      closeConfirmation();
      setAnnouncement('That action is no longer available at this step. Nothing changed.');
      return;
    }
    dispatch({ kind: 'decision', decisionId: pendingConfirm.id });
    closeConfirmation();
  };

  const handleIntent = (intent: VisualIntent) => {
    if (!scenario || !lesson) return;
    if (scenario.intentToDecision) {
      const mapped = scenario.intentToDecision(lesson.scene, intent);
      if (mapped) {
        requestDecision(mapped.decisionId);
        return;
      }
    }
    if (intent.kind === 'open-guide') {
      onOpenGuide(intent.topicId);
      return;
    }
    if (intent.kind === 'open-standard') {
      onNavigateStandard(intent.destination);
    }
  };

  const handleReset = () => {
    dispatch({ kind: 'reset' });
    setSelection(null);
    setPendingConfirm(null);
    setConfirmOpen(false);
  };

  // ---------- Selection screen ----------
  if (!scenario || !lesson) {
    return (
      <div className="space-y-6" data-testid="lab-shell">
        <header>
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-2xl font-bold text-foreground">Agora Lab</h2>
            <span className="inline-flex items-center rounded-full border border-border bg-muted/60 px-2.5 py-0.5 text-xs font-bold text-muted-foreground">
              Simulation
            </span>
          </div>
          <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
            Short, safe, simulated adventures. Nothing here changes your real instances or settings.
          </p>
        </header>

        <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
          {scenarios.map((adventure) => {
            const record = loadAdventureProgress(adventure.id, adventure.version);
            const checkpointCount = adventure.checkpoints.length;
            const completed = record?.completed === true;
            const canResume = record && record.lastSafeCheckpoint > 0 && !completed;
            return (
              <div key={adventure.id} className="flex flex-col rounded-xl border border-border bg-card p-4" data-testid={`adventure-${adventure.id}`}>
                <div className="flex items-center gap-2">
                  <span aria-hidden="true" className="text-2xl">{adventure.iconLabel}</span>
                  <h3 className="text-lg font-bold text-foreground">{adventure.title}</h3>
                </div>
                <p className="mt-1 flex-1 text-sm text-muted-foreground">{adventure.description}</p>
                <p className="mt-2 text-xs font-semibold text-muted-foreground">
                  {completed
                    ? 'Complete'
                    : `${record?.completedCheckpoints ?? 0} of ${checkpointCount} steps`}
                </p>
                <div className="mt-3 flex gap-2">
                  <button
                    type="button"
                    onClick={() => startAdventure(adventure.id, completed || !canResume ? 0 : record.lastSafeCheckpoint)}
                    className="flex-1 rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
                  >
                    {canResume ? 'Resume' : 'Start'}
                  </button>
                  {canResume ? (
                    <button
                      type="button"
                      onClick={() => startAdventure(adventure.id)}
                      className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
                    >
                      Restart
                    </button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>

        <p className="text-xs text-muted-foreground">
          Field Guide stays available under Help & Guide. Lab progress is stored locally and never contains instance data.
        </p>
      </div>
    );
  }

  // ---------- Adventure screen ----------
  const checkpoint = scenario.checkpoints[lesson.checkpoint];
  const decisions = checkpoint?.decisionsFor(lesson) ?? [];
  const complete = lesson.status === 'complete';
  const feedback = lesson.lastFeedback;
  const feedbackToneClass =
    feedback?.tone === 'success'
      ? 'border-emerald-600/50 bg-emerald-600/10 text-emerald-800 dark:text-emerald-200'
      : feedback?.tone === 'blocked'
        ? 'border-destructive/60 bg-destructive/10 text-destructive'
        : feedback?.tone === 'caution'
          ? 'border-amber-500/50 bg-amber-500/10 text-amber-800 dark:text-amber-200'
          : 'border-border bg-muted/40 text-foreground';

  return (
    <div className="space-y-5" data-testid="lab-adventure">
      {/* Persistent Simulation label */}
      <div className="flex flex-wrap items-center gap-2 rounded-lg border border-dashed border-indigo-500/50 bg-indigo-500/5 px-3 py-2">
        <span className="inline-flex items-center rounded-full border border-indigo-500/60 bg-indigo-500/10 px-2.5 py-0.5 text-xs font-bold text-indigo-700 dark:text-indigo-300">
          Simulation
        </span>
        <span className="text-xs text-muted-foreground">
          Nothing here changes your real instances. This is practice only.
        </span>
        <Announcement message={announcement} />
      </div>

      <header className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h2 className="text-xl font-bold text-foreground">
            {scenario.iconLabel} {scenario.title}
          </h2>
          <p className="text-sm text-muted-foreground">
            Step {Math.min(lesson.checkpoint + 1, scenario.checkpoints.length)} of {scenario.checkpoints.length}
          </p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={handleReset}
            className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
          >
            Reset
          </button>
          <button
            type="button"
            onClick={leaveAdventure}
            className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
          >
            Exit
          </button>
        </div>
      </header>

      <ScenarioView
        scenario={scenario}
        state={lesson}
        selection={selection}
        onSelect={setSelection}
        onIntent={handleIntent}
        reducedMotion={reducedMotion}
      />

      {!complete && checkpoint ? (
        <section aria-label="Decision" className="rounded-xl border border-border bg-card p-4">
          <h3 className="text-sm font-bold text-foreground">{checkpoint.goal}</h3>
          <ul className="mt-3 flex flex-wrap gap-2">
            {decisions.map((decision) => {
              const attempted = lastAttempt?.decisionId === decision.id;
              if (decision.disabledReason) {
                return (
                  <li key={decision.id} className="flex items-center gap-2">
                    <span className="cursor-not-allowed rounded-md border border-border bg-muted/40 px-3 py-1.5 text-sm font-semibold text-muted-foreground opacity-70">
                      {decision.label}
                    </span>
                    <span className="text-xs text-muted-foreground">{decision.disabledReason}</span>
                  </li>
                );
              }
              return (
                <li key={decision.id} className="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => handleDecision(decision)}
                    aria-label={decision.keyboardLabel ?? decision.label}
                    className={`rounded-md px-3 py-1.5 text-sm font-semibold transition-colors ${
                      attempted
                        ? 'border border-destructive bg-destructive/10 text-destructive'
                        : decision.danger
                          ? 'border border-destructive/60 text-destructive hover:bg-destructive/10'
                          : 'bg-primary text-primary-foreground hover:opacity-90'
                    }`}
                  >
                    {decision.label}
                  </button>
                  {attempted && lastAttempt ? (
                    <span className="text-xs font-semibold text-destructive" role="status">
                      {lastAttempt.feedback.message}
                    </span>
                  ) : null}
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}

      <Dialog.Root open={confirmOpen} onOpenChange={(open) => { if (!open) closeConfirmation(); }}>
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
          <Dialog.Content
            role="alertdialog"
            aria-modal="true"
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              invokeOriginRef.current?.focus();
            }}
            className="fixed left-1/2 top-1/2 z-50 w-[min(92vw,26rem)] -translate-x-1/2 -translate-y-1/2 rounded-xl border-2 border-destructive/60 bg-card p-4 shadow-lg"
          >
            <Dialog.Title className="text-base font-bold text-destructive">
              {pendingConfirm?.confirmTitle ?? 'Serious confirmation required'}
            </Dialog.Title>
            <Dialog.Description className="mt-1 text-sm text-foreground">
              {pendingConfirm?.confirmBody ??
                'This simulated action is consequential. Review it carefully before confirming.'}
            </Dialog.Description>
            <div className="mt-3 flex gap-2">
              <button
                type="button"
                onClick={confirmPendingDecision}
                className="rounded-md bg-destructive px-3 py-1.5 text-sm font-semibold text-destructive-foreground hover:opacity-90"
              >
                Confirm
              </button>
              <button
                type="button"
                autoFocus
                onClick={closeConfirmation}
                className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
              >
                Cancel
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>

      {feedback ? (
        <div className={`rounded-lg border px-3 py-2 text-sm font-medium ${feedbackToneClass}`} role="status">
          {feedback.message}
        </div>
      ) : null}

      {complete ? (
        <section aria-label="Adventure complete" className="rounded-xl border border-emerald-600/40 bg-emerald-600/5 p-4">
          <h3 className="text-base font-bold text-foreground">Adventure complete</h3>
          <p className="mt-1 text-sm text-foreground">{scenario.completionMessage}</p>
          <p className="mt-1 text-xs text-muted-foreground">
            You completed the practice — this says nothing about your real instances.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={handleReset}
              className="rounded-md bg-primary px-3 py-1.5 text-sm font-semibold text-primary-foreground hover:opacity-90"
            >
              Replay
            </button>
            {scenario.guideTopics.map((topicId) =>
              isGuideTopicId(topicId) ? (
                <button
                  key={topicId}
                  type="button"
                  onClick={() => onOpenGuide(topicId)}
                  className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
                >
                  Field Guide: {guideTopicLabels?.[topicId] ?? topicId}
                </button>
              ) : null,
            )}
            {scenario.realDestinations.map((dest, index) => (
              <button
                key={`${dest.type}-${index}`}
                type="button"
                onClick={() => onNavigateStandard(dest)}
                className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
              >
                {destinationLabel(dest)}
              </button>
            ))}
            <button
              type="button"
              onClick={leaveAdventure}
              className="rounded-md border border-border px-3 py-1.5 text-sm font-semibold text-foreground hover:bg-accent"
            >
              Back to adventures
            </button>
          </div>
        </section>
      ) : null}

      <p className="text-xs text-muted-foreground">
        Want to do this for real? Open the Field Guide or the real feature above — the simulation ends first.
      </p>
    </div>
  );
}
