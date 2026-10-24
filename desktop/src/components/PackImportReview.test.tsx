import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { PackImportReview } from './PackImportReview';
import type { PackImportPlan } from '../lib/tauri';

const plan: PackImportPlan = {
  pack_name: 'Friend Pack',
  instance: { id: null, name: 'Friend Pack', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16' },
  entries: [
    { name: 'on.jar', display_name: null, source: 'modrinth', checkable: true, reason: null, action: null, content_type: 'mod' },
    { name: 'off.jar', display_name: null, source: 'modrinth', checkable: true, reason: null, action: null, arrives_disabled: true, content_type: 'mod' },
  ],
  file_sha256: 'x',
  file_size: 1,
  name_taken: false,
  suggested_name: 'Friend Pack',
  left_out: [],
  problems: [],
  soft_problems: [],
  notes: [],
};

const mrpackPlan: PackImportPlan = {
  ...plan,
  pack_name: 'Spiff',
  entries: [
    { name: 'mods/sodium.jar', display_name: 'sodium.jar', source: 'modrinth', checkable: true, reason: null, action: null, content_type: 'mod' },
    { name: 'mods/homebrew.jar', display_name: 'homebrew.jar', source: 'bundled', checkable: true, reason: null, action: null, content_type: 'mod' },
    { name: 'shaderpacks/BSL.zip', display_name: 'BSL.zip', source: 'bundled', checkable: false, reason: null, action: null, content_type: 'shader' },
  ],
  name_taken: true,
  suggested_name: 'Spiff (2)',
  left_out: ['saves/world/level.dat (outside the folders Agora copies from a pack)'],
  notes: ['Also copies 3 settings and data files (config, scripts and the like) from the pack.'],
};

function review(overrides: Partial<React.ComponentProps<typeof PackImportReview>> = {}) {
  return render(
    <PackImportReview
      label="p.mrpack"
      plan={plan}
      error={null}
      name={plan.suggested_name}
      onNameChange={() => undefined}
      onImport={() => undefined}
      onCancel={() => undefined}
      {...overrides}
    />,
  );
}

describe('pack import review', () => {
  it('shows which mods arrive disabled, and only those', () => {
    review();
    expect(screen.getAllByText(/arrives disabled/)).toHaveLength(1);
  });

  it('for a .mrpack shows the suggested free name, where files come from, and what is left out', () => {
    review({ plan: mrpackPlan, name: mrpackPlan.suggested_name });
    expect((screen.getByLabelText('Instance name') as HTMLInputElement).value).toBe('Spiff (2)');
    expect(screen.getByText(/already exists, so this will be imported as a separate copy/)).toBeTruthy();
    expect(screen.getByText('1 from Modrinth, 2 bundled in the pack')).toBeTruthy();
    expect(screen.getAllByText(/Bundled in the pack/)).toHaveLength(2);
    expect(screen.getByLabelText('Left out of the import').textContent).toContain('saves/world/level.dat');
    expect(screen.getByText(/Also copies 3 settings and data files/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Import as a copy' })).toBeTruthy();
  });

  it('reports the name the user typed, and does not offer an import with no name', () => {
    const onNameChange = vi.fn();
    const { rerender } = review({ plan: mrpackPlan, name: 'Spiff (2)', onNameChange });
    fireEvent.change(screen.getByLabelText('Instance name'), { target: { value: 'Spiff for Sam' } });
    expect(onNameChange).toHaveBeenCalledWith('Spiff for Sam');
    rerender(
      <PackImportReview label="p.mrpack" plan={mrpackPlan} error={null} name="  "
        onNameChange={() => undefined} onImport={() => undefined} onCancel={() => undefined} />,
    );
    expect((screen.getByRole('button', { name: 'Import as a copy' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('lists what would stop the import and does not offer it', () => {
    const onImport = vi.fn();
    review({
      plan: { ...mrpackPlan, name_taken: false, problems: ['mods/ghost.jar cannot be imported, and the import stops at a file like this.'] },
      onImport,
    });
    expect(screen.getByRole('alert').textContent).toContain('mods/ghost.jar cannot be imported');
    const button = screen.getByRole('button', { name: 'Import' }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(onImport).not.toHaveBeenCalled();
  });

  it('a hard problem keeps Import disabled even when there are soft ones too', () => {
    review({
      plan: { ...mrpackPlan, problems: ['mods/bad.jar cannot be imported'], soft_problems: ['mods/ghost.jar has no way to be fetched'] },
    });
    expect(screen.queryByRole('button', { name: 'Import anyway' })).toBeNull();
    expect((screen.getByRole('button', { name: /^Import/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('soft problems are named and offer Import anyway, which says it is skipping them', () => {
    const onImport = vi.fn();
    review({
      plan: { ...mrpackPlan, name_taken: false, soft_problems: ['mods/ghost.jar has no way to be fetched, so it would be left out.'] },
      onImport,
    });
    expect(screen.getByLabelText('Files that cannot be fetched').textContent).toContain('mods/ghost.jar');
    const button = screen.getByRole('button', { name: 'Import anyway' }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    expect(onImport).toHaveBeenCalledWith(true);
  });

  it('a plain import does not skip anything', () => {
    const onImport = vi.fn();
    review({ onImport });
    fireEvent.click(screen.getByRole('button', { name: 'Import' }));
    expect(onImport).toHaveBeenCalledWith(false);
  });
});

describe('pack import review heading and counts', () => {
  it('the heading follows the name the user typed, not the name in the pack', () => {
    review({ plan: mrpackPlan, name: 'Spiff for Sam' });
    expect(screen.getByRole('heading', { name: 'Import Spiff for Sam?' })).toBeTruthy();
    expect(screen.queryByText('Import Spiff?')).toBeNull();
  });

  it('falls back to the pack name when the field is blank', () => {
    review({ plan: mrpackPlan, name: '   ' });
    expect(screen.getByRole('heading', { name: 'Import Spiff?' })).toBeTruthy();
  });

  it('counts a shader zip as a shader pack, not a mod', () => {
    review({ plan: mrpackPlan, name: mrpackPlan.suggested_name });
    expect(screen.getByText(/^2 mods, 1 shader pack in the pack/)).toBeTruthy();
    expect(screen.queryByText(/^3 mods/)).toBeNull();
  });

  it('counts each kind in plain words, singular and plural, and a non-content file as other', () => {
    const entry = (name: string, content_type: string) => ({
      name, display_name: null, source: 'download' as const, checkable: false, reason: null, action: null, content_type,
    });
    review({
      plan: {
        ...mrpackPlan,
        entries: [
          entry('mods/a.jar', 'mod'),
          entry('resourcepacks/F.zip', 'resourcepack'),
          entry('resourcepacks/G.zip', 'resourcepack'),
          entry('datapacks/d.zip', 'datapack'),
          entry('config/c.json', 'other'),
        ],
      },
      name: 'X',
    });
    expect(screen.getByText(/^1 mod, 2 resource packs, 1 data pack, 1 other file in the pack/)).toBeTruthy();
  });
});

