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
// Enabled mods the profile does not list, as the core reports them.
let unlisted: string[] = [];
let unlistedFailure: string | null = null;
// What saving a profile again from the setup would change, as the core previews it.
let updateChanges: unknown[] = [];

function calls(command: string) {
  return invokeMock.mock.calls.filter(([name]) => name === command);
}

beforeEach(() => {
  vi.clearAllMocks();
  profiles = [];
  unlisted = [];
  unlistedFailure = null;
  updateChanges = [];
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
      case 'list_loadout_unlisted_mods':
        if (unlistedFailure) throw new Error(unlistedFailure);
        return unlisted;
      case 'preview_loadout_update': return { name: 'Fast', changes: updateChanges };
      case 'update_loadout_profile': return { name: 'Fast', changes: updateChanges };
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
      'Switches on: 1 mod, 1 resource pack, 0 shader packs, 0 data packs, 0 worlds. Mods it does not list are asked about when you apply; other items it does not list are left alone.',
    )).toBeTruthy();
  });

  it('says so for a profile saved before counts were recorded', async () => {
    profiles = [{ name: 'Old', enabled_mods: ['a.jar', 'b.jar'], created_at: '2026-01-01T00:00:00Z' }];
    await openLoadoutTab();

    expect(await screen.findByText(/Saved before counts were recorded: 2 items switched on\./)).toBeTruthy();
  });

  it('explains that Apply asks about unlisted mods but leaves unlisted packs and worlds alone', async () => {
    await openLoadoutTab();
    expect(screen.getByText(/A mod it does not list, such as one added since it was saved, is asked about when you apply/)).toBeTruthy();
    expect(screen.getByText(/Packs and worlds it does not list are left as they are\./)).toBeTruthy();
  });
});

describe('applying a loadout profile', () => {
  it('tells the user an undo point was saved before the switch', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));

    expect(await screen.findByText(/Profile "Fast" applied\. Before switching, Agora saved an undo point under Snapshots, labelled before-loadout/)).toBeTruthy();
    expect(calls('apply_loadout_profile')[0][1]).toEqual({ instanceId: 'alpha', profileName: 'Fast', unlistedMods: 'keep' });
  });

  it('applies without asking when every enabled mod is listed, since there is nothing to switch off', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));

    await waitFor(() => expect(calls('apply_loadout_profile')).toHaveLength(1));
    expect(screen.queryByText('Mods this profile does not list')).toBeNull();
  });

  it('asks about unlisted mods, naming them, and applies with Keep when the user keeps them', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    unlisted = ['newer.jar', 'extra.jar'];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    expect(await screen.findByText('Mods this profile does not list')).toBeTruthy();
    expect(screen.getByText('newer.jar')).toBeTruthy();
    expect(screen.getByText('extra.jar')).toBeTruthy();
    expect(calls('apply_loadout_profile')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: 'Keep them on' }));
    await waitFor(() => expect(calls('apply_loadout_profile')).toHaveLength(1));
    expect(calls('apply_loadout_profile')[0][1]).toEqual({ instanceId: 'alpha', profileName: 'Fast', unlistedMods: 'keep' });
  });

  it('switches unlisted mods off only when the user chooses that', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    unlisted = ['newer.jar'];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Switch them off' }));

    await waitFor(() => expect(calls('apply_loadout_profile')).toHaveLength(1));
    expect(calls('apply_loadout_profile')[0][1]).toEqual({ instanceId: 'alpha', profileName: 'Fast', unlistedMods: 'off' });
  });

  it('applies nothing when the user cancels the question', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    unlisted = ['newer.jar'];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByText('Mods this profile does not list')).toBeNull());
    expect(calls('apply_loadout_profile')).toHaveLength(0);
  });

  it('applies nothing and says why when the unlisted mods cannot be read', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z', covers: null }];
    unlistedFailure = 'Profile could not be read.';
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));

    expect(await screen.findByText(/Profile could not be read\./)).toBeTruthy();
    expect(calls('apply_loadout_profile')).toHaveLength(0);
  });
});

describe('updating a loadout profile from the current setup', () => {
  const changes = [
    { filename: 'newer.jar', content_type: 'mod', kind: { added: { enabled: true } } },
    { filename: 'old.jar', content_type: 'mod', kind: { removed: { enabled: true } } },
    { filename: 'lithium.jar', content_type: 'mod', kind: 'turned_off' },
  ];

  it('shows every change first and saves nothing until the user confirms', async () => {
    profiles = [{ name: 'Fast', enabled_mods: ['old.jar'], created_at: '2026-01-01T00:00:00Z' }];
    updateChanges = changes;
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Update from setup' }));
    expect(await screen.findByText('Update "Fast" from the current setup?')).toBeTruthy();
    expect(screen.getByText('Add newer.jar (on)')).toBeTruthy();
    expect(screen.getByText('Drop old.jar, which is no longer on this instance')).toBeTruthy();
    expect(screen.getByText('Turn off lithium.jar')).toBeTruthy();
    expect(calls('update_loadout_profile')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByText('Update "Fast" from the current setup?')).toBeNull());
    expect(calls('update_loadout_profile')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: 'Update from setup' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Save changes' }));
    await waitFor(() => expect(calls('update_loadout_profile')).toHaveLength(1));
    expect(calls('update_loadout_profile')[0][1]).toEqual({ instanceId: 'alpha', profileName: 'Fast' });
    expect(await screen.findByText('Profile "Fast" now records the current setup of this instance.')).toBeTruthy();
  });

  it('says there is nothing to save, and asks nothing, when the profile already matches', async () => {
    profiles = [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z' }];
    updateChanges = [];
    await openLoadoutTab();

    fireEvent.click(await screen.findByRole('button', { name: 'Update from setup' }));
    expect(await screen.findByText('Profile "Fast" already matches the current setup. Nothing to save.')).toBeTruthy();
    expect(screen.queryByText(/from the current setup\?/)).toBeNull();
    expect(calls('update_loadout_profile')).toHaveLength(0);
  });
});
