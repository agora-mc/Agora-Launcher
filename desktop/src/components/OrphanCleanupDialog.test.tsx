import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { OrphanCleanupDialog } from './OrphanCleanupDialog';
import type { OrphanedDependency } from '@/lib/tauri';

function orphan(filename: string, modJarId: string | null = null): OrphanedDependency {
  return { filename, mod_jar_id: modJarId, content_type: 'mod' };
}

describe('OrphanCleanupDialog', () => {
  it('offers every orphan pre-selected but removes nothing without confirmation', () => {
    const onConfirm = vi.fn();
    render(
      <OrphanCleanupDialog
        orphans={[orphan('corelib.jar', 'corelib'), orphan('base.jar')]}
        onConfirm={onConfirm}
        onClose={() => {}}
      />,
    );
    expect(screen.getByRole('button', { name: 'Remove 2' })).toBeInTheDocument();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('drops unchecked entries from what it confirms', () => {
    const onConfirm = vi.fn();
    render(
      <OrphanCleanupDialog
        orphans={[orphan('corelib.jar'), orphan('base.jar')]}
        onConfirm={onConfirm}
        onClose={() => {}}
      />,
    );
    fireEvent.click(screen.getAllByRole('checkbox')[0]);
    fireEvent.click(screen.getByRole('button', { name: 'Remove 1' }));
    expect(onConfirm).toHaveBeenCalledWith(['base.jar']);
  });

  it('cannot confirm an empty selection', () => {
    const onConfirm = vi.fn();
    render(
      <OrphanCleanupDialog orphans={[orphan('corelib.jar')]} onConfirm={onConfirm} onClose={() => {}} />,
    );
    fireEvent.click(screen.getByRole('checkbox'));
    const button = screen.getByRole('button', { name: 'Remove 0' });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('lets the user keep everything', () => {
    const onClose = vi.fn();
    const onConfirm = vi.fn();
    render(
      <OrphanCleanupDialog orphans={[orphan('corelib.jar')]} onConfirm={onConfirm} onClose={onClose} />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Keep them' }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
