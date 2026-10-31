/**
 * IPC wrappers for the FOMOD slice: an archive's installer (its steps and options), a preview of
 * the files a set of choices would install, and the install itself. Replies match
 * `games/fomod.rs`.
 */

import { invoke } from '@tauri-apps/api/core';

/** One group's picks, by name (the core's `Choice`). */
export interface FomodChoice {
  step: string;
  group: string;
  plugins: string[];
}

export type FomodPluginType =
  | 'required'
  | 'optional'
  | 'recommended'
  | 'not_usable'
  | 'could_be_usable'
  | 'dependent';

export interface FomodPluginView {
  name: string;
  description: string;
  pluginType: FomodPluginType;
}

export interface FomodGroupView {
  name: string;
  rule: string;
  singleChoice: boolean;
  plugins: FomodPluginView[];
}

export interface FomodStepView {
  name: string;
  visible: string | null;
  groups: FomodGroupView[];
}

export interface FomodInstallerView {
  itemId: string;
  title: string;
  author: string | null;
  version: string | null;
  description: string | null;
  requires: string | null;
  alwaysInstalledCount: number;
  conditionalInstallCount: number;
  steps: FomodStepView[];
}

export interface FomodShow {
  installer: FomodInstallerView;
  /** The proposed choices, when they were asked for. */
  suggestedChoices: FomodChoice[] | null;
}

export interface FomodPlannedFile {
  destination: string;
  source: string;
  size: number;
}

export interface FomodPlan {
  files: FomodPlannedFile[];
  totalSize: number;
  notes: string[];
  choices: FomodChoice[];
  dataFolder: string;
}

export interface FomodInstalled {
  itemId: string;
  name: string;
  fileCount: number;
  totalSize: number;
  notes: string[];
  addedToInstance: boolean;
  layerId: string | null;
  mountPath: string | null;
}

export type FomodInstallResult =
  | ({ status: 'preview' } & FomodPlan)
  | ({ status: 'installed' } & FomodInstalled);

function isShow(value: unknown): value is FomodShow {
  if (typeof value !== 'object' || value === null) return false;
  const show = value as Partial<FomodShow>;
  return typeof show.installer === 'object' && show.installer !== null
    && Array.isArray((show.installer as Partial<FomodInstallerView>).steps);
}

/**
 * An archive's installer. `useDefaults` also asks for the proposed choices (required and
 * recommended options). A refusal arrives as a rejection carrying the core's reason.
 */
export async function showFomod(contentId: string, instanceId: string | null, useDefaults: boolean): Promise<FomodShow> {
  const reply: unknown = await invoke<unknown>('games_fomod_show', { contentId, instanceId, useDefaults });
  if (!isShow(reply)) throw new Error('games_fomod_show did not return an installer.');
  return reply;
}

/** What the choices would install. Changes nothing. */
export const previewFomod = (contentId: string, instanceId: string | null, choices: FomodChoice[]) =>
  invoke<FomodInstallResult>('games_fomod_install', { contentId, instanceId, choices, preview: true });

/** Install the choices as a new store item, and add it to the instance when one is named. */
export const installFomod = (contentId: string, instanceId: string | null, choices: FomodChoice[]) =>
  invoke<FomodInstallResult>('games_fomod_install', { contentId, instanceId, choices, preview: false });
