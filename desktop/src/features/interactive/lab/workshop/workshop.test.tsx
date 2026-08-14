/**
 * Workshop tests — the v5-lab port (V5-PORT-PLAN §12).
 *
 * These drive the real interactive DOM the benches build, exactly as a player
 * would: open a station, run the Try-it step, and verify the lesson's outcome
 * appears in the say line. The fictional-loaders rule is asserted too: Loader
 * A/B/C and "No loader" appear in the build bench, and no real product name
 * shows on the workbench surface.
 */

import { beforeEach, describe, expect, it } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Workshop } from './Workshop';

beforeEach(() => {
  try { window.localStorage.removeItem('agora-lab-v5b'); } catch { /* ignore */ }
});

function findPiece(name: string): HTMLElement {
  const els = Array.from(document.querySelectorAll('.ws-piece'));
  const el = els.find((e) => e.textContent?.includes(name));
  if (!el) throw new Error('piece not found: ' + name);
  return el as HTMLElement;
}

function sayText(): string {
  return document.querySelector('.ws-bench-body .ws-say')?.textContent ?? '';
}

/** Drop a piece into the first slot (the keyboard-equivalent drop). */
function dropIntoSlot(piece: HTMLElement): void {
  // `draggable` attaches keydown Enter to the piece; fire it.
  fireEvent.keyDown(piece, { key: 'Enter' });
}

async function waitForSay(match: RegExp): Promise<void> {
  await waitFor(() => expect(sayText()).toMatch(match));
}

describe('Workshop', () => {
  it('renders six stations with progress dots', () => {
    render(<Workshop />);
    expect(screen.getByText('The Workshop')).toBeInTheDocument();
    ['Build it', 'Add stuff', 'Something broke', 'Health check', 'Going offline', 'Undo it'].forEach((title) => {
      expect(screen.getByText(title)).toBeInTheDocument();
    });
    expect(screen.getAllByTestId('ws-map').length).toBe(1);
  });

  it('build bench: a wrong loader is rejected and the right loader snaps in', async () => {
    render(<Workshop />);
    fireEvent.click(screen.getByText('Build it'));
    await waitFor(() => expect(document.querySelector('.ws-bench.show')).not.toBeNull());
    // The workbench is now showing; drag the game version in first.
    dropIntoSlot(findPiece('Game 1.20.1'));
    await waitForSay(/loader/);
    // Wrong loader (Loader B is built for 1.21) is rejected.
    dropIntoSlot(findPiece('Loader B'));
    await waitForSay(/doesn't fit/);
    // Right loader snaps in.
    dropIntoSlot(findPiece('Loader A'));
    await waitForSay(/can take add-ons/);
  });

  it('build bench uses fictional loader names, never real products', async () => {
    render(<Workshop />);
    fireEvent.click(screen.getByText('Build it'));
    // Loader A/B and "No loader" are the workbench pieces.
    await waitFor(() => expect(findPiece('Loader A')).toBeTruthy());
    expect(findPiece('Loader B').textContent).toContain('Loader B');
    expect(findPiece('No loader').textContent).toContain('No loader');
    // No real product names on the workbench surface (the Why step links to
    // the Field Guide, which is where real names belong).
    expect(screen.queryByText(/Fabric|Forge|NeoForge|Quilt/)).toBeNull();
  });

  it('completing every step of a bench earns the badge', async () => {
    render(<Workshop />);
    // Open Build it and complete all four steps by driving each step's UI.
    fireEvent.click(screen.getByText('Build it'));
    await waitFor(() => expect(document.querySelector('.ws-bench.show')).not.toBeNull());
    await waitFor(() => expect(findPiece('Game 1.20.1')).toBeTruthy());

    // Every step ends on an explicit Next press — nothing advances on its own.
    const next = async () => {
      await waitFor(() => expect(document.querySelector('.ws-next .ws-btn')).not.toBeNull());
      fireEvent.click(document.querySelector('.ws-next .ws-btn') as HTMLElement);
    };

    // Step 1: place version + Loader A.
    dropIntoSlot(findPiece('Game 1.20.1'));
    await waitForSay(/loader/);
    dropIntoSlot(findPiece('Loader A'));
    await waitForSay(/can take add-ons/);
    await next();
    // Step 2: predict — pick the right answer; the reveal stays until Next.
    await waitFor(() => expect(screen.getByText(/It refuses to go in/)).toBeTruthy());
    fireEvent.click(screen.getByText(/It refuses to go in/));
    await next();
    // Step 3: transfer — build for Notebot (Game 1.21 + Loader B).
    await waitFor(() => expect(findPiece('Game 1.21')).toBeTruthy());
    dropIntoSlot(findPiece('Game 1.21'));
    dropIntoSlot(findPiece('Loader B'));
    await next();
    // Step 4: explain — "Got it — back to the workshop" marks it done and
    // closes; the badge appears.
    await waitFor(() => expect(screen.getByText('Got it — back to the workshop')).toBeTruthy());
    fireEvent.click(screen.getByText('Got it — back to the workshop'));
    await waitFor(() => expect(screen.queryByTestId('ws-badge')).toBeInTheDocument());
    // The stamp lights up.
    await waitFor(() => expect(document.querySelector('.ws-stamp.on')).not.toBeNull());
  });

  it('never advances a step on its own — Next is always an explicit press', async () => {
    render(<Workshop />);
    fireEvent.click(screen.getByText('Build it'));

    // Completing step 1 must NOT jump to step 2. The snap message is the
    // teaching moment; it stays until the learner moves on.
    dropIntoSlot(findPiece('Game 1.20.1'));
    await waitForSay(/loader/);
    dropIntoSlot(findPiece('Loader A'));
    await waitForSay(/can take add-ons/);
    await waitFor(() => expect(document.querySelector('.ws-next .ws-btn')).not.toBeNull());
    expect(screen.queryByText(/It works fine/)).toBeNull();       // step 2 not shown yet
    expect((document.querySelector('.ws-next .ws-btn') as HTMLElement).textContent)
      .toMatch(/^Next: /);

    fireEvent.click(document.querySelector('.ws-next .ws-btn') as HTMLElement);
    await waitFor(() => expect(screen.getByText(/It works fine/)).toBeTruthy());

    // Each option carries its own letter badge (A/B/C…).
    const keys = Array.from(document.querySelectorAll('.ws-opt-key')).map((k) => k.textContent);
    expect(keys).toEqual(['A', 'B', 'C']);

    // Answering reveals the explanation and leaves it on screen; the options
    // stay visible so the reveal can be read against them.
    fireEvent.click(screen.getByText(/It works fine/));
    await waitFor(() => expect(document.querySelector('.ws-next .ws-btn')).not.toBeNull());
    expect(document.querySelectorAll('.ws-opt').length).toBe(3);

    fireEvent.click(document.querySelector('.ws-next .ws-btn') as HTMLElement);
    await waitFor(() => expect(document.querySelectorAll('.ws-opt').length).toBe(0));
  });

  it('progress persists per step across a reopen', async () => {
    const { getByText } = render(<Workshop />);
    fireEvent.click(getByText('Add stuff'));
    await waitFor(() => expect(document.querySelector('.ws-bench.show')).not.toBeNull());
    // Step 1 Try it: drag Better Caves in → Core Helper arrives → win.
    dropIntoSlot(findPiece('Better Caves'));
    await waitForSay(/brought Core Helper along/);
    // Close; reopen → the first step is marked done.
    fireEvent.click(screen.getByText('Close'));
    fireEvent.click(screen.getByText('Add stuff'));
    // The rail shows the first pip as done.
    await waitFor(() => {
      const pips = Array.from(document.querySelectorAll('.ws-pip'));
      expect(pips[0].classList.contains('ok')).toBe(true);
    });
  });
});
