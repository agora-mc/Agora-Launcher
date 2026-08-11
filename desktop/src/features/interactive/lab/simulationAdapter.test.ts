import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { getScenario, listScenarios, simulationSource } from './simulationAdapter';
import { isGuideTopicId, isStandardDestination } from '../domain/intents';

const BOUNDARY_SCRIPT = join(process.cwd(), 'scripts', 'check-interactive-boundaries.mjs');

describe('simulationAdapter', () => {
  it('exposes the six Lab adventures', () => {
    const scenarios = listScenarios();
    expect(scenarios.map((scenario) => scenario.id).sort()).toEqual([
      'build',
      'fix',
      'heal',
      'mod',
      'offline',
      'undo',
    ]);
    for (const scenario of scenarios) {
      expect(getScenario(scenario.id)).toBe(scenario);
    }
  });

  it('builds an explicit simulation source', () => {
    expect(simulationSource('build', 1)).toEqual({
      kind: 'simulation',
      scenarioId: 'build',
      scenarioVersion: 1,
    });
  });

  it('enforces the Lab import boundary (no live authority in lab code)', () => {
    // Runs the same automated check wired into `npm run build`.
    const result = execFileSync(process.execPath, [BOUNDARY_SCRIPT], { encoding: 'utf8' });
    expect(result).toContain('import-boundary check OK');
  });

  it('enforces the boundary negative fixtures: every bypass shape is rejected (BLOCKER 1)', () => {
    const fixturesRoot = join(process.cwd(), 'scripts', 'boundary-fixtures', 'interactive');
    const result = execFileSync(
      process.execPath,
      [BOUNDARY_SCRIPT, '--root', fixturesRoot, '--fixtures'],
      { encoding: 'utf8' },
    );
    expect(result).toContain('fixtures OK');
    expect(result).toContain('every fixture file produced a violation');
  });
});

describe('scenario contracts', () => {
  const scenarios = listScenarios();

  it('uses namespaced simulated ids for every scene entity', () => {
    for (const scenario of scenarios) {
      const scene = scenario.initialScene(0);
      const ids = [
        ...scene.content.map((node) => node.id),
        ...scene.relationships.map((relationship) => [relationship.id, relationship.fromId]),
        ...scene.relationships.map((relationship) => relationship.toId ?? []),
        ...(scene.instance ? [scene.instance.id] : []),
      ].flat();
      for (const id of ids) {
        expect(id.startsWith(`lab:${scenario.id}:`)).toBe(true);
      }
    }
  });

  it('is deterministic: initialScene returns identical scenes per checkpoint', () => {
    for (const scenario of scenarios) {
      for (let checkpoint = 0; checkpoint < scenario.checkpoints.length; checkpoint += 1) {
        expect(scenario.initialScene(checkpoint)).toEqual(scenario.initialScene(checkpoint));
      }
    }
  });

  it('carries a simulation source on every canonical scene', () => {
    for (const scenario of scenarios) {
      for (let checkpoint = 0; checkpoint < scenario.checkpoints.length; checkpoint += 1) {
        expect(scenario.initialScene(checkpoint).source).toEqual({
          kind: 'simulation',
          scenarioId: scenario.id,
          scenarioVersion: scenario.version,
        });
      }
    }
  });

  it('validates guide topics and real destinations against the closed sets', () => {
    for (const scenario of scenarios) {
      expect(scenario.guideTopics.length).toBeGreaterThan(0);
      for (const topicId of scenario.guideTopics) {
        expect(isGuideTopicId(topicId)).toBe(true);
      }
      for (const dest of scenario.realDestinations) {
        expect(isStandardDestination(dest)).toBe(true);
      }
    }
  });

  it('declares a completion message and at least one checkpoint', () => {
    for (const scenario of scenarios) {
      expect(scenario.completionMessage.length).toBeGreaterThan(0);
      expect(scenario.checkpoints.length).toBeGreaterThan(0);
    }
  });

  it('never starts completed', () => {
    for (const scenario of scenarios) {
      const state = {
        scenarioId: scenario.id,
        scenarioVersion: scenario.version,
        checkpoint: 0,
        scene: scenario.initialScene(0),
        status: 'in-progress' as const,
        lastFeedback: null,
      };
      expect(scenario.successPredicate(state)).toBe(false);
    }
  });

  it('provides reachable decisions at every checkpoint', () => {
    for (const scenario of scenarios) {
      for (let checkpoint = 0; checkpoint < scenario.checkpoints.length; checkpoint += 1) {
        const state = {
          scenarioId: scenario.id,
          scenarioVersion: scenario.version,
          checkpoint,
          scene: scenario.initialScene(checkpoint),
          status: 'in-progress' as const,
          lastFeedback: null,
        };
        const decisions = scenario.checkpoints[checkpoint].decisionsFor(state);
        expect(decisions.length).toBeGreaterThan(0);
      }
    }
  });
});
