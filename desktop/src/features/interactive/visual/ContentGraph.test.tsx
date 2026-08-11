import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { ContentGraph } from './ContentGraph';
import { NO_CAPABILITIES } from '../domain/models';
import type { VisualScene } from '../domain/models';

function scene(): VisualScene {
  return {
    source: { kind: 'simulation', scenarioId: 'mod', scenarioVersion: 1 },
    content: [
      {
        id: 'lab:mod:better-caves',
        name: 'BetterCaves',
        kind: 'mod',
        presence: { current: 'not-installed' },
        enabled: { current: true },
        health: 'blocked',
        relationshipSummary: { requiredBy: 0, requires: 1, conflicts: 1 },
        availability: 'available',
      },
      {
        id: 'lab:mod:terrain-overhaul',
        name: 'Terrain Overhaul',
        kind: 'mod',
        presence: { current: 'installed' },
        enabled: { current: true },
        health: 'blocked',
        relationshipSummary: { requiredBy: 0, requires: 0, conflicts: 1 },
        availability: 'available',
      },
    ],
    relationships: [
      {
        id: 'lab:mod:rel:missing',
        fromId: 'lab:mod:better-caves',
        kind: 'requires',
        state: 'missing',
        importance: 'required',
        explanation: 'BetterCaves needs Core Lib to work.',
        affectedCount: 1,
      },
      {
        id: 'lab:mod:rel:conflict',
        fromId: 'lab:mod:better-caves',
        toId: 'lab:mod:terrain-overhaul',
        kind: 'conflicts-with',
        state: 'conflicting',
        importance: 'required',
        explanation: 'Both change world generation.',
      },
    ],
    findings: [],
    proposals: [],
  };
}

const capabilities = { ...NO_CAPABILITIES, canProposeInstall: true, canProposeRemove: true };

/** Richer scene with a named required target so socket chips have a real node. */
function socketScene(): VisualScene {
  return {
    source: { kind: 'simulation', scenarioId: 'mod', scenarioVersion: 1 },
    content: [
      {
        id: 'lab:mod:better-caves',
        name: 'BetterCaves',
        kind: 'mod',
        presence: { current: 'not-installed' },
        enabled: { current: true },
        health: 'blocked',
        relationshipSummary: { requiredBy: 0, requires: 1, conflicts: 1 },
        availability: 'available',
      },
      {
        id: 'lab:mod:core-lib',
        name: 'Core Lib',
        kind: 'mod',
        presence: { current: 'not-installed' },
        enabled: { current: true },
        health: 'healthy',
        relationshipSummary: { requiredBy: 1, requires: 0, conflicts: 0 },
        availability: 'available',
      },
      {
        id: 'lab:mod:terrain-overhaul',
        name: 'Terrain Overhaul',
        kind: 'mod',
        presence: { current: 'installed' },
        enabled: { current: true },
        health: 'blocked',
        relationshipSummary: { requiredBy: 0, requires: 0, conflicts: 1 },
        availability: 'available',
      },
    ],
    relationships: [
      {
        id: 'lab:mod:rel:requires',
        fromId: 'lab:mod:better-caves',
        toId: 'lab:mod:core-lib',
        kind: 'requires',
        state: 'missing',
        importance: 'required',
        explanation: 'BetterCaves needs Core Lib.',
      },
      {
        id: 'lab:mod:rel:conflict',
        fromId: 'lab:mod:better-caves',
        toId: 'lab:mod:terrain-overhaul',
        kind: 'conflicts-with',
        state: 'conflicting',
        importance: 'required',
        explanation: 'Both change world generation.',
      },
    ],
    findings: [],
    proposals: [],
  };
}

describe('ContentGraph', () => {
  it('renders a missing requirement as a broken socket with a label', () => {
    render(
      <ContentGraph scene={scene()} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={capabilities} />,
    );
    const region = screen.getByRole('region', { name: 'Missing requirements' });
    expect(within(region).getAllByText('Missing').length).toBeGreaterThanOrEqual(1);
    expect(within(region).getByText(/BetterCaves needs Core Lib/)).toBeInTheDocument();
  });

  it('renders conflicts in their own region', () => {
    render(
      <ContentGraph scene={scene()} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={capabilities} />,
    );
    const region = screen.getByRole('region', { name: 'Conflicts' });
    expect(region).toBeInTheDocument();
    expect(within(region).getByText('Conflicts with')).toBeInTheDocument();
    expect(within(region).getByText(/Both change world generation/)).toBeInTheDocument();
  });

  it('emits a propose-install intent from the Stage button (keyboard-accessible)', () => {
    const onIntent = vi.fn();
    render(
      <ContentGraph scene={scene()} selection={null} onSelect={() => undefined} onIntent={onIntent} capabilities={capabilities} />,
    );
    // Accessible names carry the target content (T6-9), e.g. "Stage install: Core Lib".
    const stageButton = screen.getAllByRole('button', { name: /^Stage install:/ })[0];
    fireEvent.click(stageButton);
    expect(onIntent).toHaveBeenCalledWith({ kind: 'propose-install', contentId: 'lab:mod:better-caves' });
  });

  it('offers a list view switch that does not lose selection', () => {
    const onSelect = vi.fn();
    render(
      <ContentGraph scene={scene()} selection={null} onSelect={onSelect} onIntent={() => undefined} capabilities={capabilities} />,
    );
    const listButton = screen.getByRole('button', { name: 'List view' });
    fireEvent.click(listButton);
    expect(listButton).toHaveAttribute('aria-pressed', 'true');
    const contentList = screen.getByRole('list', { name: 'Content' });
    fireEvent.click(within(contentList).getByText('BetterCaves'));
    expect(onSelect).toHaveBeenCalledWith('lab:mod:better-caves');
  });

  it('does not offer stage actions without capability flags', () => {
    render(
      <ContentGraph scene={scene()} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={NO_CAPABILITIES} />,
    );
    expect(screen.queryByRole('button', { name: /^Stage install:/ })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Stage removal:/ })).not.toBeInTheDocument();
  });

  it('diagram view shows socket chips with filled/empty states and conflict markers', () => {
    render(
      <ContentGraph scene={socketScene()} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={capabilities} />,
    );
    // BetterCaves card shows an empty/missing required socket with the named target
    expect(screen.getAllByText('Missing').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByRole('button', { name: 'Core Lib' }).length).toBeGreaterThanOrEqual(1);
    // active conflict chip is visible on the card with blocking marker
    expect(screen.getAllByText('Blocking').length).toBeGreaterThanOrEqual(1);
    // Core Lib card shows it is required by BetterCaves
    expect(screen.getByText(/Required by/)).toBeInTheDocument();
  });

  it('a resolved conflict renders as resolved, not blocking', () => {
    const resolved = socketScene();
    resolved.relationships = resolved.relationships.map((relationship) =>
      relationship.kind === 'conflicts-with' ? { ...relationship, state: 'satisfied' } : relationship,
    );
    resolved.content = resolved.content.map((node) => ({ ...node, health: 'healthy' }));
    render(
      <ContentGraph scene={resolved} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={capabilities} />,
    );
    expect(screen.queryByRole('region', { name: 'Conflicts' })).not.toBeInTheDocument();
    expect(screen.getByText('Resolved')).toBeInTheDocument();
    expect(screen.queryByText('Blocking')).not.toBeInTheDocument();
    // only the socket-chip target remains (the conflicts region is gone)
    expect(screen.getAllByRole('button', { name: 'Terrain Overhaul' })).toHaveLength(1);
  });

  it('list view exposes the full linear relationship list', () => {
    render(
      <ContentGraph scene={socketScene()} selection={null} onSelect={() => undefined} onIntent={() => undefined} capabilities={capabilities} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'List view' }));
    expect(screen.getByText('Relationships')).toBeInTheDocument();
    expect(screen.getAllByText(/Both change world generation/).length).toBeGreaterThanOrEqual(1);
  });
});
