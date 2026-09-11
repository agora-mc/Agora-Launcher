/**
 * IPC wrappers for the plugin commands.
 *
 * Thin on purpose: every one of these is a single `invoke`, because the
 * decisions live in `agora-core` and duplicating any of them here would mean
 * the GUI and the CLI could disagree about what a plugin is allowed to do.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  DiagnosticReport,
  InstallPreview,
  LaunchCheckOutcome,
  NamespacedContribution,
  PluginStartFailure,
  PluginSummary,
  RepairOutcome,
  RepairProposal,
  ViewModel,
} from './types';

export const pluginsEnabled = () => invoke<boolean>('plugins_enabled');

export const startPlugins = () => invoke<PluginStartFailure[]>('start_plugins');

export const listPlugins = () => invoke<PluginSummary[]>('list_plugins');

export const listPluginContributions = () =>
  invoke<NamespacedContribution[]>('list_plugin_contributions');

export const previewPluginPackage = (path: string) =>
  invoke<InstallPreview>('preview_plugin_package', { path });

export const previewPluginFolder = (path: string) =>
  invoke<InstallPreview>('preview_plugin_folder', { path });

export const installPluginPackage = (path: string, acceptCapabilities: boolean) =>
  invoke<PluginSummary>('install_plugin_package', { path, acceptCapabilities });

export const addPluginDevelopmentFolder = (path: string, acceptCapabilities: boolean) =>
  invoke<PluginSummary>('add_plugin_development_folder', { path, acceptCapabilities });

export const setPluginEnabled = (pluginId: string, enabled: boolean) =>
  invoke<void>('set_plugin_enabled', { pluginId, enabled });

export const uninstallPlugin = (pluginId: string, purgeData: boolean) =>
  invoke<void>('uninstall_plugin', { pluginId, purgeData });

/** The recovery path: stop and disable everything, without plugin cooperation. */
export const disableAllPlugins = () => invoke<number>('disable_all_plugins');

export const renderPluginView = (pluginId: string, exportName: string, args?: unknown) =>
  invoke<ViewModel>('render_plugin_view', { pluginId, export: exportName, args: args ?? null });

export const runPluginCommand = (pluginId: string, exportName: string, args?: unknown) =>
  invoke<unknown>('run_plugin_command', { pluginId, export: exportName, args: args ?? null });

export const runPluginDiagnostic = (pluginId: string, exportName: string, instanceId: string) =>
  invoke<DiagnosticReport>('run_plugin_diagnostic', { pluginId, export: exportName, instanceId });

export const applyPluginRepair = (pluginId: string, proposal: RepairProposal) =>
  invoke<RepairOutcome>('apply_plugin_repair', { pluginId, proposal });

/** Starts any plugin whose only activation event is `onInstanceOpened`. */
export const pluginInstanceOpened = () => invoke<number>('plugin_instance_opened');

export const runPluginLaunchChecks = (instanceId: string) =>
  invoke<LaunchCheckOutcome>('run_plugin_launch_checks', { instanceId });

export const setPluginSetting = (pluginId: string, key: string, value: unknown) =>
  invoke<void>('set_plugin_setting', { pluginId, key, value });

export const readPluginLog = (pluginId: string, lines?: number) =>
  invoke<string[]>('read_plugin_log', { pluginId, lines: lines ?? null });
