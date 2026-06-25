import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, AppInstance, ProjectWebhook, ProjectSshKey, UpdateProjectSettingsRequest } from '../../../../../../core/services/project.service';
import { CloudflareService, CloudflareCredential } from '../../../../../../core/services/cloudflare.service';
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
  private readonly cloudflareService = inject(CloudflareService);
  private readonly router = inject(Router);
  private readonly confirm = inject(ConfirmService);
  private readonly toast = inject(ToastService);

  // Container settings are managed per-app in the App details view.
  readonly error = signal<string | null>(null);

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
  readonly cfCredentialId = signal('');
  readonly cloudflareCredentials = signal<CloudflareCredential[]>([]);
  readonly cfIngressIp = signal('');
  readonly cfBaseDomain = signal('');
  readonly savingCloudflare = signal(false);

  ngOnInit(): void {
    this.loadWebhooks();
    this.loadSshKeys();
    this.loadCloudflareSettings();
  }

  loadCloudflareSettings(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.cloudflareService.listCredentials().subscribe({
      next: (list) => this.cloudflareCredentials.set(list || []),
      error: () => this.cloudflareCredentials.set([])
    });
    this.projectService.getProjectSettings(projectId).subscribe({
      next: (s) => {
        this.cfCredentialId.set(s.cloudflareCredentialId || '');
        this.cfIngressIp.set(s.ingressIp || '');
        this.cfBaseDomain.set(s.baseDomain || '');
      },
      error: () => {}
    });
  }

  onSaveCloudflareSettings(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.savingCloudflare.set(true);
    const payload: UpdateProjectSettingsRequest = {
      cloudflareCredentialId: this.cfCredentialId() || null,
      ingressIp: this.cfIngressIp().trim() || null,
      baseDomain: this.cfBaseDomain().trim() || null,
    };

    this.projectService.updateProjectSettings(projectId, payload).subscribe({
      next: (s) => {
        this.savingCloudflare.set(false);
        this.cfCredentialId.set(s.cloudflareCredentialId || '');
        this.toast.success('Cloudflare / Ingress settings saved.');
      },
      error: (err) => {
        this.savingCloudflare.set(false);
        this.toast.error(err.error?.message || 'Failed to save Cloudflare settings.');
      }
    });
  }

  async onDeleteProject(): Promise<void> {
    const projectId = this.parent.projectId();
    const project = this.parent.project();
    if (!projectId || !project || this.confirmProjectName() !== project.name) return;

    const confirmed = await this.confirm.ask({
      title: 'Delete Project Completely',
      message: `Are you sure you want to completely delete project "${project.name}"? This will irreversibly delete all applications, instances, databases, storages and environment variables from Kubernetes and the database.`,
      confirmText: 'Delete Project',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.deletingProject.set(true);
    this.error.set(null);

    this.projectService.deleteProject(projectId).subscribe({
      next: () => {
        this.deletingProject.set(false);
        this.confirmProjectName.set('');
        this.toast.success(`Project "${project.name}" deleted successfully.`);
        this.router.navigate(['/projects']);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Failed to delete project.');
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
        this.toast.error(err.error?.message || 'Failed to load webhooks.');
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
      this.toast.error('All fields are required.');
      return;
    }

    this.projectService.createWebhook(projectId, { name, url, webhookType }).subscribe({
      next: () => {
        this.toast.success('Webhook added successfully.');
        this.newWebhookName.set('');
        this.newWebhookUrl.set('');
        this.loadWebhooks();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error creating webhook.');
      }
    });
  }

  async onDeleteWebhook(webhookId: string): Promise<void> {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const confirmed = await this.confirm.ask({
      title: 'Delete Webhook Integration',
      message: 'Are you sure you want to delete this integration? You will no longer receive alerts on this channel!',
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteWebhook(projectId, webhookId).subscribe({
      next: () => {
        this.toast.success('Integration deleted.');
        this.loadWebhooks();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete webhook.');
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
        this.toast.error(err.error?.message || 'Failed to load SSH keys.');
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
      this.toast.error('Name and host are required.');
      return;
    }

    if (!this.autoGenerateSsh() && !privateKey) {
      this.toast.error('Private key must be provided if not generating automatically.');
      return;
    }

    this.generatingSshKey.set(true);
    this.projectService.createProjectSshKey(projectId, {
      name,
      host,
      privateKey
    }).subscribe({
      next: () => {
        this.toast.success('SSH key configured successfully.');
        this.newSshKeyName.set('');
        this.newSshKeyHost.set('');
        this.newSshKeyPrivateKey.set('');
        this.generatingSshKey.set(false);
        this.loadSshKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error creating SSH key.');
        this.generatingSshKey.set(false);
      }
    });
  }

  async onDeleteSshKey(keyId: string): Promise<void> {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const confirmed = await this.confirm.ask({
      title: 'Delete Project SSH Key',
      message: 'Are you sure you want to delete this SSH key? Applications using this host will no longer be able to clone on the next build!',
      confirmText: 'Delete Key',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteProjectSshKey(projectId, keyId).subscribe({
      next: () => {
        this.toast.success('SSH key deleted.');
        this.loadSshKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete SSH key.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Public key copied.');
    }).catch(() => {
      this.toast.error('Nu s-a putut copia cheia.');
    });
  }
}
