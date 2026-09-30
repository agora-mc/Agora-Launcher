/**
 * The editor must show the persisted result of a mutating operation without
 * being closed and reopened. Each test drives a real operation through the
 * editor, changes what the backend (mocked at the `invoke` boundary) reports,
 * and checks the visible state follows.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InstanceEditor } from './InstanceEditor';
import { PackInstallProvider } from '../components/PackInstallProgress';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

function row(overrides: Record<string, unknown> = {}) {
  return {
    instance_id: 'alpha',
    name: 'Alpha',
    minecraft_version: '26.2',
    loader: 'fabric',
    loader_version: '0.19.5',
    is_modpack: false,
    is_locked: false,
    last_launched_at: null,
    jvm_memory_mb: 4096,
    jvm_memory_mode: 'manual',
    jvm_gc: 'auto',
    jvm_custom_args: '',
    jvm_always_pre_touch: true,
    created_at: '2026-01-01T00:00:00Z',
    java_path: null,
    java_incompatible_override: false,
    icon_path: null,
    launch_mode_override: 'auto',
    import_source: null,
    ...overrides,
  };
}

function detail(rowOverrides: Record<string, unknown> = {}) {
  const r = row(rowOverrides);
  return {
    row: r,
    manifest: {
      instance_id: 'alpha',
      name: 'Alpha',
      created_from_pack: null,
      minecraft_version: r.minecraft_version,
      loader: 'fabric',
      loader_version: '0.19.5',
      is_locked: false,
      mods: [],
      resourcepacks: [],
      shaders: [],
      datapacks: [],
      worlds: [],
      user_preferences: {},
    },
    snapshot_readiness: 'ready',
    snapshot_error: null,
  };
}

let current = detail();
const template = {
  template_version: 1,
  id: 'tpl-1',
  name: 'Pre-touch off',
  description: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  jvm: { jvm_always_pre_touch: false },
  files: [],
};

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command).length;
}

beforeEach(() => {
  vi.clearAllMocks();
  current = detail();
  invokeMock.mockImplementation(async (command: string) => {
    switch (command) {
      case 'get_instance_detail': return current;
      case 'list_instance_content':
      case 'enrich_instance_content':
      case 'list_capturable_template_files':
      case 'get_dependency_graph':
      case 'list_loadout_profiles':
        return command === 'list_loadout_profiles'
          ? [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z' }]
          : [];
      case 'list_instance_templates': return [template];
      case 'list_snapshots':
        return [{
          id: 'snap-1', label: 'Before', created_at: '2026-01-01', file_count: 1,
          is_lkg: false, is_current_lkg: false, is_pre_restore: false,
        }];
      case 'apply_instance_template':
        current = detail({ jvm_always_pre_touch: false });
        return { jvm_applied: true, files_applied: 0, files_missing: 0, undo_snapshot_id: null };
      case 'restore_snapshot':
        current = detail({ minecraft_version: '26.1.2' });
        return undefined;
      case 'apply_loadout_profile':
        return undefined;
      case 'get_mod_groups': return {};
      default: return null;
    }
  });
});

async function openEditor() {
  render(<PackInstallProvider><InstanceEditor instanceId="alpha" onBack={() => undefined} /></PackInstallProvider>);
  await screen.findByRole('button', { name: 'Java & Args' });
}

describe('InstanceEditor reload after mutations', () => {
  it('shows applied template Java settings on the Java & Args tab', async () => {
    await openEditor();
    fireEvent.click(screen.getByRole('button', { name: 'Java & Args' }));
    const before = await screen.findByLabelText('Pre-touch allocated memory');
    expect((before as HTMLInputElement).checked).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Templates' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    await screen.findByText(/Applied Java settings/);

    fireEvent.click(screen.getByRole('button', { name: 'Java & Args' }));
    const after = await screen.findByLabelText('Pre-touch allocated memory');
    await waitFor(() => expect((after as HTMLInputElement).checked).toBe(false));
  });

  it('shows the restored state after a snapshot restore', async () => {
    await openEditor();
    expect(screen.getAllByText(/26\.2/).length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole('button', { name: 'Snapshots' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Restore' }));
    await screen.findByText('Snapshot restored.');
    await waitFor(() => expect(screen.getAllByText(/26\.1\.2/).length).toBeGreaterThan(0));
    expect(screen.queryAllByText(/26\.2/)).toHaveLength(0);
  });

  it('re-reads installed content after applying a loadout profile', async () => {
    await openEditor();
    fireEvent.click(screen.getByRole('button', { name: 'Loadout Profiles' }));
    const contentBefore = calls('list_instance_content');
    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    await screen.findByText('Profile "Fast" applied.');
    expect(calls('list_instance_content')).toBeGreaterThan(contentBefore);
    expect(calls('get_instance_detail')).toBeGreaterThan(1);
  });
});
