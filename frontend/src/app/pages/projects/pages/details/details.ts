import { Component, inject, signal, OnInit, computed } from '@angular/core';
import { ActivatedRoute, RouterLink, RouterOutlet, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { forkJoin } from 'rxjs';
import { ProjectService, AppDetail, Project, EnvVarInput } from '../../../../core/services/project.service';
import { ToastService } from '../../../../core/services/toast.service';
import { AuthService } from '../../../../core/services/auth';
import { GithubService, GithubRepo, GithubBranch } from '../../../../core/services/github.service';

@Component({
  selector: 'app-details',
  imports: [RouterLink, RouterOutlet, FormsModule, DatePipe],
  templateUrl: './details.html',
  styleUrl: './details.css',
})
export class Details implements OnInit {
  private readonly route = inject(ActivatedRoute);
  private readonly projectService = inject(ProjectService);
  readonly router = inject(Router);
  readonly toast = inject(ToastService);
  readonly authService = inject(AuthService);
  private readonly githubService = inject(GithubService);

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

  // GitHub account integration state
  readonly githubTokenInput = signal('');
  readonly linkingGithub = signal(false);
  readonly githubRepos = signal<GithubRepo[]>([]);
  readonly loadingRepos = signal(false);
  readonly githubBranches = signal<GithubBranch[]>([]);
  readonly loadingBranches = signal(false);

  // Auto-detection signals
  readonly detectedType = signal<string>('');
  readonly detectingType = signal(false);
  readonly detectedDescription = signal<string>('');

  readonly repoSearchQuery = signal('');
  readonly selectedImportRepo = signal<any | null>(null);

  // Create-app wizard: 1 = Repo, 2 = Setări, 3 = Env
  readonly createStep = signal(1);
  nextCreateStep(): void { this.createStep.update(s => Math.min(3, s + 1)); }
  prevCreateStep(): void { this.createStep.update(s => Math.max(1, s - 1)); }
  readonly isCustomGitUrl = signal(false);

  readonly filteredRepos = computed(() => {
    const query = this.repoSearchQuery().toLowerCase().trim();
    if (!query) return this.githubRepos();
    return this.githubRepos().filter(r => r.name.toLowerCase().includes(query) || r.full_name.toLowerCase().includes(query));
  });

  linkGithub(): void {
    const token = this.githubTokenInput().trim();
    if (!token) {
      this.toast.error('Tokenul GitHub nu poate fi gol.');
      return;
    }
    this.linkingGithub.set(true);
    this.githubService.saveToken(token).subscribe({
      next: (updatedUser) => {
        this.authService.updateUser(updatedUser);
        this.linkingGithub.set(false);
        this.githubTokenInput.set('');
        this.toast.success('Contul GitHub a fost conectat cu succes!');
        this.loadGithubRepos();
      },
      error: (err) => {
        this.toast.error(err.error?.error?.message || err.error?.message || 'Eroare la conectarea contului GitHub.');
        this.linkingGithub.set(false);
      }
    });
  }

  disconnectGithub(): void {
    this.linkingGithub.set(true);
    this.githubService.saveToken(null).subscribe({
      next: (updatedUser) => {
        this.authService.updateUser(updatedUser);
        this.linkingGithub.set(false);
        this.toast.success('Contul GitHub a fost deconectat.');
        this.githubRepos.set([]);
      },
      error: (err) => {
        this.toast.error(err.error?.error?.message || err.error?.message || 'Eroare la deconectarea contului GitHub.');
        this.linkingGithub.set(false);
      }
    });
  }

  loadGithubRepos(): void {
    if (!this.authService.currentUser()?.github_username) return;
    this.loadingRepos.set(true);
    this.githubService.listRepos().subscribe({
      next: (repos) => {
        this.githubRepos.set(repos);
        this.loadingRepos.set(false);
      },
      error: (err) => {
        this.toast.error('Eroare la încărcarea repository-urilor GitHub.');
        this.loadingRepos.set(false);
      }
    });
  }

  loadBranches(owner: string, repo: string): void {
    this.loadingBranches.set(true);
    this.githubBranches.set([]);
    this.githubService.listBranches(owner, repo).subscribe({
      next: (branches) => {
        this.githubBranches.set(branches);
        this.loadingBranches.set(false);
        if (branches.length > 0) {
          const hasMain = branches.some(b => b.name === 'main');
          const hasMaster = branches.some(b => b.name === 'master');
          if (hasMain) {
            this.branchName.set('main');
          } else if (hasMaster) {
            this.branchName.set('master');
          } else {
            this.branchName.set(branches[0].name);
          }
        }
      },
      error: (err) => {
        this.toast.error('Eroare la încărcarea branch-urilor.');
        this.loadingBranches.set(false);
      }
    });
  }

  onImportRepo(repo: any): void {
    this.selectedImportRepo.set(repo);
    this.createStep.set(1);
    this.isCustomGitUrl.set(false);
    this.appName.set(repo.name);
    this.gitRepository.set(repo.html_url || repo.url);
    this.branchName.set('main');
    
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
    
    this.loadBranches(repo.owner.login, repo.name);
    
    // Auto-detect project type and configs in specified subdirectory
    this.triggerAutoDetection();
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
    this.githubBranches.set([]);
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
      }
    });
    this.loadGithubRepos();
  }

  loadDetails(id: string): void {
    this.loading.set(true);
    this.error.set(null);

    forkJoin({
      project: this.projectService.getProject(id),
      apps: this.projectService.listProjectApps(id)
    }).subscribe({
      next: (res) => {
        this.project.set(res.project);
        const appsList = res.apps || [];
        this.apps.set(appsList);

        const currentSelected = this.selectedApp();
        if (currentSelected && appsList.some(a => a.id === currentSelected.id)) {
          const updated = appsList.find(a => a.id === currentSelected.id);
          this.selectedApp.set(updated || null);
        } else if (appsList.length > 0) {
          this.selectedApp.set(appsList[0]);
        } else {
          this.selectedApp.set(null);
        }
        
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea detaliilor proiectului.');
        this.loading.set(false);
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
      envVariables: envVariables.length > 0 ? envVariables : undefined
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

    const subpathVal = this.gitSubpath() ? this.gitSubpath().trim() : undefined;

    this.githubService.detectProjectType(repo.owner.login, repo.name, subpathVal).subscribe({
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
}
