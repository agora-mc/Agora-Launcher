/**
 * FOMOD slot owned by the fomod slice: the installer wizard for archives that ask questions.
 * It has no shell slot of its own; the content slice mounts it from the instance's Content tab,
 * and the Library catalog mounts it when an install needs an installer.
 */

export { FomodWizard, type FomodWizardProps } from './FomodWizard';
