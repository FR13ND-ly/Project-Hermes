import { Component, inject, signal, OnInit, OnDestroy, effect, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { Details } from '../../details';
import { IncidentService, Incident } from '../../../../../../core/services/incident.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

@Component({
  selector: 'app-project-incidents',
  standalone: true,
  imports: [CommonModule, DatePipe, Pagination],
  templateUrl: './incidents.html',
})
export class Incidents implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly incidentService = inject(IncidentService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);

  readonly incidents = signal<Incident[]>([]);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);
  readonly resolvingId = signal<string | null>(null);
  readonly filter = signal<'all' | 'active' | 'resolved'>('all');

  readonly page = signal(1);
  readonly pageSize = signal(DEFAULT_PAGE_SIZE);
  readonly total = signal(0);

  private wsSubscription: Subscription | null = null;

  readonly filteredIncidents = computed(() => {
    const list = this.incidents();
    const activeFilter = this.filter();
    if (activeFilter === 'active') {
      return list.filter(i => !i.resolvedAt);
    } else if (activeFilter === 'resolved') {
      return list.filter(i => !!i.resolvedAt);
    }
    return list;
  });

  readonly activeCount = computed(() => {
    return this.incidents().filter(i => !i.resolvedAt).length;
  });

  readonly resolvedCount = computed(() => {
    return this.incidents().filter(i => !!i.resolvedAt).length;
  });

  constructor() {
    effect(() => {
      const projId = this.parent.projectId();
      if (projId) {
        this.loadIncidents();
        this.setupWsSubscription();
      }
    });
  }

  ngOnInit(): void {
    this.loadIncidents();
    this.setupWsSubscription();
  }

  ngOnDestroy(): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
  }

  setupWsSubscription(): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
    this.wsSubscription = this.wsService.onEvent<any>('incident_created').subscribe(payload => {
      const projId = this.parent.projectId();
      if (projId && payload.project_id === projId) {
        this.loadIncidents();
      }
    });
  }

  loadIncidents(): void {
    const projId = this.parent.projectId();
    if (!projId) return;

    this.loading.set(true);
    this.error.set(null);

    this.incidentService.listProjectIncidents(projId, this.page(), this.pageSize()).subscribe({
      next: (res) => {
        this.incidents.set(res?.items || []);
        this.total.set(res?.total || 0);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea incidentelor.');
        this.loading.set(false);
      }
    });
  }

  onPageChange(page: number): void {
    this.page.set(page);
    this.loadIncidents();
  }

  async onResolveIncident(incident: Incident): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Rezolvare Incident',
      message: `Sigur doriți să marcați incidentul din instanța "${incident.instanceName}" ca rezolvat manual?`,
      confirmText: 'Rezolvă',
      cancelText: 'Anulează',
      isDanger: false
    });
    if (!confirmed) return;

    this.resolvingId.set(incident.id);
    this.incidentService.resolveIncident(incident.id).subscribe({
      next: () => {
        this.toast.success('Incidentul a fost marcat ca rezolvat.');
        this.resolvingId.set(null);
        this.loadIncidents();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la rezolvarea incidentului.');
        this.resolvingId.set(null);
      }
    });
  }

  getDuration(startStr: string, endStr: string | null): string {
    if (!endStr) return '';
    const start = new Date(startStr);
    const end = new Date(endStr);
    const diffMs = end.getTime() - start.getTime();
    if (diffMs <= 0) return '0s';
    
    const diffSecs = Math.floor(diffMs / 1000);
    const secs = diffSecs % 60;
    const mins = Math.floor(diffSecs / 60) % 60;
    const hours = Math.floor(diffSecs / 3600);

    const parts = [];
    if (hours > 0) parts.push(`${hours}h`);
    if (mins > 0) parts.push(`${mins}m`);
    if (secs > 0 || parts.length === 0) parts.push(`${secs}s`);
    return parts.join(' ');
  }

  getIncidentBadgeClass(type: string): string {
    switch (type) {
      case 'TIMEOUT_OR_DOWN':
        return 'bg-red-950/40 border border-red-500/30 text-red-400';
      case 'UNHEALTHY_HTTP_CODE':
        return 'bg-amber-950/40 border border-amber-500/30 text-amber-400';
      default:
        return 'bg-zinc-900 border border-zinc-800 text-zinc-400';
    }
  }
}
