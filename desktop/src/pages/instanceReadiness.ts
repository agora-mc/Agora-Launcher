/**
 * "Is this instance ready to play?" for the line under the Play button.
 *
 * Built only from data the instance page already holds: the instance row, the
 * health report the health monitor produced, the Java settings and whether
 * something is busy with the instance. It never launches, never probes Java
 * and never touches the network. The Java major is an estimate; the launch
 * path reads the real requirement from the game's version file.
 */

import { problemsText, warningsText } from '@/lib/healthSummary';

export type ReadinessLevel = 'ready' | 'check' | 'blocked';

export type JavaRuntimeMode = 'automatic' | 'prompt' | 'manual';

export interface ReadinessInput {
  minecraftVersion: string;
  loader: string;
  loaderVersion: string;
  /** The instance's own Java path, or null to let Agora choose. */
  javaPath: string | null;
  /** The global Java runtime mode, or null while it is unknown (loading or unreadable). */
  javaMode: JavaRuntimeMode | null;
  /** Estimated Java major for this Minecraft version. */
  javaMajor: number;
  /** Health counts, or null when no health check has run for this instance yet. */
  health: { blockers: number; warnings: number } | null;
  /**
   * Pack entries the instance still lacks (failed, or waiting on a user action),
   * or null while the instance's record has not loaded.
   */
  packGaps: number | null;
  /** True while something holds the instance (a pack install, a recovery snapshot). */
  busy: boolean;
}

export interface Readiness {
  level: ReadinessLevel;
  headline: string;
  /** Short facts about the game, mods and Java, in display order. */
  details: string[];
}

const LOADER_NAMES: Record<string, string> = {
  fabric: 'Fabric',
  forge: 'Forge',
  neoforge: 'NeoForge',
  quilt: 'Quilt',
};

export function instanceReadiness(input: ReadinessInput): Readiness {
  const minecraft = input.minecraftVersion.trim();
  const loader = input.loader.trim().toLowerCase();
  const loaderVersion = input.loaderVersion.trim();

  // Checked in order of how much each one blocks, so the headline names the first problem.
  let blocker: string | null = null;
  let check: string | null = null;

  if (input.busy) {
    blocker = 'something is still running on this instance';
  } else if (!minecraft) {
    blocker = 'the Minecraft version is missing';
  } else if (!loader) {
    blocker = 'the mod loader is not recorded';
  } else if (loader !== 'vanilla' && !loaderVersion) {
    blocker = `the ${LOADER_NAMES[loader] ?? loader} loader version is missing`;
  }

  // A pack mod that failed or still needs an action is content the instance does
  // not have, so the instance is not ready however healthy its installed mods look.
  let packLine: string | null = null;
  if (input.packGaps === null) {
    check ??= 'pack mods not checked yet';
    packLine = 'Pack mods not checked yet';
  } else if (input.packGaps > 0) {
    blocker ??= `${plural(input.packGaps, 'pack mod is', 'pack mods are')} not installed`;
    packLine = `${plural(input.packGaps, 'pack mod', 'pack mods')} not installed`;
  }

  const details: string[] = [];
  if (minecraft) {
    if (loader === 'vanilla') {
      details.push(`Minecraft ${minecraft}, no mod loader`);
    } else if (loader) {
      details.push(`Minecraft ${minecraft} with ${LOADER_NAMES[loader] ?? loader}${loaderVersion ? ` ${loaderVersion}` : ''}`);
    }
  }

  if (input.health === null) {
    check ??= 'mods not checked yet';
    details.push('Mods not checked yet');
  } else if (input.health.blockers > 0) {
    blocker ??= problemsText(input.health.blockers);
    details.push(problemsText(input.health.blockers));
  } else if (input.health.warnings > 0) {
    check ??= warningsText(input.health.warnings);
    details.push(warningsText(input.health.warnings));
  } else {
    details.push('Mods look healthy');
  }
  if (packLine) {
    details.push(packLine);
  }

  const major = input.javaMajor;
  if (input.javaPath) {
    details.push('Java: your own install (set in Java & Args)');
  } else if (input.javaMode === 'automatic') {
    details.push(`Java ${major} downloads on first launch if it isn't already installed`);
  } else if (input.javaMode === 'prompt') {
    details.push(`Java ${major} is needed: Agora will ask before downloading it`);
  } else if (input.javaMode === 'manual') {
    details.push(`Java ${major} is needed: automatic downloads are off, so install it or pick it in Java & Args`);
  } else {
    details.push(`Java ${major} is needed`);
  }

  if (blocker) {
    return { level: 'blocked', headline: `Not ready: ${blocker}`, details };
  }
  if (check) {
    return { level: 'check', headline: `Almost ready: ${check}`, details };
  }
  return { level: 'ready', headline: 'Ready to play', details };
}
