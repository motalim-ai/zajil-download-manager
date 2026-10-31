// The list read while the main window is hidden or minimized (the owner's
// choice, after measuring the list's cost): the sounds, the notifications
// and the tray's tooltip follow it, but the table is not drawn for nobody;
// the last one read is drawn once the window shows.

export class Unseen<T> {
  private kept: T | null = null;

  /** What the table draws now: `next`, or nothing while `hidden` (it is
   * kept for when the window shows). */
  take(next: T, hidden: boolean): T | null {
    if (hidden) {
      this.kept = next;
      return null;
    }
    this.kept = null;
    return next;
  }

  /** Whether a list read while hidden waits to be drawn. */
  get waiting(): boolean {
    return this.kept !== null;
  }
}
