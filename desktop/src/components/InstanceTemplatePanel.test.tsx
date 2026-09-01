import { describe, expect, it, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { InstanceTemplatePanel } from './InstanceTemplatePanel';
import type { CapturableFile, InstanceTemplate } from '@/lib/tauri';

const tauriMocks = vi.hoisted(() => ({
  listCapturableTemplateFiles: vi.fn(),
  listInstanceTemplates: vi.fn(),
  createInstanceTemplate: vi.fn(),
  applyInstanceTemplate: vi.fn(),
}));

vi.mock('@/lib/tauri', () => ({
  listCapturableTemplateFiles: tauriMocks.listCapturableTemplateFiles,
  listInstanceTemplates: tauriMocks.listInstanceTemplates,
  createInstanceTemplate: tauriMocks.createInstanceTemplate,
  applyInstanceTemplate: tauriMocks.applyInstanceTemplate,
  formatError: (error: unknown) => String(error),
}));

function file(relative_path: string, extra: Partial<CapturableFile> = {}): CapturableFile {
  return {
    relative_path,
    size: 128,
    category: relative_path.includes('/') ? relative_path.split('/')[0] : 'options',
    too_large: false,
    ...extra,
  };
}

function template(id: string, name: string, files: number): InstanceTemplate {
  return {
    template_version: 1,
    id,
    name,
    description: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    jvm: {},
    files: Array.from({ length: files }, (_, index) => ({
      relative_path: `config/f${index}.json`,
      sha256: '0'.repeat(64),
      size: 1,
    })),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  tauriMocks.listCapturableTemplateFiles.mockResolvedValue([
    file('options.txt'),
    file('config/a.json'),
    file('config/huge.bin', { too_large: true, size: 20_000_000 }),
  ]);
  tauriMocks.listInstanceTemplates.mockResolvedValue([]);
  tauriMocks.createInstanceTemplate.mockResolvedValue(template('tpl-1', 'Saved', 1));
  tauriMocks.applyInstanceTemplate.mockResolvedValue(2);
});

describe('InstanceTemplatePanel', () => {
  it('starts with nothing selected so a template is an explicit choice', async () => {
    render(<InstanceTemplatePanel instanceId="alpha" />);
    await screen.findByText('options.txt');
    expect(screen.getByText('0 files selected')).toBeInTheDocument();
    expect(
      (screen.getByText('config/a.json').previousElementSibling as HTMLInputElement).checked,
    ).toBe(false);
  });

  it('sends only the checked, capturable paths', async () => {
    render(<InstanceTemplatePanel instanceId="alpha" />);
    const optionsRow = await screen.findByText('options.txt');
    fireEvent.click(optionsRow.previousElementSibling as HTMLElement);

    fireEvent.click(screen.getByRole('button', { name: 'Save template' }));
    await waitFor(() => expect(tauriMocks.createInstanceTemplate).toHaveBeenCalledTimes(1));
    expect(tauriMocks.createInstanceTemplate.mock.calls[0][0]).toMatchObject({
      selectedPaths: ['options.txt'],
      sourceInstanceId: 'alpha',
    });
  });

  it('never offers an oversized file for selection', async () => {
    render(<InstanceTemplatePanel instanceId="alpha" />);
    const huge = await screen.findByText('config/huge.bin');
    expect((huge.previousElementSibling as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByText('too large')).toBeInTheDocument();
  });

  it('captures a JVM-only template with no source instance when nothing is picked', async () => {
    render(<InstanceTemplatePanel instanceId="alpha" />);
    await screen.findByText('options.txt');
    fireEvent.click(screen.getByRole('button', { name: 'Save template' }));
    await waitFor(() => expect(tauriMocks.createInstanceTemplate).toHaveBeenCalledTimes(1));
    // No files selected means no source instance is needed, and core rejects a
    // source-less capture that names paths — so this must send null.
    expect(tauriMocks.createInstanceTemplate.mock.calls[0][0].sourceInstanceId).toBeNull();
  });

  it('reports how many files an apply actually wrote', async () => {
    tauriMocks.listInstanceTemplates.mockResolvedValue([template('tpl-9', 'Perf', 2)]);
    const onApplied = vi.fn();
    render(<InstanceTemplatePanel instanceId="alpha" onApplied={onApplied} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(
      screen.getByText('Applied 2 files from "Perf".'),
    ).toBeInTheDocument());
    expect(onApplied).toHaveBeenCalledTimes(1);
  });

  it('surfaces a capture failure instead of silently doing nothing', async () => {
    tauriMocks.createInstanceTemplate.mockRejectedValue(new Error('disk full'));
    render(<InstanceTemplatePanel instanceId="alpha" />);
    await screen.findByText('options.txt');
    fireEvent.click(screen.getByRole('button', { name: 'Save template' }));
    await waitFor(() => expect(screen.getByText(/disk full/)).toBeInTheDocument());
  });
});
