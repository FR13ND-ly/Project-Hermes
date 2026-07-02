import { Component, inject, signal, OnInit, OnDestroy } from '@angular/core';

import { RouterLink } from '@angular/router';
import { Subscription } from 'rxjs';
import { ProjectService, BuildQueueItem } from '../../../core/services/project.service';
import { WebSocketService } from '../../../core/services/websocket.service';

/**
 * Persistent floating indicator (bottom-right) shown only while there is active
 * work: app builds, database provisioning, or serverless builds. WebSocket-driven
 * (build / database / serverless events trigger a fresh snapshot) — no polling.
 */
@Component({
  selector: 'app-build-indicator',
  imports: [RouterLink],
  template: `
    @if (items().length > 0) {
      <div class="fixed bottom-6 right-6 z-[9990] font-sans animate-fade-in">
        @if (!expanded()) {
          <button
            (click)="expanded.set(true)"
            class="flex items-center gap-2 pl-2.5 pr-3 py-2 rounded-full bg-zinc-900 border border-zinc-800 shadow-2xl hover:border-zinc-600 transition-colors cursor-pointer"
            [title]="items().length + ' active processes'"
          >
            <svg class="animate-spin h-4 w-4 text-emerald-400" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
            </svg>
            <span class="text-[13px] font-bold text-zinc-50 tabular-nums">{{ items().length }}</span>
            <span class="text-xs text-zinc-500 uppercase tracking-wider font-mono">build</span>
          </button>
        } @else {
          <div class="w-72 bg-zinc-900 border border-zinc-800 rounded-lg shadow-2xl overflow-hidden">
            <div class="flex items-center justify-between px-3.5 py-2.5 border-b border-zinc-900">
              <div class="flex items-center gap-2">
                <svg class="animate-spin h-3.5 w-3.5 text-emerald-400" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
                </svg>
                <span class="text-sm font-semibold text-zinc-100 font-mono">Active processes</span>
              </div>
              <button (click)="expanded.set(false)" class="p-1 rounded hover:bg-zinc-900 text-zinc-500 hover:text-zinc-50 cursor-pointer" title="Collapse">
                <svg xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24" stroke-width="2.5" stroke="currentColor" class="w-3.5 h-3.5"><path stroke-linecap="round" stroke-linejoin="round" d="M19.5 12h-15" /></svg>
              </button>
            </div>

            <div class="max-h-72 overflow-y-auto divide-y divide-zinc-900">
              @for (item of items().slice(0, 6); track item.id) {
                <a [routerLink]="link(item)" [queryParams]="queryParams(item)" (click)="expanded.set(false)"
                   class="px-3.5 py-2.5 flex items-center justify-between gap-2 hover:bg-zinc-900/60 transition-colors cursor-pointer">
                  <div class="min-w-0">
                    <div class="flex items-center gap-1.5">
                      <span class="px-1 py-0.2 rounded text-[7px] font-bold uppercase tracking-wider shrink-0"
                            [class]="kindClass(item.kind)">{{ kindLabel(item.kind) }}</span>
                      <span class="text-[13px] text-zinc-200 font-semibold font-mono truncate">{{ item.name }}</span>
                    </div>
                    <div class="text-[11px] text-zinc-600 truncate">{{ item.projectName }}{{ item.detail ? ' · ' + item.detail : '' }}</div>
                  </div>
                  <div class="flex items-center gap-2 shrink-0">
                    @if (item.status === 'queued') {
                      <span class="w-1.5 h-1.5 rounded-full bg-amber-500" title="queued"></span>
                    } @else if (item.status === 'deploying') {
                      <span class="w-1.5 h-1.5 rounded-full bg-sky-400 animate-pulse" title="deploying"></span>
                    } @else {
                      <span class="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" title="running"></span>
                    }
                    <span class="text-xs text-zinc-400 font-mono tabular-nums">{{ elapsed(item.createdAt) }}</span>
                  </div>
                </a>
              }
              @if (items().length > 6) {
                <div class="px-3.5 py-1.5 text-[11px] text-zinc-600 text-center">+{{ items().length - 6 }} more</div>
              }
            </div>
          </div>
        }
      </div>
    }
  `,
})
export class BuildIndicator implements OnInit, OnDestroy {
  private readonly projectService = inject(ProjectService);
  private readonly wsService = inject(WebSocketService);

  readonly items = signal<BuildQueueItem[]>([]);
  readonly expanded = signal(false);
  readonly now = signal(Date.now());

  private sub = new Subscription();
  private tickTimer: any = null;

  ngOnInit(): void {
    this.load();
    // Refresh on any build / database / serverless lifecycle event.
    // 'instance_status_changed' is the signal the app page uses to know an app is
    // ready — without it the indicator kept showing "deploying" after the deploy
    // finished (the instance flips to running but no build event fires).
    for (const evt of ['build_status_changed', 'instance_status_changed', 'database_status_changed', 'serverless_function_updated']) {
      this.sub.add(this.wsService.onEvent<any>(evt).subscribe(() => this.load()));
    }
    this.sub.add(this.wsService.onResync().subscribe(() => this.load()));
    this.tickTimer = setInterval(() => this.now.set(Date.now()), 1000);
  }

  ngOnDestroy(): void {
    this.sub.unsubscribe();
    if (this.tickTimer) clearInterval(this.tickTimer);
  }

  load(): void {
    this.projectService.listBuildQueue().subscribe({
      next: (res) => {
        this.items.set(res || []);
        if ((res || []).length === 0) this.expanded.set(false);
      },
      error: () => { /* transient — keep last view */ }
    });
  }

  link(item: BuildQueueItem): any[] {
    switch (item.kind) {
      case 'database': return ['/projects', item.projectId, 'databases', item.resourceId];
      case 'serverless': return ['/projects', item.projectId, 'serverless'];
      default: return ['/projects', item.projectId, 'apps', item.resourceId];
    }
  }

  queryParams(item: BuildQueueItem): any {
    if (item.kind === 'serverless') {
      return { functionId: item.resourceId, tab: 'builds' };
    }
    return null;
  }

  kindLabel(kind: string): string {
    return kind === 'database' ? 'DB' : (kind === 'serverless' ? 'FN' : 'APP');
  }

  kindClass(kind: string): string {
    return kind === 'database'
      ? 'bg-amber-950/40 text-amber-400 border border-amber-900/40'
      : (kind === 'serverless'
        ? 'bg-cyan-950/40 text-cyan-400 border border-cyan-900/40'
        : 'bg-indigo-950/40 text-indigo-400 border border-indigo-900/40');
  }

  elapsed(createdAt: string): string {
    const total = Math.max(0, Math.floor((this.now() - new Date(createdAt).getTime()) / 1000));
    const m = Math.floor(total / 60);
    const s = total % 60;
    return m > 0 ? `${m}m ${s}s` : `${s}s`;
  }
}
