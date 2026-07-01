import { Component, input, output, computed } from '@angular/core';

/**
 * Reusable offset-pagination control. Bind page/pageSize/total from a backend
 * Paginated<T> response and react to (pageChange) by reloading that page.
 */
@Component({
  selector: 'app-pagination',
  standalone: true,
  template: `
    @if (totalPages() > 1) {
      <div class="flex items-center justify-between gap-3 pt-3 text-[13px] font-mono text-zinc-500 select-none">
        <span>{{ rangeStart() }}–{{ rangeEnd() }} of {{ total() }}</span>
        <div class="flex items-center gap-1.5">
          <button
            type="button"
            (click)="go(page() - 1)"
            [disabled]="page() <= 1"
            class="px-2.5 py-1 rounded border border-zinc-800 text-zinc-300 hover:bg-zinc-900 disabled:opacity-40 disabled:cursor-not-allowed transition-colors cursor-pointer"
          >‹ Previous</button>
          <span class="px-2 text-zinc-400">Page {{ page() }} / {{ totalPages() }}</span>
          <button
            type="button"
            (click)="go(page() + 1)"
            [disabled]="page() >= totalPages()"
            class="px-2.5 py-1 rounded border border-zinc-800 text-zinc-300 hover:bg-zinc-900 disabled:opacity-40 disabled:cursor-not-allowed transition-colors cursor-pointer"
          >Next ›</button>
        </div>
      </div>
    }
  `,
})
export class Pagination {
  readonly page = input.required<number>();
  readonly pageSize = input.required<number>();
  readonly total = input.required<number>();
  readonly pageChange = output<number>();

  readonly totalPages = computed(() => Math.max(1, Math.ceil(this.total() / Math.max(1, this.pageSize()))));
  readonly rangeStart = computed(() => (this.total() === 0 ? 0 : (this.page() - 1) * this.pageSize() + 1));
  readonly rangeEnd = computed(() => Math.min(this.page() * this.pageSize(), this.total()));

  go(p: number): void {
    if (p < 1 || p > this.totalPages() || p === this.page()) return;
    this.pageChange.emit(p);
  }
}
