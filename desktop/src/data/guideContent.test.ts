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
});
