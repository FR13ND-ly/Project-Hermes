import { Component, inject, input, output } from '@angular/core';
import { ConfirmService } from '../../../core/services/confirm.service';

/**
 * Reusable "Danger zone" card for destructive actions. The delete button lives ONLY
 * inside this warning card (in a resource's Settings), and deletion always requires the
 * user to TYPE the resource name in a confirm dialog (ConfirmService `matchText`).
 *
 * Usage:
 *   <app-danger-zone
 *     resourceKind="database" [resourceName]="db().name"
 *     confirmLabel="Delete database" [busy]="deleting()"
 *     (confirmed)="onDelete()" />
 */
@Component({
  selector: 'app-danger-zone',
  template: `
    <div class="rounded-lg border border-red-900/40 bg-red-950/10 overflow-hidden">
      <div class="px-5 py-4 border-b border-red-900/30">
        <h3 class="text-sm font-semibold text-red-300">{{ heading() }}</h3>
        <p class="text-[13px] text-zinc-400 mt-0.5">{{ description() }}</p>
      </div>
      <div class="px-5 py-4 flex items-center justify-between gap-4 flex-wrap">
        <p class="text-[13px] text-zinc-500">
          This permanently deletes <span class="font-semibold text-zinc-300">{{ resourceName() }}</span> and cannot be undone.
        </p>
        <button (click)="trigger()" [disabled]="busy() || !resourceName()" class="btn btn-danger shrink-0">
          {{ busy() ? 'Deleting…' : confirmLabel() }}
        </button>
      </div>
    </div>
  `,
})
export class DangerZone {
  private readonly confirm = inject(ConfirmService);

  /** Card heading (defaults to the confirm label). */
  readonly heading = input('Danger zone');
  /** Explanatory line under the heading. */
  readonly description = input('');
  /** Button text + confirm-dialog title. */
  readonly confirmLabel = input('Delete');
  /** The exact name the user must type to confirm. */
  readonly resourceName = input.required<string>();
  /** Human noun used in the confirm message (e.g. "application", "database"). */
  readonly resourceKind = input('resource');
  readonly busy = input(false);

  readonly confirmed = output<void>();

  async trigger(): Promise<void> {
    const name = this.resourceName();
    if (!name) return;
    const ok = await this.confirm.ask({
      title: this.confirmLabel(),
      message: `This permanently deletes the ${this.resourceKind()} "${name}" and all of its data. This action cannot be undone.`,
      matchText: name,
      confirmText: this.confirmLabel(),
      cancelText: 'Cancel',
      isDanger: true,
    });
    if (ok) this.confirmed.emit();
  }
}
