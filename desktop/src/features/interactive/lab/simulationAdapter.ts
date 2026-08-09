/**
 * Simulation adapter: the only way Lab scenes come into existence.
 *
 * Sol-0 contract: `docs/interactive/MASTER_ARCHITECTURE.md` §8. It reads
 * authored scenario fixtures only, uses namespaced simulated IDs, and
 * calculates outcomes from deterministic lesson rules. It never imports
 * backend DTOs, never accepts a real instance ID, and never accepts an
 * operation callback.
 *
 * This is the hardest Lab boundary: enforced by
 * `scripts/check-interactive-boundaries.mjs`.
 */

import type { ExperienceSource } from '../domain/models';
import type { LabScenario } from './scenarioTypes';
import { buildItScenario } from './scenarios/buildIt';
import { modItScenario } from './scenarios/modIt';
import { undoItScenario } from './scenarios/undoIt';

const SCENARIOS: LabScenario[] = [buildItScenario, modItScenario, undoItScenario];

/** Explicit simulation origin for a scene. */
export function simulationSource(scenarioId: string, scenarioVersion: number): ExperienceSource {
  return { kind: 'simulation', scenarioId, scenarioVersion };
}

export function listScenarios(): LabScenario[] {
  return SCENARIOS;
}

export function getScenario(scenarioId: string): LabScenario | undefined {
  return SCENARIOS.find((scenario) => scenario.id === scenarioId);
}
