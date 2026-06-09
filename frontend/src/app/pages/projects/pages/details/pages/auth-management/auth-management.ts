import { Component, inject, signal, effect, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { AuthManagementService, AppUserWithRoles, ApiKeyInfo, CreateApiKeyResponse } from '../../../../../../core/services/auth-management.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-auth-management',
  standalone: true,
  imports: [CommonModule, FormsModule, DatePipe],
  templateUrl: './auth-management.html',
  styleUrl: './auth-management.css',
})
export class AuthManagement {
  readonly parent = inject(Details);
  private readonly authMgmtService = inject(AuthManagementService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly activeTab = signal<'users' | 'roles' | 'api-keys'>('users');
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Users tab states
  readonly users = signal<AppUserWithRoles[]>([]);
  readonly searchQuery = signal('');
  readonly currentPage = signal(1);
  readonly pageSize = signal(10);
  readonly totalUsers = signal(0);
  readonly totalPages = signal(0);

  readonly filteredUsers = computed(() => {
    return this.users();
  });

  // Modals for User Management
  readonly showResetPasswordModal = signal(false);
  readonly showRoleModal = signal(false);
  readonly selectedUser = signal<AppUserWithRoles | null>(null);
  
  // Password Reset state
  readonly newPassword = signal('');
  readonly resettingPassword = signal(false);

  // Role Assignment state
  readonly newRoleName = signal('');
  readonly assigningRole = signal(false);

  // JSON Config tab states
  readonly configText = signal('{\n  \n}');
  readonly parseError = signal<string | null>(null);
  readonly savingConfig = signal(false);

  // Visual Role Editor states
  readonly rolesEditMode = signal<'visual' | 'json'>('visual');
  readonly rolesConfig = signal<Record<string, string[]>>({});
  readonly newVisualRoleName = signal('');
  readonly newVisualPermissionName = signal<Record<string, string>>({});

  // API Keys tab states
  readonly apiKeys = signal<ApiKeyInfo[]>([]);
  readonly newKeyName = signal('');
  readonly newKeyExpiresAt = signal('');
  readonly creatingKey = signal(false);
  readonly generatedKeySecret = signal<string | null>(null);
  readonly showGeneratedKeyModal = signal(false);

  constructor() {
    // Reload active app data when the selected application changes
    effect(() => {
      const activeApp = this.parent.selectedApp();
      if (activeApp) {
        this.currentPage.set(1);
        this.searchQuery.set('');
        this.loadActiveTabData();
      } else {
        this.users.set([]);
        this.apiKeys.set([]);
        this.totalUsers.set(0);
        this.totalPages.set(0);
      }
    });

    // Reload tab data when tab changes
    effect(() => {
      const tab = this.activeTab();
      const activeApp = this.parent.selectedApp();
      if (activeApp && tab) {
        this.loadActiveTabData();
      }
    });
  }

  loadActiveTabData(): void {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    const tab = this.activeTab();
    if (tab === 'users') {
      this.loadUsers();
    } else if (tab === 'roles') {
      this.loadAuthConfig();
    } else if (tab === 'api-keys') {
      this.loadApiKeys();
    }
  }

  // --- Users Management ---
  loadUsers(): void {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    this.loading.set(true);
    this.authMgmtService.listUsers(
      activeApp.id,
      this.currentPage(),
      this.pageSize(),
      this.searchQuery()
    ).subscribe({
      next: (res) => {
        this.users.set(res.users || []);
        this.totalUsers.set(res.total || 0);
        this.totalPages.set(res.pages || 0);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea utilizatorilor.');
        this.loading.set(false);
      }
    });
  }

  onSearchChange(query: string): void {
    this.searchQuery.set(query);
    this.currentPage.set(1);
    this.loadUsers();
  }

  onPageChange(page: number): void {
    if (page < 1 || page > this.totalPages()) return;
    this.currentPage.set(page);
    this.loadUsers();
  }

  async onToggleUserStatus(user: AppUserWithRoles): Promise<void> {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    const nextStatus = user.status === 'active' ? 'suspended' : 'active';
    const actionText = nextStatus === 'suspended' ? 'suspendați' : 'reactivați';

    const confirmed = await this.confirm.ask({
      title: `${nextStatus === 'suspended' ? 'Suspendare' : 'Reactivare'} Cont`,
      message: `Sigur doriți să ${nextStatus === 'suspended' ? 'suspendați' : 'reactivați'} utilizatorul "${user.fullName}" (${user.email})? Conturile suspendate nu se mai pot autentifica în aplicație.`,
      confirmText: nextStatus === 'suspended' ? 'Suspendă cont' : 'Activează cont',
      cancelText: 'Anulează',
      isDanger: nextStatus === 'suspended'
    });
    if (!confirmed) return;

    this.authMgmtService.updateUserStatus(activeApp.id, user.appUserId, nextStatus).subscribe({
      next: () => {
        this.toast.success(`Contul a fost ${nextStatus === 'suspended' ? 'suspendat' : 'activat'} cu succes.`);
        this.loadUsers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la actualizarea statusului contului.');
      }
    });
  }

  onOpenResetPassword(user: AppUserWithRoles): void {
    this.selectedUser.set(user);
    this.newPassword.set('');
    this.showResetPasswordModal.set(true);
  }

  onResetPasswordSubmit(): void {
    const activeApp = this.parent.selectedApp();
    const user = this.selectedUser();
    const pwd = this.newPassword().trim();

    if (!activeApp || !user || !pwd) return;

    if (pwd.length < 6) {
      this.toast.error('Parola trebuie să aibă cel puțin 6 caractere.');
      return;
    }

    this.resettingPassword.set(true);
    this.authMgmtService.resetUserPassword(activeApp.id, user.appUserId, pwd).subscribe({
      next: () => {
        this.toast.success(`Parola utilizatorului "${user.fullName}" a fost resetată.`);
        this.showResetPasswordModal.set(false);
        this.resettingPassword.set(false);
        this.selectedUser.set(null);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la resetarea parolei.');
        this.resettingPassword.set(false);
      }
    });
  }

  onOpenRoleModal(user: AppUserWithRoles): void {
    this.selectedUser.set(user);
    this.newRoleName.set('');
    this.showRoleModal.set(true);
  }

  onAssignRoleSubmit(): void {
    const activeApp = this.parent.selectedApp();
    const user = this.selectedUser();
    const role = this.newRoleName().trim().toLowerCase();

    if (!activeApp || !user || !role) return;

    this.assigningRole.set(true);
    this.authMgmtService.assignUserRole(activeApp.id, user.email, role).subscribe({
      next: () => {
        this.toast.success(`Rolul "${role}" a fost alocat.`);
        this.newRoleName.set('');
        this.loadUsers();
        // Update local roles list to avoid reloading everything if possible
        const uIdx = this.users().findIndex(u => u.appUserId === user.appUserId);
        if (uIdx !== -1) {
          const updated = { ...this.users()[uIdx] };
          if (!updated.roles.includes(role)) {
            updated.roles = [...updated.roles, role];
            this.selectedUser.set(updated);
          }
        }
        this.assigningRole.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la alocarea rolului.');
        this.assigningRole.set(false);
      }
    });
  }

  async onRemoveRole(role: string): Promise<void> {
    const activeApp = this.parent.selectedApp();
    const user = this.selectedUser();
    if (!activeApp || !user || !role) return;

    const confirmed = await this.confirm.ask({
      title: 'Retragere Rol',
      message: `Sigur doriți să retrageți rolul "${role}" de la utilizatorul "${user.fullName}"?`,
      confirmText: 'Retrage rol',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.authMgmtService.removeUserRole(activeApp.id, {
      appUserId: user.appUserId,
      role
    }).subscribe({
      next: () => {
        this.toast.success(`Rolul "${role}" a fost retras.`);
        this.loadUsers();
        // Update local roles list
        const uIdx = this.users().findIndex(u => u.appUserId === user.appUserId);
        if (uIdx !== -1) {
          const updated = { ...this.users()[uIdx] };
          updated.roles = updated.roles.filter(r => r !== role);
          this.selectedUser.set(updated);
        }
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la retragerea rolului.');
      }
    });
  }

  // --- JSON Config ---
  loadAuthConfig(): void {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    this.loading.set(true);
    this.authMgmtService.getAuthConfig(activeApp.id).subscribe({
      next: (res) => {
        const hasKeys = res && Object.keys(res).length > 0;
        const defaultTemplate = {
          "admin": [
            "posts:create",
            "posts:update",
            "posts:delete",
            "posts:read"
          ],
          "user": [
            "posts:read"
          ]
        };
        const config = hasKeys ? res : defaultTemplate;
        this.configText.set(JSON.stringify(config, null, 2));
        this.rolesConfig.set(config);
        this.parseError.set(null);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea configurației.');
        this.loading.set(false);
      }
    });
  }

  onConfigChange(text: string): void {
    this.configText.set(text);
    try {
      if (text.trim() === '') {
        this.parseError.set(null);
        return;
      }
      const parsed = JSON.parse(text);
      this.rolesConfig.set(parsed);
      this.parseError.set(null);
    } catch (e: any) {
      this.parseError.set(e.message);
    }
  }

  onSaveConfig(): void {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    if (this.parseError()) {
      this.toast.error('Corectați erorile de sintaxă JSON înainte de salvare.');
      return;
    }

    let parsedConfig: any;
    try {
      parsedConfig = JSON.parse(this.configText().trim() || '{}');
    } catch {
      this.toast.error('Format JSON nevalid.');
      return;
    }

    this.savingConfig.set(true);
    this.authMgmtService.updateAuthConfig(activeApp.id, parsedConfig).subscribe({
      next: () => {
        this.toast.success('Configurația rolurilor a fost salvată cu succes.');
        this.rolesConfig.set(parsedConfig);
        this.savingConfig.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea configurației.');
        this.savingConfig.set(false);
      }
    });
  }

  updateConfigFromVisual(config: Record<string, string[]>): void {
    this.rolesConfig.set(config);
    this.configText.set(JSON.stringify(config, null, 2));
  }

  onAddVisualRole(): void {
    const roleName = this.newVisualRoleName().trim().toLowerCase();
    if (!roleName) return;
    const config = { ...this.rolesConfig() };
    if (config[roleName]) {
      this.toast.error('Acest rol există deja.');
      return;
    }
    config[roleName] = [];
    this.updateConfigFromVisual(config);
    this.newVisualRoleName.set('');
  }

  onRemoveVisualRole(roleName: string): void {
    const config = { ...this.rolesConfig() };
    delete config[roleName];
    this.updateConfigFromVisual(config);
  }

  onAddVisualPermission(roleName: string): void {
    const permMap = this.newVisualPermissionName();
    const permName = (permMap[roleName] || '').trim();
    if (!permName) return;
    const config = { ...this.rolesConfig() };
    if (!config[roleName]) return;
    if (config[roleName].includes(permName)) {
      this.toast.error('Această permisiune este deja asociată acestui rol.');
      return;
    }
    config[roleName] = [...config[roleName], permName];
    this.updateConfigFromVisual(config);
    this.newVisualPermissionName.set({
      ...permMap,
      [roleName]: ''
    });
  }

  onRemoveVisualPermission(roleName: string, permName: string): void {
    const config = { ...this.rolesConfig() };
    if (!config[roleName]) return;
    config[roleName] = config[roleName].filter(p => p !== permName);
    this.updateConfigFromVisual(config);
  }

  getRolesKeys(): string[] {
    return Object.keys(this.rolesConfig());
  }

  updateVisualPermissionName(roleName: string, value: string): void {
    this.newVisualPermissionName.set({
      ...this.newVisualPermissionName(),
      [roleName]: value
    });
  }

  // --- API Keys Management ---
  loadApiKeys(): void {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    this.loading.set(true);
    this.authMgmtService.listApiKeys(activeApp.id).subscribe({
      next: (res) => {
        this.apiKeys.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea cheilor API.');
        this.loading.set(false);
      }
    });
  }

  onCreateApiKey(): void {
    const activeApp = this.parent.selectedApp();
    const name = this.newKeyName().trim();
    if (!activeApp || !name) return;

    let expiresAtStr: string | null = null;
    if (this.newKeyExpiresAt()) {
      expiresAtStr = new Date(this.newKeyExpiresAt()).toISOString();
    }

    this.creatingKey.set(true);
    this.authMgmtService.createApiKey(activeApp.id, {
      name,
      expiresAt: expiresAtStr
    }).subscribe({
      next: (res) => {
        this.toast.success(`Cheia API "${name}" a fost generată.`);
        this.newKeyName.set('');
        this.newKeyExpiresAt.set('');
        this.creatingKey.set(false);
        this.generatedKeySecret.set(res.rawKey);
        this.showGeneratedKeyModal.set(true);
        this.loadApiKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la generarea cheii.');
        this.creatingKey.set(false);
      }
    });
  }

  async onDeleteApiKey(key: ApiKeyInfo): Promise<void> {
    const activeApp = this.parent.selectedApp();
    if (!activeApp) return;

    const confirmed = await this.confirm.ask({
      title: 'Revocare Cheie API',
      message: `Sigur doriți să revocați complet cheia API "${key.name}"? Serviciile externe ce folosesc această cheie vor pierde accesul instantaneu.`,
      confirmText: 'Revocă cheie',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.authMgmtService.deleteApiKey(activeApp.id, key.id).subscribe({
      next: () => {
        this.toast.success(`Cheia API "${key.name}" a fost revocată.`);
        this.loadApiKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la revocarea cheii.');
      }
    });
  }

  copyApiKeySecret(): void {
    const key = this.generatedKeySecret();
    if (!key) return;

    navigator.clipboard.writeText(key).then(() => {
      this.toast.success('Cheia API a fost copiată în clipboard!');
    });
  }
}
