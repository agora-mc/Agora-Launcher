/**
 * WorldEditor — dependency curves.
 *
 * The "bounded neighbourhood diagram on selection (real curves, capped node
 * count)" the plan calls for. It shipped with no SVG overlay at all, so there
 * was nothing to draw and nothing to notice: the tiles still highlighted, which
 * made the absence look like a styling choice rather than a missing feature.
 *
 * jsdom performs no layout, so geometry is meaningless here — these tests pin
 * the things that were actually wrong: that curves exist at all, that they
 * encode direction, and that the count stays bounded.
 */

import { render, fireEvent, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { WorldEditor } from './WorldEditor';
import type { LiveHostData } from './LiveSceneView';
import type { VisualScene } from '../domain/models';

function node(id: string, name: string, iconUrl?: string) {
  return {
    id, name, kind: 'mod' as const,
    ...(iconUrl ? { iconUrl } : {}),
    presence: { current: 'installed' as const },
    enabled: { current: true },
    health: 'healthy' as const,
    relationshipSummary: { requiredBy: 0, requires: 0, conflicts: 0 },
    availability: 'available' as const,
  };
}

/** `dependents` all require `target`. */
function sceneWith(target: string, dependents: string[]): VisualScene {
  const content = [node(target, target), ...dependents.map((d) => node(d, d))];
  const relationships = dependents.map((d) => ({
    id: `rel:${d}`,
    fromId: d,
    toId: target,
    kind: 'requires' as const,
    state: 'satisfied' as const,
    importance: 'required' as const,
    explanation: `${d} requires ${target}`,
  }));
  return {
    source: { kind: 'live', revision: 1, freshness: 'fresh' },
    instance: {
      id: 'i1', name: 'Test', gameVersion: '1.20.1',
      loader: { current: { family: 'Fabric', compatibility: 'unknown' } },
      lockState: 'editable', recoveryReadiness: 'ready', launchState: 'idle',
      contentSummary: { enabled: content.length, disabled: 0, needsAttention: 0 },
    },
    content,
    relationships,
    findings: [],
    proposals: [],
  } as unknown as VisualScene;
}

function hostData(scene: VisualScene): LiveHostData {
  return {
    scene,
    health: { status: 'ok', value: true },
    snapshots: { status: 'ok', value: [] },
    crashEvidence: { status: 'ok', value: null },
    runtime: { status: 'ok', value: null },
  } as unknown as LiveHostData;
}

function renderEditor(scene: VisualScene) {
  return render(
    <WorldEditor
      data={hostData(scene)}
      capabilities={{} as never}
      selection={null}
      onSelect={vi.fn()}
      onIntent={vi.fn()}
      onUseStandardView={vi.fn()}
    />,
  );
}

const slotNamed = (name: string) =>
  document.querySelector(`.we-slot[data-name="${name}"]`) as HTMLElement;

describe('WorldEditor dependency curves', () => {
  it('draws nothing until something is selected', () => {
    renderEditor(sceneWith('lib.jar', ['a.jar', 'b.jar']));
    expect(document.querySelectorAll('.we-link').length).toBe(0);
  });

  it('draws one curve per neighbour when an item is selected', async () => {
    renderEditor(sceneWith('lib.jar', ['a.jar', 'b.jar', 'c.jar']));
    fireEvent.click(slotNamed('lib.jar'));
    await waitFor(() => expect(document.querySelectorAll('.we-link').length).toBe(3));
  });

  it('encodes direction: inbound needs are solid, outbound dashed', async () => {
    renderEditor(sceneWith('lib.jar', ['a.jar']));
    // Selecting the library: something else needs IT → inbound.
    fireEvent.click(slotNamed('lib.jar'));
    await waitFor(() => expect(document.querySelector('.we-link.needs')).not.toBeNull());
    expect(document.querySelectorAll('.we-link.needed').length).toBe(0);

    // Selecting the dependent: IT needs something → outbound.
    fireEvent.click(slotNamed('lib.jar'));           // deselect
    fireEvent.click(slotNamed('a.jar'));
    await waitFor(() => expect(document.querySelector('.we-link.needed')).not.toBeNull());
  });

  it('draws every connection for a hub, up to a generous bound', async () => {
    // A library like Fabric API is needed by most of a pack. Capping tightly hid
    // exactly what made it a hub, so the bound only exists to stop the drawing
    // becoming an unreadable starburst — 20 dependents must all be drawn.
    const many = Array.from({ length: 20 }, (_, i) => `dep${i}.jar`);
    renderEditor(sceneWith('lib.jar', many));
    fireEvent.click(slotNamed('lib.jar'));
    await waitFor(() => expect(document.querySelectorAll('.we-link').length).toBe(20));
  });

  it('still bounds a pathological hub', async () => {
    const huge = Array.from({ length: 90 }, (_, i) => `dep${i}.jar`);
    renderEditor(sceneWith('lib.jar', huge));
    fireEvent.click(slotNamed('lib.jar'));
    await waitFor(() => expect(document.querySelectorAll('.we-link').length).toBeGreaterThan(0));
    expect(document.querySelectorAll('.we-link').length).toBeLessThanOrEqual(40);
  });

  it('clears the curves when the selection is dropped', async () => {
    renderEditor(sceneWith('lib.jar', ['a.jar']));
    fireEvent.click(slotNamed('lib.jar'));
    await waitFor(() => expect(document.querySelectorAll('.we-link').length).toBe(1));
    fireEvent.click(slotNamed('lib.jar'));
    await waitFor(() => expect(document.querySelectorAll('.we-link').length).toBe(0));
  });

  it('the overlay never intercepts pointer events', () => {
    renderEditor(sceneWith('lib.jar', ['a.jar']));
    const svg = document.querySelector('.we-links');
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute('aria-hidden')).toBe('true');
  });

  it('renders a resolved mod icon inside the gradient tile frame', () => {
    const scene = sceneWith('icon-mod.jar', ['other.jar']);
    scene.content[0].iconUrl = 'https://cdn.example.test/icon.png';
    renderEditor(scene);
    const icon = document.querySelector('.we-slot[data-name="icon-mod.jar"] img');
    expect(icon).not.toBeNull();
    expect(icon).toHaveAttribute('src', 'https://cdn.example.test/icon.png');
    expect(icon?.parentElement).toHaveClass('tile-art');
  });
});
