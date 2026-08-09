import { describe, expect, it } from 'vitest';
import {
  hasProposal,
  hasProposalInFlight,
  isStaged,
  removeProposal,
  setProposalPhase,
  stageProposal,
  stagedValue,
} from './state';
import type { VisualProposal, VisualScene } from './models';

const proposal: VisualProposal = {
  id: 'p1',
  intent: { kind: 'propose-remove', contentId: 'c1' },
  phase: 'proposed',
  title: 'Remove thing',
  summary: 'removes c1',
  destructive: true,
};

function scene(proposals: VisualProposal[] = []): VisualScene {
  return {
    source: { kind: 'simulation', scenarioId: 'test', scenarioVersion: 1 },
    content: [],
    relationships: [],
    findings: [],
    proposals,
  };
}

describe('domain/state', () => {
  it('stages a proposal by id without duplicating', () => {
    const staged = stageProposal(scene(), proposal);
    expect(staged.proposals).toHaveLength(1);
    const again = stageProposal(staged, proposal);
    expect(again.proposals).toHaveLength(1);
  });

  it('transitions a proposal phase immutably', () => {
    const staged = stageProposal(scene(), proposal);
    const applying = setProposalPhase(staged, 'p1', 'applying');
    expect(applying.proposals[0].phase).toBe('applying');
    expect(staged.proposals[0].phase).toBe('proposed');
  });

  it('does not mutate when phase target is unknown', () => {
    const staged = stageProposal(scene(), proposal);
    const same = setProposalPhase(staged, 'nope', 'applying');
    expect(same).toBe(staged);
  });

  it('removes a proposal (committed outcome)', () => {
    const staged = stageProposal(scene(), proposal);
    const removed = removeProposal(staged, 'p1');
    expect(removed.proposals).toHaveLength(0);
  });

  it('detects proposals in flight (in-review or applying)', () => {
    const staged = stageProposal(scene(), proposal);
    expect(hasProposalInFlight(staged)).toBe(false);
    const applying = setProposalPhase(staged, 'p1', 'applying');
    expect(hasProposalInFlight(applying)).toBe(true);
  });

  it('reports staged status only for proposed phase', () => {
    const staged = stageProposal(scene(), proposal);
    expect(isStaged(staged, 'p1')).toBe(true);
    const rejected = setProposalPhase(staged, 'p1', 'rejected');
    expect(isStaged(rejected, 'p1')).toBe(false);
  });

  it('hasProposal and stagedValue handle current/proposed', () => {
    const value = { current: 'a', proposed: 'b' };
    expect(hasProposal(value)).toBe(true);
    expect(stagedValue(value)).toBe('b');
    expect(stagedValue({ current: 'a' })).toBe('a');
    expect(hasProposal({ current: 'a' })).toBe(false);
  });
});
