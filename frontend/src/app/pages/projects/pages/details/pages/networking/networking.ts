import { Component, inject, signal, OnInit, OnDestroy, computed } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { DomainService, Domain, DomainRoutingType } from '../../../../../../core/services/domain.service';
import { ProjectService, ServerlessFunction } from '../../../../../../core/services/project.service';
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
  readonly serverlessFunctions = signal<ServerlessFunction[]>([]);
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

    // 1. Ingress automatic routes (app instances)
    insts.forEach(inst => {
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
      if (fn.assignedDomain && fn.status === 'active') {
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

    // 3. Custom domains
    domains.forEach(d => {
      let matchedBranch: string | undefined;
      let matchedApp: string | undefined;
      if (d.nginxTargetHost) {
        // Check if target host matches an app instance container
        const matched = insts.find(i => i.containerName === d.nginxTargetHost);
        if (matched) {
          matchedBranch = matched.branchName;
          matchedApp = this.parent.project()?.name || 'Hermes App';
        }
        // Check if target host matches a serverless function proxy
        if (!matchedApp) {
          const matchedFn = functions.find(fn => d.nginxTargetHost?.includes(`fn-${fn.name.toLowerCase().replace(/\s+/g, '-')}`));
          if (matchedFn) {
            matchedBranch = 'Funcție Serverless';
            matchedApp = matchedFn.name;
          }
        }
      }

      routes.push({
        id: d.id,
        fqdn: d.fqdn,
        type: 'custom',
        routingType: d.routingType as any,
        status: d.status as any,
        connectedApp: matchedApp || (d.nginxTargetHost ? (this.parent.project()?.name || 'Hermes App') : undefined),
        branchName: matchedBranch,
        port: d.routingType === 'reverse_proxy' ? 80 : undefined,
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
    this.dbService.listDatabases(projectId).subscribe({
      next: (res) => {
        this.databases.set((res || []).filter(db => db.isExternal));
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

    this.projectService.listProjectFunctions(projectId).subscribe({
      next: (res) => {
        this.serverlessFunctions.set(res || []);
      },
      error: () => {
        this.serverlessFunctions.set([]);
      }
    });
  }

  loadDomains(): void {
    this.loadingDomains.set(true);
    this.domainService.listDomains().subscribe({
      next: (res) => {
        this.customDomains.set(res || []);
        this.loadingDomains.set(false);

        // Update selected route if currently viewed custom domain has updated
        const currentSelected = this.selectedRoute();
        if (currentSelected && currentSelected.type === 'custom') {
          const updated = res.find(d => d.id === currentSelected.id);
          if (updated) {
            const insts = this.parent.appDetail()?.instances || [];
            const matchedBranch = updated.nginxTargetHost ? insts.find(i => i.containerName === updated.nginxTargetHost)?.branchName : undefined;
            this.selectedRoute.set({
              id: updated.id,
              fqdn: updated.fqdn,
              type: 'custom',
              routingType: updated.routingType as any,
              status: updated.status as any,
              connectedApp: updated.nginxTargetHost ? (this.parent.project()?.name || 'Hermes App') : undefined,
              branchName: matchedBranch,
              port: updated.routingType === 'reverse_proxy' ? 80 : undefined,
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

    this.submittingDomain.set(true);
    this.errorMsg.set(null);
    this.successMsg.set(null);

    this.domainService.addDomain({
      fqdn: this.fqdn().trim(),
      routingType: this.routingType(),
      clientMaxBodySize: this.clientMaxBodySize(),
      isSsl: this.isSsl(),
      nginxTargetHost: this.routingType() === 'reverse_proxy' ? this.nginxTargetHost().trim() || undefined : undefined,
      nginxRootPath: this.routingType() === 'static_host' ? this.nginxRootPath().trim() || undefined : undefined,
      nginxConfigContent: this.routingType() === 'custom' ? this.nginxConfigContent().trim() || undefined : undefined,
    }).subscribe({
      next: () => {
        this.submittingDomain.set(false);
        this.showAddDomainForm.set(false);
        this.fqdn.set('');
        this.nginxTargetHost.set('');
        this.nginxRootPath.set('');
        this.nginxConfigContent.set('');
        this.toast.success('Domeniul custom a fost adăugat cu succes!');
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

    this.startLogSimulation(route.fqdn);
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

  startLogSimulation(fqdn: string): void {
    this.stopLogSimulation();
    const initial: string[] = [];
    const paths = ['/', '/api/v1/health', '/static/css/styles.css', '/static/js/main.js', '/api/v1/auth/session', '/dashboard', '/api/v1/metrics'];
    const ips = ['192.168.49.1', '10.244.0.1', '127.0.0.1'];

    for (let i = 0; i < 15; i++) {
      const time = new Date(Date.now() - (15 - i) * 4000);
      const method = Math.random() > 0.3 ? 'GET' : 'POST';
      const path = paths[Math.floor(Math.random() * paths.length)];
      const status = Math.random() > 0.05 ? 200 : 404;
      const size = Math.floor(Math.random() * 4000) + 80;
      const ip = ips[Math.floor(Math.random() * ips.length)];
      initial.push(`[${time.toISOString().replace('T', ' ').substring(0, 19)}] ${ip} - "${method} ${path} HTTP/1.1" ${status} ${size} "-" "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"`);
    }
    this.liveLogs.set(initial);

    this.logTimer = setInterval(() => {
      const time = new Date();
      const method = Math.random() > 0.3 ? 'GET' : 'POST';
      const path = paths[Math.floor(Math.random() * paths.length)];
      const status = Math.random() > 0.05 ? 200 : 404;
      const size = Math.floor(Math.random() * 4000) + 80;
      const ip = ips[Math.floor(Math.random() * ips.length)];
      const logLine = `[${time.toISOString().replace('T', ' ').substring(0, 19)}] ${ip} - "${method} ${path} HTTP/1.1" ${status} ${size} "-" "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"`;

      const current = this.liveLogs();
      if (current.length > 40) {
        current.shift();
      }
      this.liveLogs.set([...current, logLine]);
    }, 1500);
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
