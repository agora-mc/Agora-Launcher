import { previewPackInstanceName } from './tauri';
import type { PromptOptions } from '../components/ui/confirm';

/**
 * Settle the instance name for a pack install that creates a new instance.
 *
 * Resolves `undefined` when the pack's own name is free (install as usual), the
 * name the person chose when it is taken (pre-filled with a free "<name> (2)"),
 * or `null` when they cancelled. A failed lookup resolves `undefined`: core
 * still refuses to overwrite an existing instance, so the install then fails
 * with a clear error instead of silently renaming.
 */
export async function choosePackInstanceName(
  prompt: (options: PromptOptions) => Promise<string | null>,
  packName: string,
): Promise<string | undefined | null> {
  let preview;
  try {
    preview = await previewPackInstanceName(packName);
  } catch {
    return undefined;
  }
  if (!preview.name_taken) return undefined;
  const chosen = await prompt({
    title: 'Name the new instance',
    body: `An instance named "${preview.default_name}" already exists. This will be installed as a separate copy; the existing instance is not changed.`,
    initialValue: preview.suggested_name,
    confirmLabel: 'Install as a copy',
  });
  const name = chosen?.trim();
  return name ? name : null;
}
