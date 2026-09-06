import type { CredentialBackend } from '@/lib/tauri';

/**
 * Warns that a credential is stored in a local encrypted file rather than the
 * OS keyring, which MASTER_SPEC 7.5.2 requires Settings to surface.
 *
 * Renders nothing unless the backend positively reports `encrypted-file`, so an
 * unknown or failed lookup shows no warning. That direction matters: a spurious
 * warning tells users their credentials are less protected than they are, and
 * teaches them to ignore the real one.
 *
 * The wording avoids calling the key "machine-bound". It is a random per-profile
 * secret guarded by file permissions -- it protects against a single leaked file,
 * not against anyone who can read the profile directory. Saying otherwise would
 * overstate the protection in exactly the place a user is deciding whether to
 * trust it.
 *
 * It also avoids promising that only the user's own account can read the file.
 * Owner-only permissions are set explicitly on Unix; elsewhere this inherits
 * whatever the data directory grants, and the data directory now follows
 * AGORA_DATA_DIR and portable roots -- which can be a removable drive with no
 * per-user permissions at all. "Anyone who can read that folder" is true
 * everywhere, and is the sentence a user can actually act on.
 */
export function DegradedCredentialNotice({ backend }: { backend: CredentialBackend | undefined }) {
  if (backend !== 'encrypted-file') return null;

  return (
    <div
      data-testid="degraded-credential-storage"
      role="status"
      className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3"
    >
      <p className="text-xs text-amber-700 dark:text-amber-400">
        <strong>Credential store unavailable.</strong> Your sign-in is encrypted in a file
        in Agora's data folder instead. This is less secure than OS keychain storage —
        anyone who can read that folder can read your sign-in.
      </p>
    </div>
  );
}
