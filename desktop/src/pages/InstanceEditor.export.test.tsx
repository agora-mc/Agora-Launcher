/**
 * The export tab. A pack or lockfile is written only to a file the user picks;
 * a cancelled dialog changes nothing; Explorer opens only when asked for; the
 * saved-file row can open its folder; the status banner can be dismissed.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InstanceEditor } from './InstanceEditor';
import { PackInstallProvider } from '../components/PackInstallProgress';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

function detail(id: string) {
  return {
    row: {
      instance_id: id, name: id, minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16.9',
      is_modpack: false, is_locked: false, last_launched_at: null, jvm_memory_mb: 4096,
      jvm_memory_mode: 'manual', jvm_gc: 'auto', jvm_custom_args: '', jvm_always_pre_touch: true,
      created_at: '2026-01-01T00:00:00Z', java_path: null, java_incompatible_override: false,
      icon_path: null, launch_mode_override: 'auto', import_source: null,
    },
    manifest: {
      instance_id: id, name: id, created_from_pack: null, minecraft_version: '1.21', loader: 'fabric',
      loader_version: '0.16.9', is_locked: false, mods: [], resourcepacks: [], shaders: [], datapacks: [],
      worlds: [], user_preferences: {},
    },
    snapshot_readiness: 'ready',
    snapshot_error: null,
  };
}

const LOCKFILE = { formatVersion: 1, instanceId: 'alpha', artifacts: [] };

// Per-test answers for the two save commands: a path, or null for "cancelled".
let packAnswer: string | null = null;
let lockfileAnswer: string | null = null;

beforeEach(() => {
  vi.clearAllMocks();
  packAnswer = null;
  lockfileAnswer = null;
  invokeMock.mockImplementation(async (command: string, args: { instanceId?: string } = {}) => {
    switch (command) {
      case 'get_instance_detail': return detail(args.instanceId ?? 'alpha');
      case 'list_instance_content': return [];
      case 'enrich_instance_content': return [];
      case 'get_dependency_graph': return [];
      case 'list_loadout_profiles': return [];
      case 'list_instance_templates':
      case 'list_snapshots':
      case 'list_capturable_template_files':
        return [];
      case 'get_mod_groups': return {};
      case 'export_instance_pack': return packAnswer;
      case 'export_lockfile': return LOCKFILE;
      case 'save_lockfile_export': return lockfileAnswer;
      default: return null;
    }
  });
});

function editor(instanceId: string) {
  return <PackInstallProvider><InstanceEditor instanceId={instanceId} onBack={() => undefined} /></PackInstallProvider>;
}

async function openExportTab(instanceId = 'alpha') {
  const view = render(editor(instanceId));
  fireEvent.click(await screen.findByRole('button', { name: 'Export' }));
  await screen.findByRole('heading', { name: 'Export Instance' });
  return view;
}

describe('pack export', () => {
  it('a cancelled save dialog changes nothing and opens nothing', async () => {
    packAnswer = null;
    await openExportTab();
    fireEvent.click(screen.getByRole('button', { name: 'Export .mrpack' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('export_instance_pack', { instanceId: 'alpha', format: 'mrpack' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Export .mrpack' })).toBeTruthy());
    expect(screen.queryByText(/pack saved to:/)).toBeNull();
    expect(screen.queryByRole('button', { name: 'Show in folder' })).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith('reveal_path', expect.anything());
  });

  it('a saved pack shows its location, and Explorer opens only from Show in folder', async () => {
    packAnswer = 'D:/data/exports/alpha.mrpack';
    await openExportTab();
    fireEvent.click(screen.getByRole('button', { name: 'Export .mrpack' }));

    expect(await screen.findByText('Modrinth pack saved to: D:/data/exports/alpha.mrpack')).toBeTruthy();
    expect(screen.getByText('Saved to D:/data/exports/alpha.mrpack')).toBeTruthy();
    expect(invokeMock).not.toHaveBeenCalledWith('reveal_path', expect.anything());

    fireEvent.click(screen.getByRole('button', { name: 'Show in folder' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('reveal_path', { path: 'D:/data/exports/alpha.mrpack' }));
  });

  it('forgets the saved location when the editor moves to another instance', async () => {
    packAnswer = 'D:/data/exports/alpha.mrpack';
    const view = await openExportTab('alpha');
    fireEvent.click(screen.getByRole('button', { name: 'Export .mrpack' }));
    await screen.findByText('Saved to D:/data/exports/alpha.mrpack');

    view.rerender(editor('beta'));
    await waitFor(() => expect(screen.queryByText('Saved to D:/data/exports/alpha.mrpack')).toBeNull());
  });

  it('the status banner has a dismiss control', async () => {
    packAnswer = 'D:/data/exports/alpha.json';
    await openExportTab();
    fireEvent.click(screen.getByRole('button', { name: 'Export agora-pack.json' }));
    await screen.findByText('Agora pack saved to: D:/data/exports/alpha.json');

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss message' }));
    expect(screen.queryByText(/pack saved to:/)).toBeNull();
  });

  it('says configs and keybinds are not in either pack', async () => {
    await openExportTab();
    // Both packs and the lockfile each say what they leave out.
    expect(screen.getAllByText('Not included:')).toHaveLength(3);
    expect(screen.queryByText(/config files, and overrides/)).toBeNull();
  });

  it('says the lockfile holds config fingerprints only, and no memory or Java settings', async () => {
    await openExportTab();
    expect(screen.getByText(/config file contents, memory and Java settings/)).toBeTruthy();
    expect(screen.queryByText(/all settings/)).toBeNull();
  });

  it('format badges use the named type scale, not a fixed 10px', async () => {
    await openExportTab();
    for (const badge of ['Recommended', 'Agora native', 'Advanced']) {
      const className = screen.getByText(badge).className;
      expect(className, badge).toContain('text-xs');
      expect(className, badge).not.toContain('text-[10px]');
    }
  });
});

describe('in-instance pack import', () => {
  it('is labelled as creating a new instance, and its file dialog says so too', async () => {
    render(editor('alpha'));
    fireEvent.click(await screen.findByRole('button', { name: /Import pack as new instance$/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('pick_open_file', {
      title: 'Import pack as new instance',
      extensions: ['mrpack', 'agora-pack.json', 'json'],
    }));
    expect(screen.queryByRole('button', { name: /Import Pack$/ })).toBeNull();
  });
});

describe('lockfile save', () => {
  async function exportLockfileFirst() {
    await openExportTab();
    fireEvent.click(screen.getByRole('button', { name: 'Export Lockfile' }));
    return screen.findByRole('button', { name: 'Save to file…' });
  }

  it('saves the lockfile text to the chosen file and shows where it went', async () => {
    lockfileAnswer = 'D:/data/exports/alpha.lockfile.json';
    fireEvent.click(await exportLockfileFirst());

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('save_lockfile_export', {
      instanceId: 'alpha',
      contents: expect.stringContaining('"formatVersion": 1'),
    }));
    expect(await screen.findByText('Lockfile saved.')).toBeTruthy();
    expect(screen.getByText('Saved to D:/data/exports/alpha.lockfile.json')).toBeTruthy();
  });

  it('a cancelled lockfile save says nothing', async () => {
    lockfileAnswer = null;
    fireEvent.click(await exportLockfileFirst());

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('save_lockfile_export', expect.anything()));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save to file…' })).toBeTruthy());
    expect(screen.queryByText('Lockfile saved.')).toBeNull();
    expect(screen.queryByText(/^Saved to /)).toBeNull();
  });
});
