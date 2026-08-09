// Negative fixture: a barrel re-exporting app authority must itself be rejected,
// so consumers can never smuggle app deps through it.
export * from '@/lib/tauri';
