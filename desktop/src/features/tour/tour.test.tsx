import { useState } from 'react';
import { beforeEach, describe, expect, it } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { TourProvider } from './TourProvider';
import { TourOverlay } from './TourOverlay';
import { TourStartButton } from './TourStartButton';
import { emitTourSignal } from './tourSignals';
import { loadTourRecord } from './tourStorage';
import type { TourStep } from './tourModel';

const steps: TourStep[] = [
  {
    id: 'explain',
    title: 'Here is the home page',
    body: 'Your dashboard.',
    anchors: ['page-home'],
    advance: { kind: 'next' },
  },
  {
    id: 'go-browse',
    title: 'Open Browse',
    body: 'Mods live here.',
    anchors: ['nav-browse'],
    advance: { kind: 'appear', anchor: 'page-browse' },
    waitingHint: 'Click Browse in the sidebar.',
  },
  {
    id: 'gated',
    title: 'About this page',
    body: 'Only makes sense on Browse.',
    gate: 'page-browse',
    advance: { kind: 'next' },
    offTrackHint: 'Go back to Browse to continue.',
  },
  {
    id: 'operation',
    title: 'Create it',
    body: 'Press Create.',
    advance: { kind: 'signal', signal: 'instance-created' },
    waitingHint: 'Waiting for the instance…',
  },
];

/** A miniature app: two pages and a sidebar button that navigates between them. */
function Harness() {
  const [page, setPage] = useState<'home' | 'browse'>('home');
  return (
    <div>
      <TourStartButton />
      {page === 'home'
        ? <div data-tour="page-home">Home page</div>
        : <div data-tour="page-browse">Browse page</div>}
      <button type="button" data-tour="nav-browse" onClick={() => setPage('browse')}>Browse</button>
    </div>
  );
}

function renderTour() {
  return render(
    <TourProvider steps={steps}>
      <Harness />
      <TourOverlay />
    </TourProvider>,
  );
}

describe('the guided tour', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('shows nothing until the user starts it', () => {
    renderTour();
    expect(screen.queryByRole('dialog', { name: 'Guided tour' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Start the walkthrough' })).toBeInTheDocument();
  });

  it('walks explain → action → signal, and only ever moves on the user', async () => {
    const user = userEvent.setup();
    renderTour();

    await user.click(screen.getByRole('button', { name: 'Start the walkthrough' }));
    expect(await screen.findByText('Here is the home page')).toBeInTheDocument();
    expectStep(1);

    // An explain-only step offers Continue.
    await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText('Open Browse')).toBeInTheDocument();

    // An action step does not: it waits for the app to reach the destination.
    expect(screen.queryByRole('button', { name: 'Continue' })).toBeNull();
    expect(screen.getByText('Click Browse in the sidebar.')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Browse' }));
    expect(await screen.findByText('About this page')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText('Create it')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Continue' })).toBeNull();

    act(() => emitTourSignal('instance-created'));
    await waitFor(() => {
      expect(screen.queryByRole('dialog', { name: 'Guided tour' })).toBeNull();
    });
    expect(loadTourRecord().completed).toBe(true);
  });

  it('refuses to continue while the step’s page is off screen, and recovers', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'Start the walkthrough' }));
    await user.click(screen.getByRole('button', { name: 'Continue' }));

    // Skipping the "open Browse" step lands on a step gated behind Browse
    // while the user is still on Home.
    await user.click(screen.getByRole('button', { name: 'Skip step' }));
    expect(await screen.findByText('About this page')).toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByText('Go back to Browse to continue.')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: 'Continue' })).toBeNull();

    // Getting back to the page the step is about restores it.
    await user.click(screen.getByRole('button', { name: 'Browse' }));
    expect(await screen.findByRole('button', { name: 'Continue' })).toBeInTheDocument();
    expect(screen.queryByText('Go back to Browse to continue.')).toBeNull();
  });

  it('pins a step reached by Back instead of auto-advancing, and Skip continues', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'Start the walkthrough' }));
    await user.click(screen.getByRole('button', { name: 'Continue' }));
    // Complete the "open Browse" step so the next step's page is on screen.
    await user.click(screen.getByRole('button', { name: 'Browse' }));
    expect(await screen.findByText('About this page')).toBeInTheDocument();

    // Going Back lands on the already-satisfied Browse step and it stays put —
    // the condition being technically met does not bounce the tour forward.
    await user.click(screen.getByRole('button', { name: 'Back' }));
    expect(await screen.findByText('Open Browse')).toBeInTheDocument();
    await waitFor(() => {
      expectStep(2);
    });
    expect(screen.getByText('Press Skip step to continue.')).toBeInTheDocument();

    // Skip step finishes it and lets the tour move on.
    await user.click(screen.getByRole('button', { name: 'Skip step' }));
    expect(await screen.findByText('About this page')).toBeInTheDocument();
    expectStep(3);
  });

  it('can be ended at any point and remembers that it is no longer running', async () => {
    const user = userEvent.setup();
    renderTour();
    await user.click(screen.getByRole('button', { name: 'Start the walkthrough' }));
    await user.click(screen.getByRole('button', { name: 'End tour' }));

    expect(screen.queryByRole('dialog', { name: 'Guided tour' })).toBeNull();
    expect(loadTourRecord().status).toBe('idle');
    expect(screen.getByRole('button', { name: 'Start the walkthrough' })).toBeInTheDocument();
  });

  it('resumes an interrupted tour on the next launch', async () => {
    const user = userEvent.setup();
    const first = renderTour();
    await user.click(screen.getByRole('button', { name: 'Start the walkthrough' }));
    await user.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText('Open Browse')).toBeInTheDocument();
    first.unmount();

    renderTour();
    expect(await screen.findByText('Open Browse')).toBeInTheDocument();
    expectStep(2);
  });

  it('moves on when the user types in the field a step is about', async () => {
    const user = userEvent.setup();
    const named: TourStep[] = [
      {
        id: 'name-it',
        title: 'Name it',
        body: 'Type a name here.',
        anchors: ['name-form'],
        advance: { kind: 'any', of: [{ kind: 'focus' }, { kind: 'next' }] },
      },
      {
        id: 'after',
        title: 'Then this',
        body: 'Next step.',
        advance: { kind: 'next' },
      },
    ];
    render(
      <TourProvider steps={named}>
        <div data-tour="name-form">
          <input aria-label="Name" />
          <button type="button">Other</button>
        </div>
        <TourStartButton />
        <TourOverlay />
      </TourProvider>,
    );
    await user.click(screen.getByRole('button', { name: 'Start the walkthrough' }));
    expect(await screen.findByText('Name it')).toBeInTheDocument();

    // Pressing a button in the same form does not count as working in the field.
    await user.click(screen.getByRole('button', { name: 'Other' }));
    expect(screen.getByText('Name it')).toBeInTheDocument();

    await user.type(screen.getByRole('textbox', { name: 'Name' }), 'Mine');
    expect(await screen.findByText('Then this')).toBeInTheDocument();
  });
});

/** Asserts the card's step counter. */
function expectStep(n: number) {
  expect(screen.getByText(`Step ${n} of 4`)).toBeInTheDocument();
}
