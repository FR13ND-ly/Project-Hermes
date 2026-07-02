import { Component, inject, signal, effect, computed } from '@angular/core';
import { RouterLink, RouterOutlet, RouterLinkActive, ActivatedRoute, Router } from '@angular/router';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { AuthManagementService, AppUserWithRoles, ApiKeyInfo, CreateApiKeyResponse, AuthIntegration, BaasService } from '../../../../../../core/services/auth-management.service';
import { ProjectService } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-auth-management-detail',
  imports: [FormsModule, DatePipe, RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './auth-management-detail.html',
  styleUrl: './auth-management.css',
})
export class AuthManagementDetail {
  readonly parent = inject(Details);
  private readonly authMgmtService = inject(AuthManagementService);
  private readonly projectService = inject(ProjectService);
  readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);


  readonly activeTab = signal<'users' | 'roles' | 'api-keys' | 'integration'>('users');
  readonly loading = signal(false);
  readonly loadingService = signal(false);
  readonly error = signal<string | null>(null);

  // Standalone BaaS services for this project (no app required). The page operates on
  // the selected service; users/roles/api-keys/integration are all scoped to it.
  readonly services = signal<BaasService[]>([]);
  readonly selectedService = signal<BaasService | null>(null);
  readonly serviceId = signal<string | null>(null);
  readonly loadingServices = signal(false);

  // Integration tab
  readonly integration = signal<AuthIntegration | null>(null);
  readonly revealSecret = signal(false);
  readonly rotatingSecret = signal(false);
  readonly integrationSnippet = computed(() => {
    const i = this.integration();
    if (!i) return '';
    return `// Express — Hermes BaaS auth (local JWT validation with secret from env)
import jwt from 'jsonwebtoken';

const API_KEY = process.env.HERMES_APP_TOKEN;   // API key generated in the panel
const SECRET = process.env.${i.authSecretEnvKey};   // Secretul de semnare JWT
const BAAS_ID = '${i.baasId}';
const HERMES = '${i.apiBaseUrl}';

// login: identifier + password -> { accessToken, refreshToken }
// Send the API key in the Authorization header to authorize the call
// and inject custom claims (e.g. tenantId, plan) into the access token.
async function login(identifier, password, additionalClaims = {}) {
  const r = await fetch(\`\${HERMES}/baas/\${BAAS_ID}/auth/login\`, {
    method: 'POST',
    headers: { 
      'Content-Type': 'application/json',
      'Authorization': \`Bearer \${API_KEY}\`
    },
    body: JSON.stringify({ identifier, password, additionalClaims })
  });
  return r.json(); // { accessToken, refreshToken, expiresIn, roles, permissions }
}

// access token is short-lived; renew it with the refresh token (single-use)
async function refresh(refreshToken) {
  const r = await fetch(\`\${HERMES}/baas/\${BAAS_ID}/auth/refresh\`, {
    method: 'POST',
    headers: { 
      'Content-Type': 'application/json',
      'Authorization': \`Bearer \${API_KEY}\`
    },
    body: JSON.stringify({ refreshToken })
  });
  return r.json(); // { accessToken, refreshToken, ... }
}

// protect routes: local verification, zero calls to Hermes
export function requireUser(req, res, next) {
  const token = (req.headers.authorization || '').replace('Bearer ', '');
  try {
    req.user = jwt.verify(token, SECRET); // { sub, identifier, roles, permissions, ...custom }
    next();
  } catch {
    res.status(401).json({ error: 'unauthorized' });
  }
}

export const requireRole = (role) => (req, res, next) =>
  req.user?.roles?.includes(role) ? next() : res.status(403).end();`;
  });

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
    this.route.paramMap.subscribe(params => {
      this.serviceId.set(params.get('serviceId'));
    });

    // React to both projectId and serviceId changes
    effect(() => {
      const projectId = this.parent.projectId();
      const sId = this.serviceId();
      if (projectId && sId) {
        this.loadServiceDetail(sId);
      }
    });

    // Reload active service data when the selected service changes
    effect(() => {
      const activeApp = this.selectedService();
      if (activeApp) {
        this.currentPage.set(1);
        this.searchQuery.set('');
        // Child route components will load their own data on init
      } else {
        this.users.set([]);
        this.apiKeys.set([]);
        this.totalUsers.set(0);
        this.totalPages.set(0);
      }
    });
  }

  loadServiceDetail(serviceId: string): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingService.set(true);
    this.authMgmtService.listServices(projectId).subscribe({
      next: (list) => {
        const svc = (list || []).find(s => s.id === serviceId);
        if (svc) {
          this.selectedService.set(svc);
        } else {
          this.toast.error('Authentication service not found.');
          this.backToList();
        }
        this.loadingService.set(false);
      },
      error: () => {
        this.loadingService.set(false);
        this.toast.error('Failed to load authentication service.');
      }
    });
  }

  /** Return to the service list (deselect the active service). */
  backToList(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'auth-management']);
  }

  async onDeleteService(svc: BaasService): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Authentication Service',
      message: `Are you sure you want to delete "${svc.name}"? All associated users, roles, and API keys will be permanently deleted.`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;
    this.authMgmtService.deleteService(svc.id).subscribe({
      next: () => {
        this.toast.success(`Service "${svc.name}" deleted.`);
        this.backToList();
      },
      error: (err) => this.toast.error(err.error?.message || 'Failed to delete service.')
    });
  }

  loadActiveTabData(): void {
    const activeApp = this.selectedService();
    if (!activeApp) return;

    const tab = this.activeTab();
    if (tab === 'users') {
      this.loadUsers();
    } else if (tab === 'roles') {
      this.loadAuthConfig();
    } else if (tab === 'api-keys') {
      this.loadApiKeys();
    } else if (tab === 'integration') {
      this.loadIntegration();
    }
  }

  // --- Integration ---
  loadIntegration(): void {
    const activeApp = this.selectedService();
    if (!activeApp) return;
    this.loading.set(true);
    this.revealSecret.set(false);
    this.authMgmtService.getIntegration(activeApp.id).subscribe({
      next: (res) => {
        this.integration.set(res);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to load integration data.');
        this.loading.set(false);
      }
    });
  }

  async rotateAuthSecret(): Promise<void> {
    const activeApp = this.selectedService();
    if (!activeApp) return;

    const confirmed = await this.confirm.ask({
      title: 'Rotate BaaS Secret',
      message: 'A new signing secret will be generated. All existing end-user tokens become invalid (users must re-authenticate), and the application picks up the new secret on the next reload. Continue?',
      confirmText: 'Rotate',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.rotatingSecret.set(true);
    this.authMgmtService.rotateAuthSecret(activeApp.id).subscribe({
      next: (res) => {
        this.rotatingSecret.set(false);
        this.integration.update(i => i ? { ...i, authSecret: res.auth_secret } : i);
        this.revealSecret.set(true);
        this.toast.success('Secret rotated. Existing end-user tokens are now invalid; reload the application to pick up the new secret.');
      },
      error: (err) => {
        this.rotatingSecret.set(false);
        this.toast.error(err.error?.message || 'Failed to rotate secret.');
      }
    });
  }

  copyText(text: string, label = 'Value'): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success(`${label} copied to clipboard!`);
    });
  }

  // --- Users Management ---
  loadUsers(): void {
    const activeApp = this.selectedService();
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
        this.error.set(err.error?.message || 'Failed to load users.');
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
    const activeApp = this.selectedService();
    if (!activeApp) return;

    const nextStatus = user.status === 'active' ? 'suspended' : 'active';
    const actionText = nextStatus === 'suspended' ? 'suspend' : 'reactivate';

    const confirmed = await this.confirm.ask({
      title: `${nextStatus === 'suspended' ? 'Suspendare' : 'Reactivare'} Cont`,
      message: `Are you sure you want to ${nextStatus === 'suspended' ? 'suspend' : 'reactivate'} the user "${user.identifier}"? Suspended accounts can no longer authenticate in the application.`,
      confirmText: nextStatus === 'suspended' ? 'Suspend account' : 'Activate account',
      cancelText: 'Cancel',
      isDanger: nextStatus === 'suspended'
    });
    if (!confirmed) return;

    this.authMgmtService.updateUserStatus(activeApp.id, user.appUserId, nextStatus).subscribe({
      next: () => {
        this.toast.success(`Account was successfully ${nextStatus === 'suspended' ? 'suspended' : 'activated'}.`);
        this.loadUsers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error updating account status.');
      }
    });
  }

  onOpenResetPassword(user: AppUserWithRoles): void {
    this.selectedUser.set(user);
    this.newPassword.set('');
    this.showResetPasswordModal.set(true);
  }

  onResetPasswordSubmit(): void {
    const activeApp = this.selectedService();
    const user = this.selectedUser();
    const pwd = this.newPassword().trim();

    if (!activeApp || !user || !pwd) return;

    if (pwd.length < 8) {
      this.toast.error('Password must be at least 8 characters.');
      return;
    }

    this.resettingPassword.set(true);
    this.authMgmtService.resetUserPassword(activeApp.id, user.appUserId, pwd).subscribe({
      next: () => {
        this.toast.success(`Password for "${user.identifier}" has been reset.`);
        this.showResetPasswordModal.set(false);
        this.resettingPassword.set(false);
        this.selectedUser.set(null);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error resetting password.');
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
    const activeApp = this.selectedService();
    const user = this.selectedUser();
    const role = this.newRoleName().trim().toLowerCase();

    if (!activeApp || !user || !role) return;

    this.assigningRole.set(true);
    this.authMgmtService.assignUserRole(activeApp.id, user.identifier, role).subscribe({
      next: () => {
        this.toast.success(`Role "${role}" has been assigned.`);
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
        this.toast.error(err.error?.message || 'Error assigning role.');
        this.assigningRole.set(false);
      }
    });
  }

  async onRemoveRole(role: string): Promise<void> {
    const activeApp = this.selectedService();
    const user = this.selectedUser();
    if (!activeApp || !user || !role) return;

    const confirmed = await this.confirm.ask({
      title: 'Revoke Role',
      message: `Are you sure you want to revoke the role "${role}" from user "${user.identifier}"?`,
      confirmText: 'Revoke role',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.authMgmtService.removeUserRole(activeApp.id, {
      appUserId: user.appUserId,
      role
    }).subscribe({
      next: () => {
        this.toast.success(`Role "${role}" has been revoked.`);
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
        this.toast.error(err.error?.message || 'Error revoking role.');
      }
    });
  }

  // --- JSON Config ---
  loadAuthConfig(): void {
    const activeApp = this.selectedService();
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
        this.toast.error(err.error?.message || 'Failed to load configuration.');
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
    const activeApp = this.selectedService();
    if (!activeApp) return;

    if (this.parseError()) {
      this.toast.error('Fix JSON syntax errors before saving.');
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
        this.toast.success('Roles configuration saved successfully.');
        this.rolesConfig.set(parsedConfig);
        this.savingConfig.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to save configuration.');
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
      this.toast.error('This role already exists.');
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
      this.toast.error('This permission is already associated with this role.');
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
    const activeApp = this.selectedService();
    if (!activeApp) return;

    this.loading.set(true);
    this.authMgmtService.listApiKeys(activeApp.id).subscribe({
      next: (res) => {
        this.apiKeys.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to load API keys.');
        this.loading.set(false);
      }
    });
  }

  onCreateApiKey(): void {
    const activeApp = this.selectedService();
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
        this.toast.success(`API key "${name}" generated.`);
        this.newKeyName.set('');
        this.newKeyExpiresAt.set('');
        this.creatingKey.set(false);
        this.generatedKeySecret.set(res.rawKey);
        this.showGeneratedKeyModal.set(true);
        this.loadApiKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error generating key.');
        this.creatingKey.set(false);
      }
    });
  }

  async onDeleteApiKey(key: ApiKeyInfo): Promise<void> {
    const activeApp = this.selectedService();
    if (!activeApp) return;

    const confirmed = await this.confirm.ask({
      title: 'Revoke API Key',
      message: `Are you sure you want to fully revoke the API key "${key.name}"? External services using this key will lose access immediately.`,
      confirmText: 'Revoke key',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.authMgmtService.deleteApiKey(activeApp.id, key.id).subscribe({
      next: () => {
        this.toast.success(`API key "${key.name}" revoked.`);
        this.loadApiKeys();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error revoking key.');
      }
    });
  }

  copyApiKeySecret(): void {
    const key = this.generatedKeySecret();
    if (!key) return;

    navigator.clipboard.writeText(key).then(() => {
      this.toast.success('API key copied to clipboard!');
    });
  }

}
