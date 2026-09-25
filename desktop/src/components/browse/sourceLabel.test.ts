import { describe, expect, it } from 'vitest';
import { sourceLabel } from './sourceLabel';
import { isProviderItemId, providerIdOf } from '../../lib/tauri';

describe('sourceLabel', () => {
  it('names the curated catalog as Agora’s own', () => {
    expect(sourceLabel({ source: 'curated' })).toBe('Agora Registry');
  });

  it('uses the title core reports for any provider, without knowing which exist', () => {
    expect(sourceLabel({ source: 'modrinth', providerTitle: 'Modrinth' })).toBe('Modrinth');
    expect(sourceLabel({ source: 'acme.cf/curseforge', providerTitle: 'CurseForge' })).toBe(
      'CurseForge',
    );
  });

  it('falls back rather than guessing when no title was reported', () => {
    expect(sourceLabel({ source: 'something-new' })).toBe('Third-party');
  });
});

describe('provider item ids', () => {
  it('recognises provider ids and extracts the provider, even when the project id has colons', () => {
    const id = 'provider:agora.example-provider/shelf:mod:123';
    expect(isProviderItemId(id)).toBe(true);
    expect(providerIdOf(id)).toBe('agora.example-provider/shelf');
  });

  it('leaves curated, Modrinth and Technic ids alone', () => {
    for (const id of ['sodium', 'AANobbMI', 'technic:tekkit']) {
      expect(isProviderItemId(id)).toBe(false);
      expect(providerIdOf(id)).toBeNull();
    }
  });
});
