import { Component, input } from '@angular/core';

/**
 * Consistent empty-state placeholder for "no resources yet" across all list pages.
 * Projects an optional icon (`[icon]` slot) and an action button (default slot).
 *
 * Usage:
 *   <app-empty-state title="No databases" description="Add a Postgres, Redis or Mongo instance.">
 *     <svg icon>…</svg>
 *     <button class="btn btn-primary">Provision database</button>
 *   </app-empty-state>
 */
@Component({
  selector: 'app-empty-state',
  template: `
    <div class="card py-14 px-8 flex flex-col items-center justify-center text-center max-w-md mx-auto">
      <div class="w-12 h-12 rounded-xl bg-indigo-500/10 border border-indigo-500/20 flex items-center justify-center text-indigo-400 mb-4">
        <ng-content select="[icon]" />
      </div>
      <h3 class="text-sm font-semibold text-zinc-100 mb-1.5">{{ title() }}</h3>
      <p class="text-[13px] text-zinc-500 mb-5 max-w-xs leading-relaxed">{{ description() }}</p>
      <ng-content />
    </div>
  `,
})
export class EmptyState {
  readonly title = input('Nothing here yet');
  readonly description = input('');
}
