/**
 * Human-readable name for where a browse item came from.
 *
 * Curated items come from Agora's own registry. Everything else came from a
 * content provider, and core already tells us that provider's name — so the
 * card says "Modrinth", "Technic" or a plugin's own title without this file
 * having to know which providers exist.
 */
export function sourceLabel(item: { source: string; providerTitle?: string | null }): string {
  if (item.source === 'curated') return 'Agora Registry';
  return item.providerTitle || 'Third-party';
}
