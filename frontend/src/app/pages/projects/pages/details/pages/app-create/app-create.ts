import { Component, inject, signal, OnInit, OnDestroy, computed } from '@angular/core';
import { ActivatedRoute, RouterLink, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { NgClass } from '@angular/common';
import { ProjectService, EnvVarInput, ProjectEnvResponse, ComposePlan } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { GitService, GitCredential, GitRepo } from '../../../../../../core/services/git.service';
import { EnvLinkModal } from '../../../../../../shared/components/env-link-modal/env-link-modal';
import { Details } from '../../details';

@Component({
  selector: 'app-app-create',
  imports: [RouterLink, FormsModule, EnvLinkModal, NgClass],
  templateUrl: './app-create.html',
  styleUrl: './app-create.css',
})
export class AppCreate implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly gitService = inject(GitService);

  readonly projectId = computed(() => this.parent.projectId());

  // Wizard States
  readonly deployingApp = signal(false);
  readonly appName = signal('');
  readonly gitRepository = signal('');
  readonly branchName = signal('main');
  readonly buildCommand = signal('');
  readonly startCommand = signal('');
  readonly internalPort = signal<number>(8080);
  readonly externalPort = signal<number | null>(null);
  readonly gitSubpath = signal('');
  readonly networkName = signal('');
  readonly publishUrl = signal(false);
  readonly urlEnvKey = signal('');
  readonly detectedSubdirectories = signal<string[]>([]);
  readonly subpathSelectionMode = signal<'select' | 'custom'>('select');
  readonly selectedSubpathOption = signal('');
  private detectionTimeout: any = null;

  // Environment variables provided at app-creation time
  readonly newAppEnvRows = signal<EnvVarInput[]>([]);

  addEnvRow(): void {
    this.newAppEnvRows.update(rows => [...rows, { key: '', value: '', isSecret: false }]);
  }

  removeEnvRow(index: number): void {
    this.newAppEnvRows.update(rows => rows.filter((_, i) => i !== index));
  }

  updateEnvRow(index: number, field: 'key' | 'value', value: string): void {
    this.newAppEnvRows.update(rows =>
      rows.map((row, i) => (i === index ? { ...row, [field]: value } : row))
    );
  }

  toggleEnvRowSecret(index: number): void {
    this.newAppEnvRows.update(rows =>
      rows.map((row, i) => (i === index ? { ...row, isSecret: !row.isSecret } : row))
    );
  }

  // Project environment variables linking
  readonly projectEnvPool = signal<ProjectEnvResponse[]>([]);
  readonly selectedProjectEnvIds = signal<string[]>([]);
  readonly showCreateEnvModal = signal(false);

  readonly projectEnvForModal = computed<ProjectEnvResponse[]>(() => {
    const selected = this.selectedProjectEnvIds();
    return this.projectEnvPool().map(v => ({ ...v, linked: selected.includes(v.id) }));
  });

  openCreateEnvModal(): void {
    const projectId = this.projectId();
    if (projectId) {
      this.projectService.listProjectEnv(projectId).subscribe({
        next: (res) => this.projectEnvPool.set(res || []),
        error: () => this.projectEnvPool.set([])
      });
    }
    this.showCreateEnvModal.set(true);
  }

  toggleCreateEnvSelection(env: ProjectEnvResponse): void {
    this.selectedProjectEnvIds.update(ids =>
      ids.includes(env.id) ? ids.filter(id => id !== env.id) : [...ids, env.id]
    );
  }

  // JSON editor for environment variables
  readonly newAppEnvJsonMode = signal(false);
  readonly newAppEnvJsonText = signal('');

  openNewAppEnvJson(): void {
    const obj: Record<string, string> = {};
    for (const row of this.newAppEnvRows()) {
      const key = row.key.trim();
      if (key) obj[key] = row.value;
    }
    this.newAppEnvJsonText.set(JSON.stringify(obj, null, 2));
    this.newAppEnvJsonMode.set(true);
  }

  applyNewAppEnvJson(): void {
    let parsed: any;
    try {
      parsed = JSON.parse(this.newAppEnvJsonText() || '{}');
    } catch {
      this.toast.error('Invalid JSON. Check the syntax.');
      return;
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      this.toast.error('JSON must be an object { "KEY": "value" }.');
      return;
    }
    const rows: EnvVarInput[] = Object.entries(parsed).map(([key, value]) => ({
      key,
      value: value == null ? '' : String(value),
      isSecret: false
    }));
    this.newAppEnvRows.set(rows);
    this.newAppEnvJsonMode.set(false);
  }

  // Git credentials and repositories
  readonly credentials = signal<GitCredential[]>([]);
  readonly selectedCredentialId = signal<string | null>(null);
  readonly repos = signal<GitRepo[]>([]);
  readonly loadingRepos = signal(false);
  readonly branches = signal<string[]>([]);
  readonly loadingBranches = signal(false);

  // Auto-detection signals
  readonly detectedType = signal<string>('');
  readonly detectingType = signal(false);
  readonly detectedDescription = signal<string>('');

  readonly repoSearchQuery = signal('');
  readonly selectedImportRepo = signal<GitRepo | null>(null);

  // Create-app wizard: 1 = Repo, 2 = Settings, 3 = Env
  readonly createStep = signal(1);
  nextCreateStep(): void { this.createStep.update(s => Math.min(3, s + 1)); }
  prevCreateStep(): void { this.createStep.update(s => Math.max(1, s - 1)); }
  readonly isCustomGitUrl = signal(false);

  readonly filteredRepos = computed(() => {
    const query = this.repoSearchQuery().toLowerCase().trim();
    if (!query) return this.repos();
    return this.repos().filter(r => r.name.toLowerCase().includes(query) || r.fullPath.toLowerCase().includes(query));
  });

  readonly selectedCredential = computed(() =>
    this.credentials().find(c => c.id === this.selectedCredentialId()) || null);

  loadCredentials(): void {
    this.gitService.listCredentials().subscribe({
      next: (creds) => {
        this.credentials.set(creds || []);
        if (!this.selectedCredentialId() && creds.length > 0) {
          this.selectedCredentialId.set(creds[0].id);
          this.loadRepos();
        }
      },
      error: () => this.credentials.set([])
    });
  }

  onSelectCredential(credentialId: string): void {
    this.selectedCredentialId.set(credentialId);
    this.repos.set([]);
    this.selectedImportRepo.set(null);
    this.loadRepos();
  }

  loadRepos(): void {
    const credId = this.selectedCredentialId();
    if (!credId) return;
    this.loadingRepos.set(true);
    this.gitService.listRepos(credId).subscribe({
      next: (repos) => {
        this.repos.set(repos || []);
        this.loadingRepos.set(false);
      },
      error: () => {
        this.toast.error('Failed to load repositories.');
        this.loadingRepos.set(false);
      }
    });
  }

  loadBranches(repo: string): void {
    const credId = this.selectedCredentialId();
    if (!credId) return;
    this.loadingBranches.set(true);
    this.branches.set([]);
    this.gitService.listBranches(credId, repo).subscribe({
      next: (branches) => {
        this.branches.set(branches);
        this.loadingBranches.set(false);
        if (branches.length > 0) {
          if (branches.includes('main')) this.branchName.set('main');
          else if (branches.includes('master')) this.branchName.set('master');
          else this.branchName.set(branches[0]);
        }
      },
      error: () => {
        this.toast.error('Failed to load branches.');
        this.loadingBranches.set(false);
      }
    });
  }

  onImportRepo(repo: GitRepo): void {
    this.selectedImportRepo.set(repo);
    this.createStep.set(1);
    this.isCustomGitUrl.set(false);
    this.appName.set(repo.name);
    this.gitRepository.set(repo.htmlUrl || '');
    this.branchName.set(repo.defaultBranch || 'main');

    this.internalPort.set(8080);
    this.externalPort.set(null);
    this.gitSubpath.set('');
    this.detectedSubdirectories.set([]);
    this.subpathSelectionMode.set('select');
    this.selectedSubpathOption.set('');
    this.buildCommand.set('');
    this.startCommand.set('');
    this.detectedType.set('');
    this.detectedDescription.set('');

    this.loadBranches(repo.fullPath);
    this.triggerAutoDetection();
    this.checkCompose(repo.fullPath);
  }

  onBranchChange(newBranch: string): void {
    this.branchName.set(newBranch);
    const repo = this.selectedImportRepo();
    if (repo) {
      this.checkCompose(repo.fullPath);
      this.triggerAutoDetection();
    }
  }

  // docker-compose auto-split
  readonly composeDetected = signal(false);
  readonly composeYaml = signal('');
  readonly composeFilename = signal<string | null>(null);
  readonly composePlan = signal<ComposePlan | null>(null);
  readonly showComposePreview = signal(false);
  readonly planningCompose = signal(false);
  readonly applyingCompose = signal(false);

  checkCompose(repo: string): void {
    const credId = this.selectedCredentialId();
    if (!credId) return;
    this.composeDetected.set(false);
    this.composePlan.set(null);
    this.gitService.getCompose(credId, repo, this.gitSubpath() || undefined, this.branchName() || undefined).subscribe({
      next: (res) => {
        if (res.found) {
          this.composeDetected.set(true);
          this.composeYaml.set(res.yaml);
          this.composeFilename.set(res.filename || 'docker-compose.yml');
        }
      },
      error: () => this.composeDetected.set(false)
    });
  }

  openComposeSplit(): void {
    if (!this.composeYaml()) return;
    this.planningCompose.set(true);
    this.projectService.planComposeSplit(this.composeYaml()).subscribe({
      next: (plan) => {
        if (plan && plan.databases) {
          plan.databases = plan.databases.map(d => ({
            ...d,
            publishToEnv: false,
            envKey: d.envKey || ''
          }));
        }
        this.composePlan.set(plan);
        this.showComposePreview.set(true);
        this.planningCompose.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to parse docker-compose.');
        this.planningCompose.set(false);
      }
    });
  }

  readonly expandedApps = signal<Record<number, boolean>>({});
  readonly expandedDbs = signal<Record<number, boolean>>({});
  toggleAppExpand(i: number): void { this.expandedApps.update(m => ({ ...m, [i]: !m[i] })); }
  toggleDbExpand(i: number): void { this.expandedDbs.update(m => ({ ...m, [i]: !m[i] })); }
  
  readonly editingAppIndex = signal<number | null>(null);
  readonly editorTab = signal<'general' | 'network' | 'env'>('general');
  openAppEditor(i: number): void { this.editorTab.set('general'); this.editingAppIndex.set(i); }
  closeAppEditor(): void { this.editingAppIndex.set(null); }

  toggleComposeApp(index: number): void {
    this.composePlan.update(p => {
      if (!p) return p;
      const apps = p.apps.map((a, i) => i === index ? { ...a, include: !a.include } : a);
      return { ...p, apps };
    });
  }

  toggleComposeDb(index: number): void {
    this.composePlan.update(p => {
      if (!p) return p;
      const databases = p.databases.map((d, i) => i === index ? { ...d, include: !d.include } : d);
      return { ...p, databases };
    });
  }

  updateAppName(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === i ? { ...a, name: v } : a) } : p);
  }
  updateAppInternalPort(i: number, v: number): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === i ? { ...a, internalPort: +v || 0 } : a) } : p);
  }
  updateAppExternalPort(i: number, v: string): void {
    const port = v === '' || v === null ? null : (+v || null);
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === i ? { ...a, externalPort: port } : a) } : p);
  }
  addAppEnv(appIdx: number): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === appIdx ? { ...a, env: [...a.env, { key: '', value: '' }] } : a) } : p);
  }
  removeAppEnv(appIdx: number, envIdx: number): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === appIdx ? { ...a, env: a.env.filter((_, j) => j !== envIdx) } : a) } : p);
  }
  updateAppEnvKey(appIdx: number, envIdx: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === appIdx ? { ...a, env: a.env.map((e, j) => j === envIdx ? { ...e, key: v.toUpperCase() } : e) } : a) } : p);
  }
  updateAppEnvValue(appIdx: number, envIdx: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === appIdx ? { ...a, env: a.env.map((e, j) => j === envIdx ? { ...e, value: v } : e) } : a) } : p);
  }
  updateAppNetworkName(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === i ? { ...a, networkName: v } : a) } : p);
  }
  updateAppUrlEnvKey(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === i ? { ...a, urlEnvKey: v.toUpperCase() } : a) } : p);
  }
  toggleComposeAppPublishUrl(i: number): void {
    this.composePlan.update(p => p ? { ...p, apps: p.apps.map((a, idx) => idx === i ? { ...a, publishUrl: a.publishUrl === false } : a) } : p);
  }

  defaultUrlKey(app: { service: string }): string {
    const base = (app.service || 'APP').toUpperCase().replace(/[^A-Z0-9]/g, '_').replace(/^_+|_+$/g, '');
    return (base || 'APP') + '_URL';
  }

  updateDbName(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, name: v } : d) } : p);
  }
  updateDbVersion(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, version: v } : d) } : p);
  }
  updateDbInternalPort(i: number, v: number): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, internalPort: +v || 0 } : d) } : p);
  }
  updateDbPublishToEnv(i: number, v: boolean): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, publishToEnv: v } : d) } : p);
  }
  updateDbEnvKey(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, envKey: v } : d) } : p);
  }

  composeSelectedCount(): number {
    const p = this.composePlan();
    if (!p) return 0;
    return p.apps.filter(a => a.include).length + p.databases.filter(d => d.include).length;
  }

  onApplyComposeSplit(): void {
    const projectId = this.projectId();
    const plan = this.composePlan();
    if (!projectId || !plan) return;
    this.applyingCompose.set(true);
    this.projectService.applyComposeSplit({
      projectId,
      gitRepository: this.gitRepository() || undefined,
      gitCredentialId: this.selectedCredentialId() || undefined,
      branchName: this.branchName() || 'main',
      plan
    }).subscribe({
      next: () => {
        this.applyingCompose.set(false);
        this.showComposePreview.set(false);
        this.toast.success('Stack created from docker-compose (apps, databases, volumes).');
        this.parent.loadDetails(projectId);
        this.router.navigate(['/projects', projectId, 'apps']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to create the stack.');
        this.applyingCompose.set(false);
      }
    });
  }

  onUseCustomGitUrl(): void {
    this.selectedImportRepo.set(null);
    this.createStep.set(1);
    this.isCustomGitUrl.set(true);
    this.appName.set('');
    this.gitRepository.set('');
    this.branchName.set('main');
    this.internalPort.set(8080);
    this.externalPort.set(null);
    this.gitSubpath.set('');
    this.buildCommand.set('');
    this.startCommand.set('');
    this.branches.set([]);
  }

  onBackToSelection(): void {
    this.selectedImportRepo.set(null);
    this.isCustomGitUrl.set(false);
    this.createStep.set(1);
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.projectId(), 'apps']);
  }

  ngOnInit(): void {
    this.loadCredentials();
  }

  ngOnDestroy(): void {
    if (this.detectionTimeout) {
      clearTimeout(this.detectionTimeout);
    }
  }

  onDeployApp(): void {
    if (!this.appName() || !this.gitRepository()) {
      this.toast.error('Application name and Git repository are required.');
      return;
    }

    const projectId = this.projectId();
    if (!projectId) return;

    const envVariables = this.newAppEnvRows()
      .map(row => ({ key: row.key.trim(), value: row.value, isSecret: row.isSecret }))
      .filter(row => row.key.length > 0);

    this.deployingApp.set(true);
    this.projectService.createApp({
      projectId,
      name: this.appName(),
      gitRepository: this.gitRepository(),
      branchName: this.branchName() || undefined,
      buildCommand: this.buildCommand() || undefined,
      startCommand: this.startCommand() || undefined,
      internalPort: this.internalPort() || undefined,
      externalPort: this.externalPort() || undefined,
      gitSubpath: this.gitSubpath() || undefined,
      gitCredentialId: this.selectedCredentialId() || undefined,
      envVariables: envVariables.length > 0 ? envVariables : undefined,
      linkedProjectEnvIds: this.selectedProjectEnvIds().length > 0 ? this.selectedProjectEnvIds() : undefined,
      networkName: this.networkName().trim() || undefined,
      publishUrl: this.publishUrl(),
      urlEnvKey: this.urlEnvKey().trim() || undefined
    }).subscribe({
      next: (res) => {
        this.deployingApp.set(false);
        this.appName.set('');
        this.gitRepository.set('');
        this.branchName.set('main');
        this.buildCommand.set('');
        this.startCommand.set('');
        this.internalPort.set(8080);
        this.externalPort.set(null);
        this.gitSubpath.set('');
        this.newAppEnvRows.set([]);
        this.selectedProjectEnvIds.set([]);
        this.newAppEnvJsonMode.set(false);
        this.networkName.set('');
        this.publishUrl.set(false);
        this.urlEnvKey.set('');
        this.selectedImportRepo.set(null);
        this.isCustomGitUrl.set(false);
        this.toast.success('Application successfully registered for deployment!');
        
        this.parent.loadDetails(projectId);
        if (res && res.id) {
          this.router.navigate(['/projects', projectId, 'apps', res.id]);
        } else {
          this.router.navigate(['/projects', projectId, 'apps']);
        }
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to create application.');
        this.deployingApp.set(false);
      }
    });
  }

  triggerAutoDetection(): void {
    const repo = this.selectedImportRepo();
    if (!repo) return;

    this.detectingType.set(true);
    this.detectedType.set('');
    this.detectedDescription.set('');

    const credId = this.selectedCredentialId();
    if (!credId) { this.detectingType.set(false); return; }
    const subpathVal = this.gitSubpath() ? this.gitSubpath().trim() : undefined;

    this.gitService.detect(credId, repo.fullPath, subpathVal, this.branchName() || undefined).subscribe({
      next: (res) => {
        this.detectingType.set(false);
        this.detectedType.set(res.projectType);
        this.detectedDescription.set(res.description);
        this.internalPort.set(res.internalPort);
        this.buildCommand.set(res.buildCommand);
        this.startCommand.set(res.startCommand);

        if (res.subdirectories && res.subdirectories.length > 0) {
          this.detectedSubdirectories.set(res.subdirectories);
          if (subpathVal && res.subdirectories.includes(subpathVal)) {
            this.selectedSubpathOption.set(subpathVal);
            this.subpathSelectionMode.set('select');
          } else if (!subpathVal) {
            this.selectedSubpathOption.set('');
            this.subpathSelectionMode.set('select');
          }
        }

        if (res.detectedEnvs && res.detectedEnvs.length > 0) {
          const rows: EnvVarInput[] = res.detectedEnvs.map((env: any) => ({
            key: env.key,
            value: env.value,
            isSecret: false
          }));
          this.newAppEnvRows.set(rows);
        } else {
          this.newAppEnvRows.set([]);
        }

        this.toast.success(`Project type detected: ${res.projectType.toUpperCase()}`);
      },
      error: () => {
        this.detectingType.set(false);
        this.detectedType.set('generic');
        this.detectedDescription.set('Unknown project type or invalid subdirectory.');
        this.toast.error('Could not detect project type in subdirectory.');
      }
    });
  }

  onSubpathSelectChange(value: string): void {
    this.selectedSubpathOption.set(value);
    if (value === 'custom') {
      this.subpathSelectionMode.set('custom');
      this.gitSubpath.set('');
    } else {
      this.subpathSelectionMode.set('select');
      this.gitSubpath.set(value);
      this.triggerAutoDetection();
    }
  }

  onSubpathInputChange(value: string): void {
    this.gitSubpath.set(value);
    
    if (this.detectionTimeout) {
      clearTimeout(this.detectionTimeout);
    }
    
    this.detectionTimeout = setTimeout(() => {
      this.triggerAutoDetection();
    }, 600);
  }

  onAppNameChange(value: string): void {
    this.appName.set(value);
    if (value.includes(':')) {
      const parts = value.split(':');
      if (parts.length > 1) {
        const sub = parts[1].trim();
        if (sub && !this.gitSubpath()) {
          this.gitSubpath.set(sub);
          if (this.detectedSubdirectories().includes(sub)) {
            this.selectedSubpathOption.set(sub);
            this.subpathSelectionMode.set('select');
          } else {
            this.selectedSubpathOption.set('custom');
            this.subpathSelectionMode.set('custom');
          }
          this.triggerAutoDetection();
        }
      }
    }
  }
}
