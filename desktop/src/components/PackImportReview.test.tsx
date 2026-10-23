import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PackImportReview } from './PackImportReview';
import type { PackImportPlan } from '../lib/tauri';

const plan: PackImportPlan = {
  pack_name: 'Friend Pack',
  instance: { id: null, name: 'Friend Pack', minecraft_version: '1.21', loader: 'fabric', loader_version: '0.16' },
  entries: [
    { name: 'on.jar', display_name: null, source: 'modrinth', checkable: true, reason: null, action: null },
    { name: 'off.jar', display_name: null, source: 'modrinth', checkable: true, reason: null, action: null, arrives_disabled: true },
  ],
  file_sha256: 'x',
  file_size: 1,
};

describe('pack import review', () => {
  it('shows which mods arrive disabled, and only those', () => {
    render(<PackImportReview label="p.json" plan={plan} error={null} onImport={() => undefined} onCancel={() => undefined} />);
    expect(screen.getAllByText(/arrives disabled/)).toHaveLength(1);
  });
});
