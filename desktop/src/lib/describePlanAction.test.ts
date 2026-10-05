import { describe, expect, it } from 'vitest';
import { describePlanAction, type ResolvedInstallPlan } from './installFlow';

const plan = (operation: unknown, add: number, remove: number) =>
  ({
    operation,
    filesToAdd: Array.from({ length: add }, (_, i) => ({ targetFilename: `a${i}.jar` })),
    filesToRemove: Array.from({ length: remove }, (_, i) => ({ filename: `r${i}.jar` })),
    filesToDisable: [],
  }) as unknown as ResolvedInstallPlan;

describe('describePlanAction', () => {
  it('describes a bulk removal as a removal without file download progress', () => {
    const action = describePlanAction(plan({ type: 'batch-remove', operations: [] }, 0, 7));
    expect(action).toEqual({ verb: 'Removing 7 files', done: 'Removed 7 files.', downloadsFiles: false });
  });

  it('counts updates by item and installs by file', () => {
    expect(describePlanAction(plan({ type: 'batch-update', operations: [{}, {}, {}] }, 3, 3)).verb).toBe('Updating 3 items');
    expect(describePlanAction(plan({ type: 'install' }, 1, 0))).toEqual({
      verb: 'Installing 1 file', done: 'Installed 1 file.', downloadsFiles: true,
    });
  });
});
