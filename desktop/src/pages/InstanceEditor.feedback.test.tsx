/**
 * Feedback the editor shows while data is still arriving, and banners that
 * belong to one instance: the optional-dependency count must match its list,
 * rows must say they are loading before they say "Unknown", and a status from
 * one instance must not follow the user to another.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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

function contentRow(filename: string, overrides: Record<string, unknown> = {}) {
  return {
    key: `mod:${filename}:abc`, filename, display_name: filename.replace(/\.jar$/, ''), version: '1.0.0',
    content_type: 'mod', enabled: true, installed_at: '2026-07-01T00:00:00Z', source: 'modrinth',
    source_label: 'Modrinth', pack_managed: false, installed_as_dependency: false, update_pinned: false,
    source_url: null, registry_id: null, modrinth_id: 'proj', mod_jar_id: null, loader_mod_id: null,
    size_bytes: 10, file_present: true, resolved_path: null, author: null, categories: ['Utility'],
    icon_url: null, curation_status: 'unknown', agora_score: null, modrinth_downloads: null,
    metadata_status: 'partial', ...overrides,
  };
}

let edges: unknown[] = [];
let edgesGate: Promise<void> | null = null;
let content: unknown[] = [];
let enrichment: Promise<unknown[]> = Promise.resolve([]);

beforeEach(() => {
  vi.clearAllMocks();
  edges = [];
  edgesGate = null;
  content = [];
  enrichment = Promise.resolve([]);
  invokeMock.mockImplementation(async (command: string, args: { instanceId?: string } = {}) => {
    switch (command) {
      case 'get_instance_detail': return detail(args.instanceId ?? 'alpha');
      case 'list_instance_content': return content;
      case 'enrich_instance_content': return enrichment;
      case 'get_dependency_graph':
        if (edgesGate) await edgesGate;
        return edges;
      case 'list_loadout_profiles':
        return [{ name: 'Fast', enabled_mods: [], created_at: '2026-01-01T00:00:00Z' }];
      case 'list_instance_templates':
      case 'list_snapshots':
      case 'list_capturable_template_files':
        return [];
      case 'get_mod_groups': return {};
      default: return null;
    }
  });
});

function editor(instanceId: string) {
  return <PackInstallProvider><InstanceEditor instanceId={instanceId} onBack={() => undefined} /></PackInstallProvider>;
}

describe('optional dependency count', () => {
  it('shows a placeholder while the graph resolves, then exactly the entries the overlay lists', async () => {
    let release: () => void = () => undefined;
    edgesGate = new Promise<void>((resolve) => { release = resolve; });
    // Three owners recommending the same add-on: three entries in the overlay.
    edges = ['a', 'b', 'c'].map((owner) => ({
      from_filename: `${owner}.jar`, to_filename: 'shared-addon.jar', requirement: 'optional',
    }));
    render(editor('alpha'));
    const button = await screen.findByTestId('optional-deps-button');
    expect(button.textContent).toContain('(…)');
    expect(button.textContent).not.toMatch(/\(\d+\)/);

    await act(async () => { release(); });
    await waitFor(() => expect(button.textContent).toContain('(3)'));

    fireEvent.click(button);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getAllByText('recommends')).toHaveLength(3);
  });
});

describe('installed content details', () => {
  it('shows a loading state, not "Unknown", until enrichment finishes', async () => {
    let finish: (value: unknown[]) => void = () => undefined;
    enrichment = new Promise<unknown[]>((resolve) => { finish = resolve; });
    const row = contentRow('lookup-me.jar');
    content = [row];
    render(editor('alpha'));
    await screen.findByText('lookup-me');
    expect(screen.getByRole('status', { name: 'Loading author' })).toBeTruthy();
    expect(screen.getByText('Loading details…')).toBeTruthy();
    expect(within(screen.getByRole('table')).queryByText('Unknown')).toBeNull();

    await act(async () => { finish([{ key: row.key, display_name: 'Lookup Mod', icon_url: null, author: 'Some Author' }]); });
    await screen.findByText('Some Author');
    expect(screen.queryByText('Loading details…')).toBeNull();
  });

  it('settles on "Unknown" once enrichment finishes with no author', async () => {
    content = [contentRow('nothing-found.jar')];
    render(editor('alpha'));
    await screen.findByText('nothing-found');
    await within(screen.getByRole('table')).findByText('Unknown');
    expect(screen.queryByText('Loading details…')).toBeNull();
  });

  it('does not wait on rows that have nothing to look up', async () => {
    enrichment = new Promise<unknown[]>(() => undefined);
    content = [contentRow('hand-made.jar', { modrinth_id: null, metadata_status: 'unavailable' })];
    render(editor('alpha'));
    await screen.findByText('hand-made');
    expect(within(screen.getByRole('table')).getByText('Unknown')).toBeTruthy();
  });
});

describe('instance banners', () => {
  it('drops a status message when the editor moves to another instance', async () => {
    const view = render(editor('alpha'));
    fireEvent.click(await screen.findByRole('button', { name: 'Loadout Profiles' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    await screen.findByText('Profile "Fast" applied.');

    view.rerender(editor('beta'));
    await waitFor(() => expect(screen.queryByText('Profile "Fast" applied.')).toBeNull());
  });
});
