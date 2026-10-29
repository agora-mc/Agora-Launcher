/**
 * The instance page's Check readiness action runs the launch health check on
 * demand and hands its result to the Health Check dialog, even when the result
 * has no findings. A failed check says so instead of opening nothing.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InstanceEditor } from './InstanceEditor';
import { PackInstallProvider } from '../components/PackInstallProgress';
import type { HealthReport } from '../lib/tauri';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

function detail() {
  return {
    row: {
      instance_id: 'alpha', name: 'Alpha', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.9',
      is_modpack: false, is_locked: false, last_launched_at: null, jvm_memory_mb: 4096,
      jvm_memory_mode: 'manual', jvm_gc: 'auto', jvm_custom_args: '', jvm_always_pre_touch: true,
      created_at: '2026-01-01T00:00:00Z', java_path: null, java_incompatible_override: false,
      icon_path: null, launch_mode_override: 'auto', import_source: null,
    },
    manifest: {
      instance_id: 'alpha', name: 'Alpha', created_from_pack: null, minecraft_version: '1.21', loader: 'fabric',
      loader_version: '0.16.9', is_locked: false, mods: [], resourcepacks: [], shaders: [], datapacks: [],
      worlds: [], user_preferences: {},
    },
    snapshot_readiness: 'ready',
    snapshot_error: null,
    pack_gaps: [],
  };
}

const clean: HealthReport = { score: 'green', scan_token: 't', blockers: [], warnings: [], recommendations: [] };
let healthFailure: string | null = null;

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command);
}

beforeEach(() => {
  vi.clearAllMocks();
  healthFailure = null;
  invokeMock.mockImplementation(async (command: string) => {
    switch (command) {
      case 'get_instance_detail': return detail();
      case 'check_instance_health':
        if (healthFailure) throw new Error(healthFailure);
        return clean;
      case 'list_instance_content':
      case 'enrich_instance_content':
      case 'get_dependency_graph':
      case 'list_instance_templates':
      case 'list_snapshots':
      case 'list_capturable_template_files':
      case 'list_loadout_profiles':
        return [];
      case 'get_mod_groups': return {};
      default: return null;
    }
  });
});

describe('instance page readiness check', () => {
  it('runs the health check and opens its result, even when there are no findings', async () => {
    const onReviewHealth = vi.fn();
    render(
      <PackInstallProvider>
        <InstanceEditor instanceId="alpha" onBack={() => undefined} onReviewHealth={onReviewHealth} />
      </PackInstallProvider>,
    );
    fireEvent.click(await screen.findByRole('button', { name: 'Check readiness' }));

    await waitFor(() => expect(onReviewHealth).toHaveBeenCalledTimes(1));
    expect(calls('check_instance_health')).toHaveLength(1);
    expect(calls('check_instance_health')[0][1]).toEqual({ instanceId: 'alpha' });
    expect(onReviewHealth).toHaveBeenCalledWith('alpha', 'Alpha', clean);
  });

  it('says why nothing opened when the check fails', async () => {
    healthFailure = 'Health check could not read the instance.';
    const onReviewHealth = vi.fn();
    render(
      <PackInstallProvider>
        <InstanceEditor instanceId="alpha" onBack={() => undefined} onReviewHealth={onReviewHealth} />
      </PackInstallProvider>,
    );
    fireEvent.click(await screen.findByRole('button', { name: 'Check readiness' }));

    expect(await screen.findByText(/Health check could not read the instance\./)).toBeTruthy();
    expect(onReviewHealth).not.toHaveBeenCalled();
  });
});
