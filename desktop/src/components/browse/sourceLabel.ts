/**
 * Human-readable name for a browse item's source.
 *
 * Was previously inlined as `source === 'curated' ? 'Agora Registry' : 'Modrinth'`
 * in each card, which labelled Technic packs as Modrinth once a third source
 * joined the list.
 */
export function sourceLabel(source: string): string {
  switch (source) {
    case 'curated':
      return 'Agora Registry';
    case 'technic':
      return 'Technic';
    case 'modrinth':
      return 'Modrinth';
    default:
      return 'Third-party';
  }
}
