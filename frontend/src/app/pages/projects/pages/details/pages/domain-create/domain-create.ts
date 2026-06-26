import { Component, inject, signal, computed, OnInit } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { DomainService, DomainTargetType, DomainRoutingType } from '../../../../../../core/services/domain.service';
import { ProjectService, ServerlessInstance } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';

@Component({
  selector: 'app-domain-create',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './domain-create.html',
  styleUrl: './domain-create.css',
})
export class DomainCreate implements OnInit {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly domainService = inject(DomainService);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly serverlessFunctions = signal<ServerlessInstance[]>([]);
  readonly loadingDbs = signal(false);
  readonly submitting = signal(false);

  // Form states (Addition)
  readonly fqdn = signal('');
  readonly routingType = signal<DomainRoutingType>('reverse_proxy');
  readonly clientMaxBodySize = signal<number>(50);
  readonly isSsl = signal<boolean>(true);
  readonly nginxTargetHost = signal('');
  readonly nginxRootPath = signal('');
  readonly nginxConfigContent = signal('');

  // Resource-oriented target selection
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

  ngOnInit(): void {
    this.loadExposedDbs();
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
        this.setDefaultTarget();
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
        this.setDefaultTarget();
      },
      error: () => {
        this.serverlessFunctions.set([]);
      }
    });
  }

  onChangeTargetType(type: DomainTargetType): void {
    this.newTargetType.set(type);
    this.setDefaultTarget();
  }

  private setDefaultTarget(): void {
    const first = this.targetOptions()[0];
    this.selectedTargetId.set(first ? first.id : '');
  }

  onAddDomain(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    if (!this.fqdn().trim()) {
      this.toast.error('Domain (FQDN) is required.');
      return;
    }

    const targetType = this.newTargetType();
    if (targetType !== 'custom' && !this.selectedTargetId()) {
      this.toast.error('Select the resource to connect the domain to.');
      return;
    }

    this.submitting.set(true);

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
        this.submitting.set(false);
        this.toast.success('Domain added successfully!');
        this.router.navigate(['/projects', projectId, 'networking']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to add domain.');
        this.submitting.set(false);
      }
    });
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'networking']);
  }
}
