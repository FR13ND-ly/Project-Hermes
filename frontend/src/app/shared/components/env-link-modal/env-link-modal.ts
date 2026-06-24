import { Component, computed, input, model, output, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ProjectEnvResponse } from '../../../core/services/project.service';

/**
 * Reusable modal for connecting an app to shared project-pool env vars.
 *
 * Presentational only: the parent owns the list (each var carries a `linked`
 * flag) and reacts to `toggle`. Used both at app-creation time (local selection)
 * and from app-detail (live link/unlink against the backend).
 */
@Component({
  selector: 'app-env-link-modal',
  imports: [FormsModule],
  template: `
    @if (open()) {
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4 animate-fade-in"
        (click)="close()"
      >
        <div
          class="w-full max-w-lg bg-[#0a0a0a] border border-zinc-800 rounded-xl shadow-2xl overflow-hidden font-mono"
          (click)="$event.stopPropagation()"
        >
          <div class="flex items-center justify-between px-5 py-4 border-b border-zinc-900">
            <div>
              <h3 class="text-xs font-bold text-zinc-200 uppercase tracking-wider">{{ title() }}</h3>
              <p class="text-[9px] text-zinc-550 mt-0.5">Live reference — changes in the pool propagate automatically.</p>
            </div>
            <button
              (click)="close()"
              class="p-1.5 rounded hover:bg-zinc-900 text-zinc-500 hover:text-white transition-colors cursor-pointer"
              title="Close"
            >
              <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2" stroke="currentColor" class="w-4 h-4">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6 18 18 6M6 6l12 12" />
              </svg>
            </button>
          </div>

          <div class="px-5 py-3 border-b border-zinc-900">
            <input
              type="text"
              placeholder="Search by name..."
              [ngModel]="search()"
              (ngModelChange)="search.set($event)"
              class="block w-full px-3 py-2 text-xs bg-black border border-zinc-800 rounded text-white focus:outline-none focus:border-zinc-500 transition-colors"
            />
          </div>

          <div class="max-h-80 overflow-y-auto px-5 py-2">
            @if (filtered().length === 0) {
              <p class="text-[10px] text-zinc-600 py-6 text-center">
                @if (vars().length === 0) {
                  {{ empty() }}
                } @else {
                  No results for "{{ search() }}".
                }
              </p>
            } @else {
              <div class="divide-y divide-zinc-900">
                @for (env of filtered(); track env.id) {
                  <div class="flex items-center justify-between gap-3 py-2.5">
                    <div class="flex items-center gap-2 min-w-0">
                      <span class="font-mono text-xs text-white truncate">{{ env.key }}</span>
                      @if (env.source !== 'manual') {
                        <span class="text-[8px] uppercase font-bold px-1.5 py-0.5 rounded bg-emerald-950/40 text-emerald-400 border border-emerald-900/40 shrink-0">{{ env.source }}</span>
                      }
                      @if (env.isSecret) {
                        <span class="text-[8px] uppercase text-zinc-600 shrink-0">secret</span>
                      }
                    </div>
                    <button
                      (click)="toggle.emit(env)"
                      [disabled]="busyId() === env.id"
                      [class]="env.linked
                        ? 'px-2.5 py-1 rounded text-[10px] font-semibold border border-emerald-900/50 bg-emerald-950/30 text-emerald-400 hover:bg-emerald-950/50 cursor-pointer disabled:opacity-50 shrink-0'
                        : 'px-2.5 py-1 rounded text-[10px] font-semibold border border-zinc-800 bg-zinc-900 text-zinc-300 hover:bg-zinc-800 cursor-pointer disabled:opacity-50 shrink-0'"
                    >
                      {{ env.linked ? 'Linked ✓' : 'Link' }}
                    </button>
                  </div>
                }
              </div>
            }
          </div>

          <div class="flex items-center justify-end px-5 py-3 border-t border-zinc-900">
            <button
              (click)="close()"
              class="px-3.5 py-1.5 rounded bg-white hover:bg-zinc-200 text-black text-xs font-semibold shadow transition-all cursor-pointer"
            >
              Done
            </button>
          </div>
        </div>
      </div>
    }
  `,
})
export class EnvLinkModal {
  /** Two-way visibility — set false to close. */
  readonly open = model(false);
  readonly title = input('Link project variables');
  readonly vars = input<ProjectEnvResponse[]>([]);
  /** Id of the var currently being toggled (disables its button). */
  readonly busyId = input<string | null>(null);
  readonly empty = input("No project-level env vars. Add them from the project's “Environments” tab.");

  readonly toggle = output<ProjectEnvResponse>();

  readonly search = signal('');

  readonly filtered = computed(() => {
    const q = this.search().trim().toLowerCase();
    const list = this.vars();
    if (!q) return list;
    return list.filter(v => v.key.toLowerCase().includes(q));
  });

  close(): void {
    this.open.set(false);
    this.search.set('');
  }
}
