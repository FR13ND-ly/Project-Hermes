import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { ProjectService, EnvResponse } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-environments',
  imports: [FormsModule],
  templateUrl: './environments.html',
  styleUrl: './environments.css',
})
export class Environments implements OnInit {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly envVariables = signal<EnvResponse[]>([]);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Form states
  readonly showCreateForm = signal(false);
  readonly settingEnv = signal(false);
  
  readonly envKey = signal('');
  readonly envVal = signal('');
  readonly envScope = signal<'all' | 'production' | 'staging' | 'preview'>('all');
  readonly isSecret = signal(true);

  readonly saveTarget = signal<'project' | 'app'>('app');
  readonly revealedEnvIds = signal<Record<string, boolean>>({});

  constructor() {
    effect(() => {
      const appId = this.parent.projectId();
      // Watch both project ID and app detail changes to reload
      const appDetail = this.parent.appDetail();
      if (appId) {
        this.loadEnvVariables();
      }
    });
  }

  ngOnInit(): void {
    this.loadEnvVariables();
  }

  loadEnvVariables(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    const appInstanceId = this.parent.appDetail()?.instances?.[0]?.id || null;

    this.loading.set(true);
    this.error.set(null);

    this.projectService.listEnvVariables(projectId, appInstanceId).subscribe({
      next: (res) => {
        this.envVariables.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea variabilelor de mediu.');
        this.loading.set(false);
      }
    });
  }

  isOverridden(env: EnvResponse): boolean {
    if (env.appInstanceId !== null) return false;
    return this.envVariables().some(e => e.appInstanceId !== null && e.key === env.key);
  }

  onToggleReveal(id: string): void {
    this.revealedEnvIds.update(ids => ({
      ...ids,
      [id]: !ids[id]
    }));
  }

  onSaveEnv(): void {
    const projectId = this.parent.projectId();
    if (!projectId || !this.envKey() || !this.envVal()) return;

    const appInstanceId = this.parent.appDetail()?.instances?.[0]?.id || null;
    const isAppTarget = this.saveTarget() === 'app' && appInstanceId !== null;

    this.settingEnv.set(true);
    this.error.set(null);

    this.projectService.setEnvVariable({
      projectId: isAppTarget ? null : projectId,
      appInstanceId: isAppTarget ? appInstanceId : null,
      key: this.envKey().trim(),
      value: this.envVal().trim(),
      scope: this.envScope(),
      isSecret: this.isSecret()
    }).subscribe({
      next: () => {
        this.envKey.set('');
        this.envVal.set('');
        this.showCreateForm.set(false);
        this.settingEnv.set(false);
        this.toast.success('Variabila de mediu a fost salvată cu succes!');
        this.loadEnvVariables();
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la salvarea variabilei de mediu.');
        this.settingEnv.set(false);
      }
    });
  }

  async onDeleteEnv(envId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Variabilă de Mediu',
      message: 'Sigur doriți să ștergeți această variabilă de mediu? Aceasta va fi eliminată definitiv din instanțe.',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteEnvVariable(envId).subscribe({
      next: () => {
        this.toast.success('Variabila de mediu a fost ștearsă.');
        this.loadEnvVariables();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea variabilei.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }
}
