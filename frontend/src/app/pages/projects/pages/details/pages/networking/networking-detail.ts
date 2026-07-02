import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { ActivatedRoute, Router, RouterLink, RouterOutlet, RouterLinkActive } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService } from '../../../../../../core/services/database.service';
import { DomainService, Domain, DomainRoutingType } from '../../../../../../core/services/domain.service';
import { ProjectService, ServerlessInstance } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { UnifiedRoute } from './networking-list';

@Component({
  selector: 'app-networking-detail',
  imports: [RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './networking-detail.html',
})
export class NetworkingDetail implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly domainService = inject(DomainService);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  private readonly route = inject(ActivatedRoute);
  readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly selectedRoute = signal<UnifiedRoute | null>(null);
  readonly routeId = signal<string | null>(null);
  readonly loading = signal(false);

  // Deep Routing panel settings states
  readonly editRoutingType = signal<DomainRoutingType>('reverse_proxy');
  readonly editClientMaxBodySize = signal<number>(50);
  readonly editIsSsl = signal<boolean>(true);
  readonly editNginxTargetHost = signal('');
  readonly editNginxRootPath = signal('');
  readonly editNginxConfigContent = signal('');
  readonly updatingSettings = signal(false);

  // Diagnostics check state
  readonly dnsVerified = signal(true);
  readonly certRequested = signal(true);
  readonly tlsSecured = signal(true);

  // Access Logs state
  readonly liveLogs = signal<string[]>([]);
  readonly logsSupported = signal(true);
  private logTimer: any = null;

  constructor() {
    this.route.paramMap.subscribe(params => {
      this.routeId.set(params.get('routeId'));
    });

    effect(() => {
      const projectId = this.parent.projectId();
      const rId = this.routeId();
      if (projectId && rId) {
        this.loadRouteDetail(rId);
      }
    });
  }

  ngOnInit(): void {}

  ngOnDestroy(): void {
    this.stopLogSimulation();
  }

  loadRouteDetail(routeId: string): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loading.set(true);

    // Load custom domains and check
    this.domainService.listDomains(1, 1000, projectId).subscribe({
      next: (res) => {
        const domains = res?.items || [];
        const foundDomain = domains.find(d => d.id === routeId);

        if (foundDomain) {
          const typeLabel: Record<string, string> = {
            app: 'Application', serverless: 'Serverless Function', database: 'Database', custom: 'Custom'
          };
          const r: UnifiedRoute = {
            id: foundDomain.id,
            fqdn: foundDomain.fqdn,
            type: 'custom',
            routingType: foundDomain.routingType as any,
            status: foundDomain.status as any,
            connectedApp: foundDomain.targetName || (foundDomain.targetType === 'custom' ? 'Nginx custom' : undefined),
            branchName: typeLabel[foundDomain.targetType] || undefined,
            port: foundDomain.targetType === 'database' ? undefined : (foundDomain.routingType === 'reverse_proxy' ? 80 : undefined),
            rawObject: foundDomain
          };

          this.selectedRoute.set(r);
          this.editRoutingType.set(foundDomain.routingType);
          this.editClientMaxBodySize.set(foundDomain.clientMaxBodySize);
          this.editIsSsl.set(foundDomain.isSsl);
          this.editNginxTargetHost.set(foundDomain.nginxTargetHost || '');
          this.editNginxRootPath.set(foundDomain.nginxRootPath || '');
          this.editNginxConfigContent.set(foundDomain.nginxConfigContent || '');

          this.startLogStream(r);
          this.loading.set(false);
          return;
        }

        // Check app instances
        const insts = this.parent.appDetail()?.instances || [];
        const foundInst = insts.find(i => i.id === routeId);
        if (foundInst) {
          const r: UnifiedRoute = {
            id: foundInst.id,
            fqdn: foundInst.assignedDomain || '',
            type: 'automatic',
            routingType: 'ingress',
            status: foundInst.status === 'running' ? 'active' : 'pending_verification',
            connectedApp: this.parent.project()?.name || 'Hermes App',
            branchName: foundInst.branchName,
            port: foundInst.internalPort,
            rawObject: foundInst
          };
          this.selectedRoute.set(r);
          this.startLogStream(r);
          this.loading.set(false);
          return;
        }

        // Check serverless
        this.projectService.listProjectFunctions(projectId, 1, 1000).subscribe({
          next: (fRes) => {
            const functions = fRes?.items || [];
            const foundFn = functions.find(fn => fn.id === routeId);
            if (foundFn) {
              const r: UnifiedRoute = {
                id: foundFn.id,
                fqdn: foundFn.assignedDomain || '',
                type: 'automatic',
                routingType: 'ingress',
                status: 'active',
                connectedApp: foundFn.name,
                branchName: 'Serverless Function',
                port: foundFn.externalPort || undefined,
                rawObject: foundFn
              };
              this.selectedRoute.set(r);
              this.startLogStream(r);
            } else {
              this.toast.error('Access route not found.');
              this.backToList();
            }
            this.loading.set(false);
          },
          error: () => {
            this.loading.set(false);
            this.toast.error('Failed to resolve route target.');
            this.backToList();
          }
        });
      },
      error: () => {
        this.loading.set(false);
        this.toast.error('Failed to load routing data.');
        this.backToList();
      }
    });
  }

  deselectRoute(): void {
    this.backToList();
  }

  backToList(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'networking']);
  }

  onSaveDomainSettings(): void {
    const route = this.selectedRoute();
    if (!route || route.type !== 'custom') return;

    this.updatingSettings.set(true);
    this.domainService.updateDomain(route.id, {
      fqdn: route.fqdn,
      targetType: (route.rawObject as Domain).targetType || 'custom',
      routingType: this.editRoutingType(),
      clientMaxBodySize: this.editClientMaxBodySize(),
      isSsl: this.editIsSsl(),
      nginxTargetHost: this.editRoutingType() === 'reverse_proxy' ? this.editNginxTargetHost().trim() || undefined : undefined,
      nginxRootPath: this.editRoutingType() === 'static_host' ? this.editNginxRootPath().trim() || undefined : undefined,
      nginxConfigContent: this.editRoutingType() === 'custom' ? this.editNginxConfigContent().trim() || undefined : undefined,
    }).subscribe({
      next: () => {
        this.updatingSettings.set(false);
        this.toast.success('Domain configuration updated!');
        this.loadRouteDetail(route.id);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to update configuration.');
        this.updatingSettings.set(false);
      }
    });
  }

  startLogStream(route: UnifiedRoute): void {
    this.stopLogSimulation();
    if (route.type !== 'custom') {
      this.liveLogs.set([]);
      this.logsSupported.set(false);
      return;
    }
    this.logsSupported.set(true);
    this.loadDomainLogs(route.id);
    this.logTimer = setInterval(() => this.loadDomainLogs(route.id), 5000);
  }

  loadDomainLogs(id: string): void {
    this.domainService.getDomainLogs(id).subscribe({
      next: (res) => {
        this.logsSupported.set(res.supported);
        this.liveLogs.set(res.lines || []);
      },
      error: () => { /* keep last view */ }
    });
  }

  stopLogSimulation(): void {
    if (this.logTimer) {
      clearInterval(this.logTimer);
      this.logTimer = null;
    }
  }

  refreshDiagnostics(): void {
    this.dnsVerified.set(false);
    this.certRequested.set(false);
    this.tlsSecured.set(false);

    setTimeout(() => {
      this.dnsVerified.set(true);
    }, 800);
    setTimeout(() => {
      this.certRequested.set(true);
    }, 1600);
    setTimeout(() => {
      this.tlsSecured.set(true);
    }, 2400);
  }
}
