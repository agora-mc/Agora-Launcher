/**
 * Carryable ground items (prototype `ITEMS`), verbatim.
 */

export interface ItemDef {
  glyph: string;
  name: string;
}

export const ITEMS: Record<string, ItemDef> = {
  acorn: { glyph: '🌰', name: 'Acorn' }, pinecone: { glyph: '🌲', name: 'Pinecone' }, fish: { glyph: '🐟', name: 'Fish' },
  berry: { glyph: '🫐', name: 'Berries' }, honey: { glyph: '🍯', name: 'Honey' }, feather: { glyph: '🪶', name: 'Feather' },
  truffle: { glyph: '🍄', name: 'Truffle' }, flower: { glyph: '🌸', name: 'Flower' }, firefly: { glyph: '✨', name: 'Firefly' },
  snowball: { glyph: '❄️', name: 'Snowball' }, coin: { glyph: '🪙', name: 'Coin' }, key: { glyph: '🗝️', name: 'Key' },
  shell: { glyph: '🐚', name: 'Shell' }, water: { glyph: '💧', name: 'Water' },
};
