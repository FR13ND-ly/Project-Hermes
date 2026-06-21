import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { WorkspaceService, WorkspaceUsage, Workspace, WorkspaceMember } from '../../../core/services/workspace.service';
import { GitService, GitCredential, GitProvider } from '../../../core/services/git.service';
import { AuthService } from '../../../core/services/auth';
import { ToastService } from '../../../core/services/toast.service';
import { ConfirmService } from '../../../core/services/confirm.service';
import { DecimalPipe, CommonModule } from '@angular/common';

@Component({
  selector: 'app-workspace-settings',
  imports: [DecimalPipe, CommonModule, FormsModule],
  templateUrl: './settings.html',
  styleUrl: './settings.css',
})
export class WorkspaceSettings implements OnInit {
  private readonly workspaceService = inject(WorkspaceService);
  private readonly gitService = inject(GitService);
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

  // Members list & forms signals
  readonly members = signal<WorkspaceMember[]>([]);
  readonly loadingMembers = signal(false);
  readonly invitingMember = signal(false);
  readonly newMemberEmail = signal('');
  readonly newMemberRole = signal('developer');

  ngOnInit(): void {
    this.loadData();
    this.loadGitCredentials();
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
      this.toast.error('Eticheta și token-ul sunt obligatorii.');
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
        this.toast.success('Credențiala Git a fost adăugată.');
        this.loadGitCredentials();
      },
      error: (err) => {
        this.addingCred.set(false);
        this.toast.error(err.error?.message || 'Eroare la adăugarea credențialei (token invalid?).');
      }
    });
  }

  async onDeleteCredential(cred: GitCredential): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere credențială Git',
      message: `Sigur ștergi credențiala "${cred.label}"? Aplicațiile care o folosesc nu vor mai putea clona până la realocare.`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;
    this.gitService.deleteCredential(cred.id).subscribe({
      next: () => { this.toast.success('Credențială ștearsă.'); this.loadGitCredentials(); },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la ștergere.')
    });
  }

  loadData(): void {
    this.loading.set(true);
    this.error.set(null);
    this.successMsg.set(null);

    // Load usage
    this.workspaceService.getUsage().subscribe({
      next: (res) => this.usage.set(res),
      error: (err) => console.error('Eroare la încărcarea cotelor de resurse.', err)
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
        this.error.set(err.error?.message || 'Eroare la încărcarea detaliilor workspace-ului.');
        this.loading.set(false);
      }
    });
  }

  onSaveSettings(): void {
    if (!this.wsName().trim()) {
      this.error.set('Numele workspace-ului este obligatoriu.');
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
        this.successMsg.set('Setările spațiului de lucru au fost actualizate cu succes.');
        this.loadData();
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la salvarea setărilor.');
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
        console.error('Eroare la încărcarea membrilor.', err);
        this.loadingMembers.set(false);
      }
    });
  }

  onInviteMember(): void {
    const email = this.newMemberEmail().trim();
    const role = this.newMemberRole();
    if (!email) {
      this.toast.error('Adresa de email este obligatorie.');
      return;
    }

    this.invitingMember.set(true);
    this.workspaceService.addMember(email, role).subscribe({
      next: () => {
        this.toast.success('Membru adăugat cu succes!');
        this.newMemberEmail.set('');
        this.invitingMember.set(false);
        this.loadMembers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la adăugarea membrului.');
        this.invitingMember.set(false);
      }
    });
  }

  onUpdateRole(member: WorkspaceMember, newRole: string): void {
    const rName = member.roleName.toString();
    if (rName === newRole) return;
    
    this.workspaceService.updateMemberRole(member.userId, newRole).subscribe({
      next: () => {
        this.toast.success('Rolul membrului a fost actualizat.');
        this.loadMembers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la modificarea rolului.');
      }
    });
  }

  async onRemoveMember(member: WorkspaceMember): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Excludere Membru',
      message: `Sigur doriți să eliminați utilizatorul "${member.username}" (${member.email}) din acest workspace?`,
      confirmText: 'Elimină',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.workspaceService.removeMember(member.userId).subscribe({
      next: () => {
        this.toast.success('Membrul a fost eliminat.');
        this.loadMembers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la eliminarea membrului.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }
}
