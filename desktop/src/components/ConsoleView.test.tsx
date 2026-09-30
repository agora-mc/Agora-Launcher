import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => undefined) }));

import { ConsoleView } from './ConsoleView';

const buffer = [
  '<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1790743960312" level="INFO" thread="Render thread">',
  '  <log4j:Message><![CDATA[Backend library: LWJGL]]></log4j:Message>',
  '</log4j:Event>',
  '<log4j:Event logger="Sodium" timestamp="1790743960400" level="WARN" thread="main"><log4j:Message><![CDATA[Careful]]></log4j:Message></log4j:Event>',
].map((line) => ({ line, stream: 'stdout', instance_id: 'i1' }));
buffer.push({ line: 'Exception in thread "main" java.lang.Error: boom', stream: 'stderr', instance_id: 'i1' });

describe('ConsoleView', () => {
  it('shows readable entries by default and raw captured text on toggle', () => {
    render(<ConsoleView instanceId="i1" logBuffer={buffer} />);
    expect(screen.getAllByTestId('console-entry')).toHaveLength(3);
    expect(screen.getByText('Backend library: LWJGL')).toBeTruthy();
    expect(screen.queryByText(/<log4j:Event/)).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: 'Raw' }));
    const raw = screen.getByTestId('console-raw');
    expect(raw.textContent).toContain('<log4j:Message><![CDATA[Backend library: LWJGL]]></log4j:Message>');
    expect(screen.queryAllByTestId('console-entry')).toHaveLength(0);

    fireEvent.click(screen.getByRole('button', { name: 'Readable' }));
    expect(screen.getAllByTestId('console-entry')).toHaveLength(3);
  });

  it('filters by level and copies raw text', () => {
    const writeText = vi.fn();
    Object.assign(navigator, { clipboard: { writeText } });
    render(<ConsoleView instanceId="i1" logBuffer={buffer} />);
    fireEvent.click(screen.getByRole('button', { name: 'Errors' }));
    expect(screen.getAllByTestId('console-entry')).toHaveLength(1);
    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(writeText).toHaveBeenCalledWith('Exception in thread "main" java.lang.Error: boom');
  });
});
