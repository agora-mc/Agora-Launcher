import { describe, expect, it } from 'vitest';
import { GUIDE_TOPICS } from './guideContent';

describe('Packs & sharing guide', () => {
  it('has a topic that explains moving a setup to another computer, and search can find it', () => {
    const topic = GUIDE_TOPICS.find((t) => t.id === 'packs-sharing');
    const titles = topic?.basic.sections.map((s) => s.title) ?? [];
    expect(titles).toContain('Send a setup to a friend or another computer');
    const text = JSON.stringify(topic).toLowerCase();
    expect(text).toContain('another computer');
  });

  it('points at the buttons that exist for pack import, and says what an export carries', () => {
    const topic = GUIDE_TOPICS.find((t) => t.id === 'packs-sharing');
    const text = JSON.stringify(topic);
    // The primary control is on My Instances; it was once described as being on the Mods tab.
    expect(text).toContain('Import pack file');
    expect(text).not.toContain('on the Mods tab');
    expect(text).toContain('resource packs, shader packs and datapacks');
  });
});
