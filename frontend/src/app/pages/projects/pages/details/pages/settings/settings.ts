import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, AppInstance, ProjectWebhook, ProjectSshKey, UpdateProjectSettingsRequest } from '../../../../../../core/services/project.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { ToastService } from '../../../../../../core/services/toast.service';

@Component({
  selector: 'app-settings',
  imports: [FormsModule, CommonModule],
  templateUrl: './settings.html',
  styleUrl: './settings.css',
})
export class Settings implements OnInit {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  private readonly confirm = inject(ConfirmService);
  private readonly toast = inject(ToastService);

  // CPU/RAM defaults or loaded values
  readonly cpuLimit = signal(500); // m
  readonly memLimit = signal(1024); // MB
  readonly internalPort = signal(8080);
  readonly replicasMin = signal(1);
  readonly replicasMax = signal(3);

  readonly saving = signal(false);
  readonly saveSuccess = signal(false);
  readonly error = signal<string | null>(null);

  // Destructive actions
  readonly selectedInstanceToDelete = signal<AppInstance | null>(null);
  readonly confirmName = signal('');
  readonly deleting = signal(false);

  // Project deletion
  readonly confirmProjectName = signal('');
  readonly deletingProject = signal(false);

  // Webhooks integrations signals
  readonly webhooks = signal<ProjectWebhook[]>([]);
  readonly loadingWebhooks = signal(false);
  readonly newWebhookName = signal('');
  readonly newWebhookUrl = signal('');
  readonly newWebhookType = signal<'slack' | 'discord' | 'generic'>('slack');

  // SSH Keys integrations signals
  readonly sshKeys = signal<ProjectSshKey[]>([]);
  readonly loadingSshKeys = signal(false);
  readonly newSshKeyName = signal('');
  readonly newSshKeyHost = signal('');
  readonly newSshKeyPrivateKey = signal('');
  readonly autoGenerateSsh = signal(true);
  readonly generatingSshKey = signal(false);

  // Cloudflare / Ingress (project-level) settings
  readonly cfApiToken = signal('');
  readonly cfZoneId = signal('');
  readonly cfIngressIp = signal('');
  readonly cfBaseDomain = signal('');
  readonly cfHasToken = signal(false);
  readonly savingCloudflare = signal(false);

  constructor() {
    effect(() => {
      const details = this.parent.appDetail();
      if (details && details.instances && details.instances.length > 0) {
        // Load settings from the first instance as reference
        const inst = details.instances[0];
        const cpuVal = inst.cpuLimit || 500;
        const memVal = inst.memoryLimitMb || 1024;
        const portVal = inst.internalPort || 8080;
        this.cpuLimit.set(cpuVal);
        this.memLimit.set(memVal);
        this.internalPort.set(portVal);
        
        // If not already selected, default the instance to delete to the first one
        if (!this.selectedInstanceToDelete()) {
          this.selectedInstanceToDelete.set(inst);
        }
      }
    });
  }

  ngOnInit(): void {
    const details = this.parent.appDetail();
    if (details && details.instances && details.instances.length > 0) {
      this.selectedInstanceToDelete.set(details.instances[0]);
    }
    this.loadWebhooks();
    this.loadSshKeys();
    this.loadCloudflareSettings();
  }

  loadCloudflareSettings(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.projectService.getProjectSettings(projectId).subscribe({
      next: (s) => {
        this.cfZoneId.set(s.cloudflareZoneId || '');
        this.cfIngressIp.set(s.ingressIp || '');
        this.cfBaseDomain.set(s.baseDomain || '');
        this.cfHasToken.set(s.hasCloudflareToken);
        this.cfApiToken.set('');
      },
      error: () => {}
    });
  }

  onSaveCloudflareSettings(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.savingCloudflare.set(true);
    const payload: UpdateProjectSettingsRequest = {
      cloudflareZoneId: this.cfZoneId().trim() || null,
      ingressIp: this.cfIngressIp().trim() || null,
      baseDomain: this.cfBaseDomain().trim() || null,
    };
    // Only send the token when the user typed a new one — otherwise the stored secret is kept.
    const token = this.cfApiToken().trim();
    if (token) payload.cloudflareApiToken = token;

    this.projectService.updateProjectSettings(projectId, payload).subscribe({
      next: (s) => {
        this.savingCloudflare.set(false);
        this.cfHasToken.set(s.hasCloudflareToken);
        this.cfApiToken.set('');
        this.toast.success('Setările Cloudflare / Ingress au fost salvate.');
      },
      error: (err) => {
        this.savingCloudflare.set(false);
        this.toast.error(err.error?.message || 'Eroare la salvarea setărilor Cloudflare.');
      }
    });
  }

  onSaveSettings(): void {
    const details = this.parent.appDetail();
    const appId = details?.id;
    if (!appId || !details.instances || details.instances.length === 0) return;
    
    const inst = details.instances[0];
    this.saving.set(true);
    this.saveSuccess.set(false);
    this.error.set(null);

    this.projectService.updateInstanceSettings(appId, inst.id, {
      cpuLimit: this.cpuLimit(),
      memoryLimitMb: this.memLimit(),
      internalPort: this.internalPort()
    }).subscribe({
      next: () => {
        this.saving.set(false);
        this.saveSuccess.set(true);
        this.parent.loadDetails(this.parent.projectId() || '');
        setTimeout(() => this.saveSuccess.set(false), 4000);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la salvarea setărilor.');
        this.saving.set(false);
      }
    });
  }

  onSelectInstanceToDelete(event: Event): void {
    const select = event.target as HTMLSelectElement;
    const details = this.parent.appDetail();
    if (details && details.instances) {
      const found = details.instances.find(i => i.id === select.value);
      if (found) {
        this.selectedInstanceToDelete.set(found);
      }
    }
  }

  onDeleteInstance(): void {
    const inst = this.selectedInstanceToDelete();
    const appId = this.parent.appDetail()?.id;
    const projectId = this.parent.projectId();
    if (!inst || !appId || !projectId || this.confirmName() !== inst.containerName) return;

    this.deleting.set(true);
    this.error.set(null);

    this.projectService.deleteAppInstance(appId, inst.id).subscribe({
      next: () => {
        this.deleting.set(false);
        this.confirmName.set('');
        alert(`Instanța ${inst.containerName} a fost ștearsă din Kubernetes.`);
        
        // Reload details parent component
        this.parent.loadDetails(projectId);
        
        // Redirect to dashboard if no instances left
        const updatedDetails = this.parent.appDetail();
        if (!updatedDetails || !updatedDetails.instances || updatedDetails.instances.length <= 1) {
          this.router.navigate(['/dashboard']);
        } else {
          this.selectedInstanceToDelete.set(updatedDetails.instances.find(i => i.id !== inst.id) || null);
        }
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la ștergerea instanței.');
        this.deleting.set(false);
      }
    });
  }

  async onDeleteProject(): Promise<void> {
    const projectId = this.parent.projectId();
    const project = this.parent.project();
    if (!projectId || !project || this.confirmProjectName() !== project.name) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Completă Proiect',
      message: `Sigur doriți să ștergeți complet proiectul "${project.name}"? Această acțiune va șterge ireversibil toate aplicațiile, instanțele, bazele de date, stocările și variabilele de mediu din Kubernetes și baza de date.`,
      confirmText: 'Șterge Proiect',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.deletingProject.set(true);
    this.error.set(null);

    this.projectService.deleteProject(projectId).subscribe({
      next: () => {
        this.deletingProject.set(false);
        this.confirmProjectName.set('');
        this.toast.success(`Proiectul "${project.name}" a fost șters cu succes.`);
        this.router.navigate(['/projects']);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la ștergerea proiectului.');
        this.deletingProject.set(false);
      }
    });
  }

  // --- Webhooks Operations ---
  loadWebhooks(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingWebhooks.set(true);
    this.projectService.listProjectWebhooks(projectId).subscribe({
      next: (res) => {
        this.webhooks.set(res || []);
        this.loadingWebhooks.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea webhook-urilor.');
        this.loadingWebhooks.set(false);
      }
    });
  }

  onCreateWebhook(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const name = this.newWebhookName().trim();
    const url = this.newWebhookUrl().trim();
    const webhookType = this.newWebhookType();

    if (!name || !url) {
      this.toast.error('Toate câmpurile sunt obligatorii.');
      return;
    }

    this.projectService.createWebhook(projectId, { name, url, webhookType }).subscribe({
      next: () => {
        this.toast.success('Webhook-ul a fost adăugat cu succes.');
        this.newWebhookName.set('');
        this.newWebhookUrl.set('');
        this.loadWebhooks();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea webhook-ului.');
      }
    });
  }

  async onDeleteWebhook(webhookId: string): Promise<void> {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Integrare Webhook',
      message: 'Sigur doriți să ștergeți această integrare? Nu veți mai primi alerte pe acest canal!',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteWebhook(projectId, webhookId).subscribe({
      next: () => {
        this.toast.success('Integrarea a fost ștearsă.');
        this.loadWebhooks();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea webhook-ului.');
      }
    });
  }

  // --- SSH Keys Operations ---
  loadSshKeys(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingSshKeys.set(true);
    this.projectService.listProjectSshKeys(projectId).subscribe({
      next: (res) => {
        this.sshKeys.set(res || []);
        this.loadingSshKeys.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea cheilor SSH.');
        this.loadingSshKeys.set(false);
      }
    });
  }

  onCreateSshKey(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const name = this.newSshKeyName().trim();
    const host = this.newSshKeyHost().trim();
    const privateKey = this.autoGenerateSsh() ? null : this.newSshKeyPrivateKey().trim();

    if (!name || !host) {
      this.toast.error('Numele și host-ul sunt obligatorii.');
      return;
    }

    if (!this.autoGenerateSsh() && !privateKey) {
      this.toast.error('Cheia privată trebuie introdusă dacă nu generați automat.');
      return;
    }

    this.generatingSshKey.set(true);
    this.projectService.createProjectSshKey(projectId, {
      name,
      host,
      privateKey
    }).subscribe({
      next: () => {
        this.toast.success('Cheia SSH a fost configurată cu succes.');
        this.newSshKeyName.set('');
        this.newSshKeyHost.set('');
        this.newSshKeyPrivateKey.set('');
        this.generatingSshKey.set(false);
        this.loadSshKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea cheii SSH.');
        this.generatingSshKey.set(false);
      }
    });
  }

  async onDeleteSshKey(keyId: string): Promise<void> {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Cheie SSH Proiect',
      message: 'Sigur doriți să ștergeți această cheie SSH? Aplicațiile care folosesc acest host nu se vor mai putea clona la următorul build!',
      confirmText: 'Șterge Cheie',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteProjectSshKey(projectId, keyId).subscribe({
      next: () => {
        this.toast.success('Cheia SSH a fost ștearsă.');
        this.loadSshKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea cheii SSH.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Cheia publică a fost copiată.');
    }).catch(() => {
      this.toast.error('Nu s-a putut copia cheia.');
    });
  }
}
