import { Component, inject, signal, OnInit, computed } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { DomainService, Domain } from '../../../../../../core/services/domain.service';
import { ProjectService, ServerlessInstance } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';

export interface UnifiedRoute {
  id: string;
  fqdn: string;
  type: 'automatic' | 'custom';
  routingType: 'ingress' | 'reverse_proxy' | 'static_host' | 'custom';
  status: 'active' | 'pending_verification' | 'failed';
  connectedApp?: string;
  branchName?: string;
  port?: number;
  rawObject: any;
}

@Component({
  selector: 'app-networking-list',
  imports: [FormsModule, RouterLink],
  templateUrl: './networking-list.html',
})
export class NetworkingList implements OnInit {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly domainService = inject(DomainService);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly serverlessFunctions = signal<ServerlessInstance[]>([]);
  readonly loadingDbs = signal(false);
  readonly successMsg = signal<string | null>(null);
  readonly errorMsg = signal<string | null>(null);

  readonly customDomains = signal<Domain[]>([]);
  readonly loadingDomains = signal(false);
  readonly expandedDomainCerts = signal<Record<string, boolean>>({});

  readonly activeServerlessPorts = computed(() => {
    return this.serverlessFunctions().filter(fn => fn.status === 'active' && fn.externalPort);
  });

  readonly unifiedRoutes = computed<UnifiedRoute[]>(() => {
    const insts = this.parent.appDetail()?.instances || [];
    const domains = this.customDomains();
    const functions = this.serverlessFunctions();
    const routes: UnifiedRoute[] = [];

    const attachedFqdns = new Set(domains.map(d => d.fqdn));

    insts.forEach(inst => {
      if (inst.assignedDomain && attachedFqdns.has(inst.assignedDomain)) return;
      routes.push({
        id: inst.id,
        fqdn: inst.assignedDomain || '',
        type: 'automatic',
        routingType: 'ingress',
        status: inst.status === 'running' ? 'active' : 'pending_verification',
        connectedApp: this.parent.project()?.name || 'Hermes App',
        branchName: inst.branchName,
        port: inst.internalPort,
        rawObject: inst
      });
    });

    functions.forEach(fn => {
      if (fn.assignedDomain && fn.status === 'active' && !attachedFqdns.has(fn.assignedDomain)) {
        routes.push({
          id: fn.id,
          fqdn: fn.assignedDomain,
          type: 'automatic',
          routingType: 'ingress',
          status: 'active',
          connectedApp: fn.name,
          branchName: 'Serverless Function',
          port: fn.externalPort || undefined,
          rawObject: fn
        });
      }
    });

    const typeLabel: Record<string, string> = {
      app: 'Application', serverless: 'Serverless Function', database: 'Database', custom: 'Custom'
    };
    domains.forEach(d => {
      routes.push({
        id: d.id,
        fqdn: d.fqdn,
        type: 'custom',
        routingType: d.routingType as any,
        status: d.status as any,
        connectedApp: d.targetName || (d.targetType === 'custom' ? 'Nginx custom' : undefined),
        branchName: typeLabel[d.targetType] || undefined,
        port: d.targetType === 'database' ? undefined : (d.routingType === 'reverse_proxy' ? 80 : undefined),
        rawObject: d
      });
    });

    return routes;
  });

  ngOnInit(): void {
    this.loadExposedDbs();
    this.loadDomains();
    this.loadServerlessFunctions();
  }

  loadExposedDbs(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingDbs.set(true);
    this.dbService.listDatabases(projectId, 1, 1000).subscribe({
      next: (res) => {
        this.databases.set((res?.items || []).filter(db => db.isExternal));
        this.loadingDbs.set(false);
      },
      error: () => {
        this.databases.set([]);
        this.loadingDbs.set(false);
      }
    });
  }

  loadServerlessFunctions(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.projectService.listProjectFunctions(projectId, 1, 1000).subscribe({
      next: (res) => {
        this.serverlessFunctions.set(res?.items || []);
      },
      error: () => {
        this.serverlessFunctions.set([]);
      }
    });
  }

  loadDomains(): void {
    this.loadingDomains.set(true);
    const projectId = this.parent.projectId();
    this.domainService.listDomains(1, 1000, projectId || undefined).subscribe({
      next: (res) => {
        this.customDomains.set(res?.items || []);
        this.loadingDomains.set(false);
      },
      error: (err) => {
        console.error('Failed to load domains', err);
        this.loadingDomains.set(false);
      }
    });
  }

  onVerifyDomain(id: string): void {
    this.domainService.verifyAndSyncDomain(id).subscribe({
      next: () => {
        this.toast.success('DNS/SSL verification and sync process started.');
        this.loadDomains();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to verify domain.');
      }
    });
  }

  async onRemoveDomain(id: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Domain',
      message: 'Are you sure you want to delete this custom domain? This will remove Nginx and DNS routes.',
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.domainService.removeDomain(id).subscribe({
      next: () => {
        this.toast.success('Custom domain removed.');
        this.loadDomains();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to remove domain.');
      }
    });
  }

  selectRoute(route: UnifiedRoute): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'networking', route.id]);
  }

  toggleCertDetails(domainId: string): void {
    const current = this.expandedDomainCerts();
    this.expandedDomainCerts.set({
      ...current,
      [domainId]: !current[domainId]
    });
  }

  copyPort(port: number | null | undefined): void {
    if (port === null || port === undefined) return;
    navigator.clipboard.writeText(port.toString()).then(() => {
      this.toast.success('Public port copied to clipboard!');
    });
  }
}
