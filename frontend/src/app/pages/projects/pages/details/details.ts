import { Component, inject, signal, OnInit, OnDestroy, computed } from '@angular/core';
import { ActivatedRoute, RouterLink, RouterOutlet, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { forkJoin, Subscription } from 'rxjs';
import { ProjectService, AppDetail, Project, EnvVarInput, ProjectEnvResponse, ComposePlan } from '../../../../core/services/project.service';
import { ToastService } from '../../../../core/services/toast.service';
import { AuthService } from '../../../../core/services/auth';
import { GitService, GitCredential, GitRepo } from '../../../../core/services/git.service';
import { EnvLinkModal } from '../../../../shared/components/env-link-modal/env-link-modal';
import { WebSocketService } from '../../../../core/services/websocket.service';

@Component({
  selector: 'app-details',
  imports: [RouterLink, RouterOutlet, FormsModule, EnvLinkModal],
  templateUrl: './details.html',
  styleUrl: './details.css',
})
export class Details implements OnInit, OnDestroy {
  private readonly route = inject(ActivatedRoute);
  private readonly projectService = inject(ProjectService);
  readonly router = inject(Router);
  readonly toast = inject(ToastService);
  readonly authService = inject(AuthService);
  private readonly gitService = inject(GitService);
  private readonly wsService = inject(WebSocketService);

  private refreshInterval: any = null;
  private sub = new Subscription();

  readonly projectId = signal<string | null>(null);
  readonly project = signal<Project | null>(null);
  readonly apps = signal<AppDetail[]>([]);
  readonly selectedApp = signal<AppDetail | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Computed wrapper so child components continue to function on parent.appDetail()
  readonly appDetail = computed(() => this.selectedApp());

  // Inline App Deployment States
  readonly showAddAppForm = signal(false);
  readonly deployingApp = signal(false);

  readonly appName = signal('');
  readonly gitRepository = signal('');
  readonly branchName = signal('main');
  readonly buildCommand = signal('');
  readonly startCommand = signal('');
  readonly internalPort = signal<number>(8080);
  readonly externalPort = signal<number | null>(null);
  readonly gitSubpath = signal('');
  readonly enableBaas = signal(false);
  readonly detectedSubdirectories = signal<string[]>([]);
  readonly subpathSelectionMode = signal<'select' | 'custom'>('select');
  readonly selectedSubpathOption = signal('');
  private detectionTimeout: any = null;

  // Environment variables provided at app-creation time
  readonly newAppEnvRows = signal<EnvVarInput[]>([]);

  addEnvRow(): void {
    this.newAppEnvRows.update(rows => [...rows, { key: '', value: '', isSecret: true }]);
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

  // --- Create-wizard: project-pool links chosen before the instance exists ---
  readonly projectEnvPool = signal<ProjectEnvResponse[]>([]);
  readonly selectedProjectEnvIds = signal<string[]>([]);
  readonly showCreateEnvModal = signal(false);

  // Pool vars decorated with a `linked` flag reflecting the local selection, so
  // the shared modal can render them with the same toggle UI as app-detail.
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

  // --- Create-wizard: JSON editor for the simple key/value rows ---
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
      this.toast.error('JSON invalid. Verifică sintaxa.');
      return;
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      this.toast.error('JSON-ul trebuie să fie un obiect { "CHEIE": "valoare" }.');
      return;
    }
    const rows: EnvVarInput[] = Object.entries(parsed).map(([key, value]) => ({
      key,
      value: value == null ? '' : String(value),
      isSecret: true
    }));
    this.newAppEnvRows.set(rows);
    this.newAppEnvJsonMode.set(false);
  }

  // Git credentials (workspace PATs) + repo browser state
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

  // Create-app wizard: 1 = Repo, 2 = Setări, 3 = Env
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
        // Default to the first credential if none picked yet.
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
        this.toast.error('Eroare la încărcarea repository-urilor.');
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
        this.toast.error('Eroare la încărcarea branch-urilor.');
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

  // --- docker-compose auto-split ---
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
        this.composePlan.set(plan);
        this.showComposePreview.set(true);
        this.planningCompose.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Nu am putut analiza docker-compose.');
        this.planningCompose.set(false);
      }
    });
  }

  // Which app/db rows are expanded for editing (by index).
  readonly expandedApps = signal<Record<number, boolean>>({});
  readonly expandedDbs = signal<Record<number, boolean>>({});
  toggleAppExpand(i: number): void { this.expandedApps.update(m => ({ ...m, [i]: !m[i] })); }
  toggleDbExpand(i: number): void { this.expandedDbs.update(m => ({ ...m, [i]: !m[i] })); }

  toggleComposeApp(index: number): void {
    this.composePlan.update(p => {
      if (!p) return p;
      const apps = p.apps.map((a, i) => i === index ? { ...a, include: !a.include } : a);
      return { ...p, apps };
    });
  }

  toggleComposeAppBaas(index: number): void {
    this.composePlan.update(p => {
      if (!p) return p;
      const apps = p.apps.map((a, i) => i === index ? { ...a, enableBaas: !a.enableBaas } : a);
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

  // --- Edit app fields in the plan before applying ---
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

  // --- Edit database fields in the plan before applying ---
  updateDbName(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, name: v } : d) } : p);
  }
  updateDbVersion(i: number, v: string): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, version: v } : d) } : p);
  }
  updateDbInternalPort(i: number, v: number): void {
    this.composePlan.update(p => p ? { ...p, databases: p.databases.map((d, idx) => idx === i ? { ...d, internalPort: +v || 0 } : d) } : p);
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
        this.showAddAppForm.set(false);
        this.toast.success('Stack-ul a fost creat din docker-compose (aplicații, baze de date, volume).');
        this.loadDetails(projectId);
        this.router.navigate(['/projects', projectId, 'apps']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea stack-ului.');
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

  ngOnInit(): void {
    this.route.paramMap.subscribe(params => {
      const id = params.get('id');
      if (id) {
        this.projectId.set(id);
        this.loadDetails(id);
        this.startPolling(id);
      }
    });
    this.loadCredentials();

    // Real-time WebSocket subscriptions
    const events = ['instance_status_changed', 'build_status_changed', 'database_status_changed', 'serverless_function_updated'];
    for (const evt of events) {
      this.sub.add(
        this.wsService.onEvent<any>(evt).subscribe(() => {
          const pid = this.projectId();
          if (pid) {
            this.loadDetails(pid, true);
          }
        })
      );
    }
  }

  // Reactivity comes from the WebSocket subscriptions in ngOnInit (instance /
  // build / database / serverless events refresh instantly). This slow poll is
  // only a reconciliation safety-net in case a WS event is missed (reconnect,
  // backend restart) — hence 30s rather than a tight loop.
  private startPolling(id: string): void {
    if (this.refreshInterval) {
      clearInterval(this.refreshInterval);
    }
    this.refreshInterval = setInterval(() => {
      if (this.projectId() && !this.loading() && !this.deployingApp()) {
        this.loadDetails(this.projectId()!, true);
      }
    }, 30000);
  }

  ngOnDestroy(): void {
    this.sub.unsubscribe();
    if (this.refreshInterval) {
      clearInterval(this.refreshInterval);
    }
  }

  loadDetails(id: string, silent = false): void {
    if (!silent) {
      this.loading.set(true);
    }
    this.error.set(null);

    forkJoin({
      project: this.projectService.getProject(id),
      // Shared lookup state used across child pages — fetch all apps (large page).
      apps: this.projectService.listProjectApps(id, 1, 1000)
    }).subscribe({
      next: (res) => {
        this.project.set(res.project);
        const appsList = res.apps?.items || [];
        this.apps.set(appsList);

        const currentSelected = this.selectedApp();
        if (currentSelected && appsList.some(a => a.id === currentSelected.id)) {
          const updated = appsList.find(a => a.id === currentSelected.id);
          this.selectedApp.set(updated || null);
        } else if (appsList.length > 0) {
          if (!currentSelected) {
            this.selectedApp.set(appsList[0]);
          }
        } else {
          this.selectedApp.set(null);
        }
        
        if (!silent) {
          this.loading.set(false);
        }
      },
      error: (err) => {
        if (!silent) {
          this.error.set(err.error?.message || 'Eroare la încărcarea detaliilor proiectului.');
          this.loading.set(false);
        }
      }
    });
  }

  onSelectApp(appId: string): void {
    const found = this.apps().find(a => a.id === appId);
    if (found) {
      this.selectedApp.set(found);
    }
  }

  onDeployApp(): void {
    if (!this.appName() || !this.gitRepository()) {
      this.toast.error('Numele aplicației și repository-ul Git sunt obligatorii.');
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
      enableBaas: this.enableBaas()
    }).subscribe({
      next: (res) => {
        this.deployingApp.set(false);
        this.showAddAppForm.set(false);
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
        this.enableBaas.set(false);
        this.selectedImportRepo.set(null);
        this.isCustomGitUrl.set(false);
        this.toast.success('Aplicația a fost înregistrată pentru deployment cu succes!');
        
        if (res && res.id) {
          this.router.navigate(['/projects', projectId, 'apps', res.id]);
        } else {
          this.loadDetails(projectId);
        }
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea aplicației.');
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

        // Populate subdirectories if returned (only at root/first detection)
        if (res.subdirectories && res.subdirectories.length > 0) {
          this.detectedSubdirectories.set(res.subdirectories);
          // Auto-select match if subpath is already set
          if (subpathVal && res.subdirectories.includes(subpathVal)) {
            this.selectedSubpathOption.set(subpathVal);
            this.subpathSelectionMode.set('select');
          } else if (!subpathVal) {
            this.selectedSubpathOption.set('');
            this.subpathSelectionMode.set('select');
          }
        }

        // Auto-populate env variables
        if (res.detectedEnvs && res.detectedEnvs.length > 0) {
          const rows: EnvVarInput[] = res.detectedEnvs.map((env: any) => ({
            key: env.key,
            value: env.value,
            isSecret: true
          }));
          this.newAppEnvRows.set(rows);
        } else {
          this.newAppEnvRows.set([]);
        }

        this.toast.success(`Tip proiect detectat: ${res.projectType.toUpperCase()}`);
      },
      error: () => {
        this.detectingType.set(false);
        this.detectedType.set('generic');
        this.detectedDescription.set('Tip proiect nespecificat sau subdirector invalid.');
        this.toast.error('Nu s-a putut detecta tipul în subdirector.');
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
    }, 600); // 600ms debounce
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

  getAppStatus(app: AppDetail | null): 'ACTIV' | 'INACTIV' | 'CONSTRUIRE' | 'EȘUAT' | 'CRASHED' | 'OPRIT' {
    if (!app || !app.instances || app.instances.length === 0) return 'INACTIV';
    const status = app.instances[0].status; // e.g. 'building', 'running', 'stopped', 'failed', 'crashed'
    if (status === 'running') return 'ACTIV';
    if (status === 'building') return 'CONSTRUIRE';
    if (status === 'failed') return 'EȘUAT';
    if (status === 'crashed') return 'CRASHED';
    if (status === 'stopped') return 'OPRIT';
    return 'INACTIV';
  }

  getAppStatusClass(app: AppDetail | null): string {
    const status = this.getAppStatus(app);
    switch (status) {
      case 'ACTIV':
        return 'bg-emerald-950/20 border-emerald-900/30 text-emerald-400';
      case 'CONSTRUIRE':
        return 'bg-amber-950/20 border-amber-900/30 text-amber-400 animate-pulse';
      case 'EȘUAT':
      case 'CRASHED':
        return 'bg-red-950/20 border-red-900/30 text-red-400';
      case 'OPRIT':
      case 'INACTIV':
      default:
        return 'bg-zinc-950 border-zinc-900 text-zinc-500';
    }
  }

  getAppIndicatorClass(app: AppDetail | null): string {
    const status = this.getAppStatus(app);
    switch (status) {
      case 'ACTIV':
        return 'bg-emerald-500';
      case 'CONSTRUIRE':
        return 'bg-amber-500 animate-pulse';
      case 'EȘUAT':
      case 'CRASHED':
        return 'bg-red-500';
      case 'OPRIT':
      case 'INACTIV':
      default:
        return 'bg-zinc-500';
    }
  }

  get activeTab(): string {
    const urlParts = this.router.url.split('/');
    return urlParts[3] || 'overview';
  }

  /** Whether the given sidebar tab is the active one (overview also matches ''). */
  isTabActive(tab: string): boolean {
    const active = this.activeTab;
    if (tab === 'overview') {
      return active === 'overview' || active === '';
    }
    return active === tab;
  }

  /** Class string for a sidebar tab link (active vs inactive variant). */
  tabClass(tab: string): string {
    const base = 'px-3 py-2.5 rounded-md text-xs flex items-center gap-2.5 transition-colors cursor-pointer';
    return this.isTabActive(tab)
      ? `${base} font-semibold text-white bg-zinc-900 border border-zinc-850`
      : `${base} text-zinc-400 hover:text-zinc-200 hover:bg-zinc-950/40`;
  }
}
