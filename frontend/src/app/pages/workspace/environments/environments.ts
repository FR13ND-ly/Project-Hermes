import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ProjectService, EnvResponse } from '../../../core/services/project.service';
import { ToastService } from '../../../core/services/toast.service';
import { ConfirmService } from '../../../core/services/confirm.service';

@Component({
  selector: 'app-workspace-environments',
  imports: [FormsModule],
  templateUrl: './environments.html',
  styleUrl: './environments.css',
})
export class WorkspaceEnvironments implements OnInit {
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

  // Set of env variable IDs that are revealed (non-secret ones)
  readonly revealedEnvIds = signal<Record<string, boolean>>({});

  ngOnInit(): void {
    this.loadEnvVariables();
  }

  loadEnvVariables(): void {
    this.loading.set(true);
    this.error.set(null);

    this.projectService.listEnvVariables(null).subscribe({
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

  onToggleReveal(id: string): void {
    this.revealedEnvIds.update(ids => ({
      ...ids,
      [id]: !ids[id]
    }));
  }

  onSaveEnv(): void {
    if (!this.envKey() || !this.envVal()) return;

    this.settingEnv.set(true);
    this.error.set(null);

    this.projectService.setEnvVariable({
      projectId: null,
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
