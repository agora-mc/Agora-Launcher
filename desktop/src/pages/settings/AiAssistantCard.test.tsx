import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));

import { AiAssistantCard, assistantMessage } from './AiAssistantCard';

describe('AiAssistantCard', () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it('copies a message naming the bundled CLI by full path', async () => {
    invokeMock.mockResolvedValue({ path: 'C:\\Program Files\\Agora\\agora.exe', temporary: false });
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(<AiAssistantCard />);
    expect(await screen.findByTestId('ai-assistant-cli-path')).toHaveTextContent('C:\\Program Files\\Agora\\agora.exe');
    expect(screen.queryByText(/only works while Agora is open/)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Copy message' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(assistantMessage('C:\\Program Files\\Agora\\agora.exe')));
    expect(invokeMock).toHaveBeenCalledWith('bundled_cli_path');
  });

  it('warns that an AppImage path is temporary', async () => {
    invokeMock.mockResolvedValue({ path: '/tmp/.mount_x/usr/bin/agora', temporary: true });
    render(<AiAssistantCard />);
    expect(await screen.findByText(/only works while Agora is open/)).toBeInTheDocument();
  });

  it('points at the release download when this build has no CLI', async () => {
    invokeMock.mockResolvedValue(null);
    render(<AiAssistantCard />);
    expect(await screen.findByRole('link', { name: 'latest release' })).toHaveAttribute(
      'href',
      expect.stringMatching(/\/releases\/latest$/),
    );
    expect(screen.queryByRole('button', { name: 'Copy message' })).not.toBeInTheDocument();
  });
});

describe('assistantMessage', () => {
  it('quotes the path and asks for the skill guide', () => {
    expect(assistantMessage('/usr/bin/agora')).toBe(
      'Agora\'s command-line tool is installed at "/usr/bin/agora". Run it with the argument "skill" '
      + 'and follow the guide it prints. Then help me with: ',
    );
  });
});
