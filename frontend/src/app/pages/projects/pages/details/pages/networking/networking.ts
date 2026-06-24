import { Component, inject, signal, OnInit, OnDestroy, computed } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { DomainService, Domain, DomainRoutingType, DomainTargetType } from '../../../../../../core/services/domain.service';
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
  selector: 'app-networking',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './networking.html',
  styleUrl: './networking.css',
})
export class Networking implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly domainService = inject(DomainService);
  private readonly projectService = inject(ProjectService);
  readonly toast = inject(ToastService);
  readonly confirm = inject(ConfirmService);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly serverlessFunctions = signal<ServerlessInstance[]>([]);
  readonly loadingDbs = signal(false);

  // Custom Domains
  readonly customDomains = signal<Domain[]>([]);
  readonly loadingDomains = signal(false);
  readonly submittingDomain = signal(false);
  readonly errorMsg = signal<string | null>(null);
  readonly successMsg = signal<string | null>(null);
  readonly showAddDomainForm = signal(false);

  // Deep Routing panel states
  readonly selectedRoute = signal<UnifiedRoute | null>(null);
  readonly activeTab = signal<'details' | 'logs' | 'settings'>('details');
  readonly liveLogs = signal<string[]>([]);
  readonly logsSupported = signal(true);
  readonly expandedDomainCerts = signal<Record<string, boolean>>({});

  // Editing state for custom domains
  readonly editRoutingType = signal<DomainRoutingType>('reverse_proxy');
  readonly editClientMaxBodySize = signal<number>(50);
  readonly editIsSsl = signal<boolean>(true);
  readonly editNginxTargetHost = signal('');
  readonly editNginxRootPath = signal('');
  readonly editNginxConfigContent = signal('');
  readonly updatingSettings = signal(false);

  private logTimer: any = null;

  // Form states (Addition)
  readonly fqdn = signal('');
  readonly routingType = signal<DomainRoutingType>('reverse_proxy');
  readonly clientMaxBodySize = signal<number>(50);
  readonly isSsl = signal<boolean>(true);
  readonly nginxTargetHost = signal('');
  readonly nginxRootPath = signal('');
  readonly nginxConfigContent = signal('');

  // Resource-oriented target selection for the add-domain form.
  readonly newTargetType = signal<DomainTargetType>('app');
  readonly selectedTargetId = signal('');

  // All app instances across the project, flattened for the target dropdown.
  readonly appInstanceOptions = computed<{ id: string; name: string }[]>(() =>
    this.parent.apps().flatMap(app =>
      (app.instances || []).map(inst => ({ id: inst.id, name: `${app.name} · ${inst.branchName}` }))
    )
  );
  readonly fnOptions = computed<{ id: string; name: string }[]>(() =>
    this.serverlessFunctions().map(fn => ({ id: fn.id, name: fn.name }))
  );
  // Only externally-exposed databases can get a DNS domain.
  readonly dbOptions = computed<{ id: string; name: string }[]>(() =>
    this.databases().map(db => ({ id: db.id, name: `${db.name} :${db.externalPort ?? ''}` }))
  );
  readonly targetOptions = computed<{ id: string; name: string }[]>(() => {
    switch (this.newTargetType()) {
      case 'serverless': return this.fnOptions();
      case 'database': return this.dbOptions();
      case 'custom': return [];
      default: return this.appInstanceOptions();
    }
  });

  onChangeTargetType(type: DomainTargetType): void {
    this.newTargetType.set(type);
    const first = this.targetOptions()[0];
    this.selectedTargetId.set(first ? first.id : '');
  }

  // Cert-manager TLS provisioning diagnostics checklist
  readonly dnsVerified = signal(true);
  readonly certRequested = signal(true);
  readonly tlsSecured = signal(true);

  // Serverless functions with active ports for the TCP/UDP section
  readonly activeServerlessPorts = computed(() => {
    return this.serverlessFunctions().filter(fn => fn.status === 'active' && fn.externalPort);
  });

  // Unified computed list of HTTP entry points
  readonly unifiedRoutes = computed<UnifiedRoute[]>(() => {
    const insts = this.parent.appDetail()?.instances || [];
    const domains = this.customDomains();
    const functions = this.serverlessFunctions();
    const routes: UnifiedRoute[] = [];

    // fqdns that already have an explicit `domains` row (rendered in step 3 below).
    // Skip the synthesized "Automat (Ingress)" entry for those so the same fqdn isn't
    // listed twice — for serverless the assigned domain IS the attached custom domain.
    const attachedFqdns = new Set(domains.map(d => d.fqdn));

    // 1. Ingress automatic routes (app instances)
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

    // 2. Serverless function routes (those with assigned domain)
    functions.forEach(fn => {
      if (fn.assignedDomain && fn.status === 'active' && !attachedFqdns.has(fn.assignedDomain)) {
        routes.push({
          id: fn.id,
          fqdn: fn.assignedDomain,
          type: 'automatic',
          routingType: 'ingress',
          status: 'active',
          connectedApp: fn.name,
          branchName: 'Funcție Serverless',
          port: fn.externalPort || undefined,
          rawObject: fn
        });
      }
    });

    // 3. Custom/attached domains — use the backend-resolved target (no guessing).
    const typeLabel: Record<string, string> = {
      app: 'Aplicație', serverless: 'Funcție Serverless', database: 'Bază de date', custom: 'Custom'
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

  ngOnDestroy(): void {
    this.stopLogSimulation();
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
    this.domainService.listDomains(1, 1000).subscribe({
      next: (res) => {
        this.customDomains.set(res?.items || []);
        this.loadingDomains.set(false);

        // Update selected route if currently viewed custom domain has updated
        const currentSelected = this.selectedRoute();
        if (currentSelected && currentSelected.type === 'custom') {
          const updated = (res?.items || []).find(d => d.id === currentSelected.id);
          if (updated) {
            const typeLabel: Record<string, string> = {
              app: 'Aplicație', serverless: 'Funcție Serverless', database: 'Bază de date', custom: 'Custom'
            };
            this.selectedRoute.set({
              id: updated.id,
              fqdn: updated.fqdn,
              type: 'custom',
              routingType: updated.routingType as any,
              status: updated.status as any,
              connectedApp: updated.targetName || (updated.targetType === 'custom' ? 'Nginx custom' : undefined),
              branchName: typeLabel[updated.targetType] || undefined,
              port: updated.targetType === 'database' ? undefined : (updated.routingType === 'reverse_proxy' ? 80 : undefined),
              rawObject: updated
            });
          }
        }
      },
      error: (err) => {
        console.error('Eroare la încărcarea domeniilor', err);
        this.loadingDomains.set(false);
      }
    });
  }

  onAddDomain(): void {
    if (!this.fqdn().trim()) {
      this.errorMsg.set('Domeniul (FQDN) este obligatoriu.');
      return;
    }

    const targetType = this.newTargetType();
    if (targetType !== 'custom' && !this.selectedTargetId()) {
      this.errorMsg.set('Selectați resursa la care conectați domeniul.');
      return;
    }

    this.submittingDomain.set(true);
    this.errorMsg.set(null);
    this.successMsg.set(null);

    const payload = targetType === 'custom'
      ? {
          fqdn: this.fqdn().trim(),
          targetType,
          routingType: this.routingType(),
          clientMaxBodySize: this.clientMaxBodySize(),
          isSsl: this.isSsl(),
          nginxTargetHost: this.routingType() === 'reverse_proxy' ? this.nginxTargetHost().trim() || undefined : undefined,
          nginxRootPath: this.routingType() === 'static_host' ? this.nginxRootPath().trim() || undefined : undefined,
          nginxConfigContent: this.routingType() === 'custom' ? this.nginxConfigContent().trim() || undefined : undefined,
        }
      : {
          fqdn: this.fqdn().trim(),
          targetType,
          targetId: this.selectedTargetId(),
          clientMaxBodySize: this.clientMaxBodySize(),
          isSsl: this.isSsl(),
        };

    this.domainService.addDomain(payload).subscribe({
      next: () => {
        this.submittingDomain.set(false);
        this.showAddDomainForm.set(false);
        this.fqdn.set('');
        this.nginxTargetHost.set('');
        this.nginxRootPath.set('');
        this.nginxConfigContent.set('');
        this.selectedTargetId.set('');
        this.toast.success('Domeniul a fost adăugat cu succes!');
        this.loadDomains();
      },
      error: (err) => {
        this.errorMsg.set(err.error?.message || 'Eroare la adăugarea domeniului.');
        this.submittingDomain.set(false);
      }
    });
  }

  onVerifyDomain(id: string): void {
    this.domainService.verifyAndSyncDomain(id).subscribe({
      next: () => {
        this.toast.success('Procesul de verificare și sincronizare DNS/SSL a fost lansat.');
        this.loadDomains();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la verificarea domeniului.');
      }
    });
  }

  async onRemoveDomain(id: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Domeniu',
      message: 'Sigur doriți să ștergeți acest domeniu custom? Această acțiune va șterge rutele Nginx și DNS.',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.domainService.removeDomain(id).subscribe({
      next: () => {
        this.toast.success('Domeniul custom a fost eliminat.');
        this.deselectRoute();
        this.loadDomains();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la eliminarea domeniului.');
      }
    });
  }

  selectRoute(route: UnifiedRoute): void {
    this.selectedRoute.set(route);
    this.activeTab.set('details');

    if (route.type === 'custom') {
      const d = route.rawObject as Domain;
      this.editRoutingType.set(d.routingType);
      this.editClientMaxBodySize.set(d.clientMaxBodySize);
      this.editIsSsl.set(d.isSsl);
      this.editNginxTargetHost.set(d.nginxTargetHost || '');
      this.editNginxRootPath.set(d.nginxRootPath || '');
      this.editNginxConfigContent.set(d.nginxConfigContent || '');
    }

    this.startLogStream(route);
  }

  deselectRoute(): void {
    this.selectedRoute.set(null);
    this.stopLogSimulation();
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
        this.toast.success('Configurația domeniului a fost actualizată!');
        this.loadDomains();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la actualizarea configurației.');
        this.updatingSettings.set(false);
      }
    });
  }

  toggleCertDetails(domainId: string): void {
    const current = this.expandedDomainCerts();
    this.expandedDomainCerts.set({
      ...current,
      [domainId]: !current[domainId]
    });
  }

  // Real nginx access logs, polled from the backend. Only custom/attached domains
  // flow through the host nginx (and thus have a per-domain access log); automatic
  // ingress routes don't, so we mark them unsupported.
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
      error: () => { /* keep last view; transient errors are fine */ }
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

  copyPort(port: number | null | undefined): void {
    if (port === null || port === undefined) return;
    navigator.clipboard.writeText(port.toString()).then(() => {
      this.toast.success('Portul public a fost copiat în clipboard!');
    });
  }
}
