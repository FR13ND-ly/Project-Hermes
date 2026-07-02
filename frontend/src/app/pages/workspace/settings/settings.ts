import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { WorkspaceService, WorkspaceUsage, Workspace, WorkspaceMember } from '../../../core/services/workspace.service';
import { GitService, GitCredential, GitProvider } from '../../../core/services/git.service';
import { CloudflareService, CloudflareCredential } from '../../../core/services/cloudflare.service';
import { AuthService } from '../../../core/services/auth';
import { ToastService } from '../../../core/services/toast.service';
import { ConfirmService } from '../../../core/services/confirm.service';
import { DecimalPipe } from '@angular/common';

@Component({
  selector: 'app-workspace-settings',
  imports: [DecimalPipe, FormsModule],
  templateUrl: './settings.html',
  styleUrl: './settings.css',
})
export class WorkspaceSettings implements OnInit {
  private readonly workspaceService = inject(WorkspaceService);
  private readonly gitService = inject(GitService);
  private readonly cloudflareService = inject(CloudflareService);
  readonly auth = inject(AuthService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly usage = signal<WorkspaceUsage | null>(null);
  readonly workspace = signal<Workspace | null>(null);
  readonly loading = signal(false);
  readonly saving = signal(false);
  readonly error = signal<string | null>(null);
  readonly successMsg = signal<string | null>(null);

  // Tab signals
  readonly activeSubTab = signal<'general' | 'members'>('general');

  // Form fields signals
  readonly wsName = signal('');
  readonly maxMemory = signal<number>(0);
  readonly maxStorage = signal<number>(0);
  readonly maxCpu = signal<number>(0);

  // Git credentials (PATs)
  readonly gitCredentials = signal<GitCredential[]>([]);
  readonly newCredProvider = signal<GitProvider>('github');
  readonly newCredHost = signal('');
  readonly newCredLabel = signal('');
  readonly newCredToken = signal('');
  readonly newCredSkipTls = signal(false);
  readonly addingCred = signal(false);

  // Cloudflare credentials (token + zone + base domain; one credential = one domain)
  readonly cloudflareCredentials = signal<CloudflareCredential[]>([]);
  readonly newCfLabel = signal('');
  readonly newCfToken = signal('');
  readonly newCfZoneId = signal('');
  readonly newCfBaseDomain = signal('');
  readonly addingCf = signal(false);

  // Members list & forms signals
  readonly members = signal<WorkspaceMember[]>([]);
  readonly loadingMembers = signal(false);
  readonly invitingMember = signal(false);
  readonly newMemberEmail = signal('');
  readonly newMemberRole = signal('developer');

  ngOnInit(): void {
    this.loadData();
    this.loadGitCredentials();
    this.loadCloudflareCredentials();
  }

  loadCloudflareCredentials(): void {
    this.cloudflareService.listCredentials().subscribe({
      next: (creds) => this.cloudflareCredentials.set(creds || []),
      error: () => this.cloudflareCredentials.set([])
    });
  }

  onAddCloudflareCredential(): void {
    const label = this.newCfLabel().trim();
    const token = this.newCfToken().trim();
    const zoneId = this.newCfZoneId().trim();
    if (!label || !token || !zoneId) {
      this.toast.error('Label, token and Zone ID are required.');
      return;
    }
    this.addingCf.set(true);
    this.cloudflareService.createCredential({
      label, token, zoneId,
      baseDomain: this.newCfBaseDomain().trim() || undefined,
    }).subscribe({
      next: () => {
        this.addingCf.set(false);
        this.newCfLabel.set('');
        this.newCfToken.set('');
        this.newCfZoneId.set('');
        this.newCfBaseDomain.set('');
        this.toast.success('Cloudflare token added.');
        this.loadCloudflareCredentials();
      },
      error: (err) => {
        this.addingCf.set(false);
        this.toast.error(err.error?.message || 'Failed to add Cloudflare token.');
      }
    });
  }

  async onDeleteCloudflareCredential(cred: CloudflareCredential): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Cloudflare Token',
      message: `Are you sure you want to delete "${cred.label}"? Projects using it will lose Cloudflare DNS until another token is assigned.`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;
    this.cloudflareService.deleteCredential(cred.id).subscribe({
      next: () => { this.toast.success('Cloudflare token deleted.'); this.loadCloudflareCredentials(); },
      error: (err) => this.toast.error(err.error?.message || 'Failed to delete.')
    });
  }

  loadGitCredentials(): void {
    this.gitService.listCredentials().subscribe({
      next: (creds) => this.gitCredentials.set(creds || []),
      error: () => this.gitCredentials.set([])
    });
  }

  onAddCredential(): void {
    const label = this.newCredLabel().trim();
    const token = this.newCredToken().trim();
    if (!label || !token) {
      this.toast.error('Label and token are required.');
      return;
    }
    this.addingCred.set(true);
    this.gitService.createCredential({
      provider: this.newCredProvider(),
      host: this.newCredHost().trim() || undefined,
      label,
      token,
      skipTlsVerify: this.newCredSkipTls(),
    }).subscribe({
      next: () => {
        this.addingCred.set(false);
        this.newCredLabel.set('');
        this.newCredToken.set('');
        this.newCredHost.set('');
        this.newCredSkipTls.set(false);
        this.toast.success('Git credential added.');
        this.loadGitCredentials();
      },
      error: (err) => {
        this.addingCred.set(false);
        this.toast.error(err.error?.message || 'Failed to add credential (invalid token?).');
      }
    });
  }

  async onDeleteCredential(cred: GitCredential): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Git Credential',
      message: `Are you sure you want to delete the credential "${cred.label}"? Applications using it will no longer be able to clone until reassigned.`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;
    this.gitService.deleteCredential(cred.id).subscribe({
      next: () => { this.toast.success('Credential deleted.'); this.loadGitCredentials(); },
      error: (err) => this.toast.error(err.error?.message || 'Failed to delete.')
    });
  }

  loadData(): void {
    this.loading.set(true);
    this.error.set(null);
    this.successMsg.set(null);

    // Load usage
    this.workspaceService.getUsage().subscribe({
      next: (res) => this.usage.set(res),
      error: (err) => console.error('Failed to load resource quotas.', err)
    });

    // Load members
    this.loadMembers();

    // Load details
    this.workspaceService.getCurrentWorkspace().subscribe({
      next: (res) => {
        this.workspace.set(res);
        this.wsName.set(res.name);
        this.maxMemory.set(res.maxMemoryMb);
        this.maxStorage.set(res.maxStorageGb);
        this.maxCpu.set(res.maxCpuMillicores);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Failed to load workspace details.');
        this.loading.set(false);
      }
    });
  }

  onSaveSettings(): void {
    if (!this.wsName().trim()) {
      this.error.set('Workspace name is required.');
      return;
    }

    this.saving.set(true);
    this.error.set(null);
    this.successMsg.set(null);

    this.workspaceService.updateWorkspace({
      name: this.wsName().trim(),
      maxMemoryMb: this.maxMemory(),
      maxStorageGb: this.maxStorage(),
      maxCpuMillicores: this.maxCpu(),
    }).subscribe({
      next: (res) => {
        this.saving.set(false);
        this.successMsg.set('Workspace settings updated successfully.');
        this.loadData();
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Failed to save settings.');
        this.saving.set(false);
      }
    });
  }

  loadMembers(): void {
    this.loadingMembers.set(true);
    this.workspaceService.listMembers().subscribe({
      next: (res) => {
        this.members.set(res || []);
        this.loadingMembers.set(false);
      },
      error: (err) => {
        console.error('Failed to load members.', err);
        this.loadingMembers.set(false);
      }
    });
  }

  onInviteMember(): void {
    const email = this.newMemberEmail().trim();
    const role = this.newMemberRole();
    if (!email) {
      this.toast.error('Email address is required.');
      return;
    }

    this.invitingMember.set(true);
    this.workspaceService.addMember(email, role).subscribe({
      next: () => {
        this.toast.success('Member added successfully!');
        this.newMemberEmail.set('');
        this.invitingMember.set(false);
        this.loadMembers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to add member.');
        this.invitingMember.set(false);
      }
    });
  }

  onUpdateRole(member: WorkspaceMember, newRole: string): void {
    const rName = member.roleName.toString();
    if (rName === newRole) return;
    
    this.workspaceService.updateMemberRole(member.userId, newRole).subscribe({
      next: () => {
        this.toast.success('Member role updated.');
        this.loadMembers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to update role.');
      }
    });
  }

  async onRemoveMember(member: WorkspaceMember): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Remove Member',
      message: `Are you sure you want to remove "${member.username}" (${member.email}) from this workspace?`,
      confirmText: 'Remove',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.workspaceService.removeMember(member.userId).subscribe({
      next: () => {
        this.toast.success('Member has been removed.');
        this.loadMembers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to remove member.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copied to clipboard!');
    });
  }
}
