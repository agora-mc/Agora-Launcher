/**
 * Loadout profiles in the instance editor: a profile must say what it covers,
 * a blank name must not save anything, and applying one must say that an undo
 * point was taken first.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { InstanceEditor } from './InstanceEditor';
import { PackInstallProvider } from '../components/PackInstallProgress';

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

type Profile = {
  name: string;
  enabled_mods: string[];
  created_at: string;
  covers?: { mods: number; resourcepacks: number; shaders: number; datapacks: number; worlds: number } | null;
};

let profiles: Profile[] = [];

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command);
}

beforeEach(() => {
  vi.clearAllMocks();
  profiles = [];
  invokeMock.mockImplementation(async (command: string) => {
    switch (command) {
      case 'get_instance_detail': return detail();
      case 'list_instance_content':
      case 'enrich_instance_content':
      case 'get_dependency_graph':
      case 'list_instance_templates':
      case 'list_snapshots':
      case 'list_capturable_template_files':
        return [];
      case 'list_loadout_profiles': return profiles;
      case 'create_loadout_profile': return undefined;
      case 'apply_loadout_profile': return undefined;
      case 'get_mod_groups': return {};
      default: return null;
    }
  });
});

async function openLoadoutTab() {
  render(<PackInstallProvider><InstanceEditor instanceId="alpha" onBack={() => undefined} /></PackInstallProvider>);
  fireEvent.click(await screen.findByRole('button', { name: 'Loadout Profiles' }));
  await screen.findByRole('button', { name: 'Create Profile' });
}

describe('loadout profile creation', () => {
  it('keeps Create Profile disabled until the name has text in it', async () => {
    await openLoadoutTab();
    const create = screen.getByRole('button', { name: 'Create Profile' });
    expect(create).toBeDisabled();

    const input = screen.getByPlaceholderText('Profile name…');
    fireEvent.change(input, { target: { value: '   ' } });
    expect(create).toBeDisabled();
    expect(calls('create_loadout_profile')).toHaveLength(0);

    fireEvent.change(input, { target: { value: 'Winter' } });
    expect(create).toBeEnabled();
  });

  it('saves a profile under the trimmed name typed by the user', async () => {
    await openLoadoutTab();
    fireEvent.change(screen.getByPlaceholderText('Profile name…'), { target: { value: '  Winter  ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Create Profile' }));

    await waitFor(() => expect(calls('create_loadout_profile')).toHaveLength(1));
    expect(calls('create_loadout_profile')[0][1]).toEqual({ instanceId: 'alpha', name: 'Winter' });
  });
});

describe('loadout profile coverage', () => {
  it('says which kinds of item a profile switches on, in plain words', async () => {
    profiles = [{
      name: 'Fast',
      enabled_mods: ['sodium.jar', 'faithful.zip'],
      created_at: '2026-01-01T00:00:00Z',
      covers: { mods: 1, resourcepacks: 1, shaders: 0, datapacks: 0, worlds: 0 },
    }];
    await openLoadoutTab();

    expect(await screen.findByText(
      'Switches on: 1 mod, 1 resource pack, 0 shader packs, 0 data packs, 0 worlds. Mods it does not list are switched off; other items it does not list are left alone.',
    )).toBeTruthy();
  });

  it('says so for a profile saved before counts were recorded', async () => {
    profiles = [{ name: 'Old', enabled_mods: ['a.jar', 'b.jar'], created_at: '2026-01-01T00:00:00Z' }];
    await openLoadoutTab();

    expect(await screen.findByText(/Saved before counts were recorded: 2 items switched on\./)).toBeTruthy();
  });

  it('explains that Apply switches off unlisted mods but leaves unlisted packs and worlds alone', async () => {
    await openLoadoutTab();
    expect(screen.getByText(/A mod it does not list is switched off\./)).toBeTruthy();
    expect(screen.getByText(/Packs and worlds it does not list, such as ones added since it was saved, are left as they are\./)).toBeTruthy();
  });
});

describe('applying a loadout profile', () => {
  it('tells the user an undo point was saved before the switch', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));

    expect(await screen.findByText(/Profile "Fast" applied\. Before switching, Agora saved an undo point under Snapshots, labelled before-loadout/)).toBeTruthy();
    expect(calls('apply_loadout_profile')[0][1]).toEqual({ instanceId: 'alpha', profileName: 'Fast' });
  });
});
