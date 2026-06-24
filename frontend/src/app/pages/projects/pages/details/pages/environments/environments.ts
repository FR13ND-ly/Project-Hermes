import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { ProjectService, GroupedAppEnv, EnvResponse, ProjectEnvResponse } from '../../../../../../core/services/project.service';
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

  readonly groups = signal<GroupedAppEnv[]>([]);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);
  readonly revealedEnvIds = signal<Record<string, boolean>>({});

  // Project-level env pool
  readonly projectEnv = signal<ProjectEnvResponse[]>([]);
  readonly showProjectAddForm = signal(false);
  readonly pKey = signal('');
  readonly pVal = signal('');
  readonly pIsSecret = signal(false);
  readonly savingProjectEnv = signal(false);

  // Revealed secret values for pool vars (id -> decrypted value), fetched on demand.
  readonly projectRevealed = signal<Record<string, string>>({});
  // Inline rename state for a pool var.
  readonly renamingId = signal<string | null>(null);
  readonly renameKey = signal('');
  readonly savingRename = signal(false);

  // Inline add/edit form (per instance)
  readonly addFormInstance = signal<string | null>(null);
  readonly editingEnvId = signal<string | null>(null);
  readonly formKey = signal('');
  readonly formVal = signal('');
  readonly formIsSecret = signal(false);
  readonly savingEnv = signal(false);

  // JSON editor (per instance)
  readonly jsonInstance = signal<string | null>(null);
  readonly jsonText = signal('');
  readonly savingJson = signal(false);

  constructor() {
    effect(() => {
      const projectId = this.parent.projectId();
      if (projectId) {
        this.load();
      }
    });
  }

  ngOnInit(): void {
    this.load();
  }

  load(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loading.set(true);
    this.error.set(null);
    this.projectService.listProjectEnvsGrouped(projectId).subscribe({
      next: (res) => {
        this.groups.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea variabilelor de mediu.');
        this.loading.set(false);
      }
    });
    this.loadProjectEnv();
  }

  // --- Project-level env pool ---
  loadProjectEnv(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.projectService.listProjectEnv(projectId).subscribe({
      next: (res) => this.projectEnv.set(res || []),
      error: () => this.projectEnv.set([])
    });
  }

  toggleProjectAddForm(): void {
    this.showProjectAddForm.set(!this.showProjectAddForm());
    this.pKey.set('');
    this.pVal.set('');
    this.pIsSecret.set(false);
  }

  saveProjectEnv(): void {
    const projectId = this.parent.projectId();
    if (!projectId || !this.pKey().trim() || !this.pVal().trim()) return;

    this.savingProjectEnv.set(true);
    this.projectService.setProjectEnv(projectId, {
      key: this.pKey().trim(),
      value: this.pVal().trim(),
      isSecret: this.pIsSecret()
    }).subscribe({
      next: () => {
        this.savingProjectEnv.set(false);
        this.showProjectAddForm.set(false);
        this.toast.success('Variabila de proiect a fost salvată!');
        this.loadProjectEnv();
      },
      error: (err) => {
        this.savingProjectEnv.set(false);
        this.toast.error(err.error?.message || 'Eroare la salvare.');
      }
    });
  }

  async deleteProjectEnv(id: string): Promise<void> {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Variabilă de Proiect',
      message: 'Sigur doriți să ștergeți această variabilă? Va fi eliminată din toate aplicațiile care o folosesc.',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteProjectEnv(projectId, id).subscribe({
      next: () => {
        this.toast.success('Variabila de proiect a fost ștearsă.');
        this.loadProjectEnv();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergere.');
      }
    });
  }

  // Reveal a pool var's value. Non-secrets already carry their value; secrets are
  // fetched (decrypted) from the backend on demand and cached.
  toggleRevealProjectEnv(env: ProjectEnvResponse): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    // Already revealed -> hide.
    if (this.projectRevealed()[env.id] !== undefined) {
      this.projectRevealed.update(m => {
        const next = { ...m };
        delete next[env.id];
        return next;
      });
      return;
    }

    if (!env.isSecret && env.value != null) {
      this.projectRevealed.update(m => ({ ...m, [env.id]: env.value as string }));
      return;
    }

    this.projectService.revealProjectEnv(projectId, env.id).subscribe({
      next: (res) => this.projectRevealed.update(m => ({ ...m, [env.id]: res.value })),
      error: (err) => this.toast.error(err.error?.message || 'Eroare la dezvăluirea valorii.')
    });
  }

  startRename(env: ProjectEnvResponse): void {
    this.renamingId.set(env.id);
    this.renameKey.set(env.key);
  }

  cancelRename(): void {
    this.renamingId.set(null);
    this.renameKey.set('');
  }

  saveRename(env: ProjectEnvResponse): void {
    const projectId = this.parent.projectId();
    const key = this.renameKey().trim();
    if (!projectId || !key || key === env.key) {
      this.cancelRename();
      return;
    }
    this.savingRename.set(true);
    this.projectService.renameProjectEnv(projectId, env.id, key).subscribe({
      next: () => {
        this.savingRename.set(false);
        this.cancelRename();
        this.toast.success('Cheia a fost redenumită.');
        this.loadProjectEnv();
      },
      error: (err) => {
        this.savingRename.set(false);
        this.toast.error(err.error?.message || 'Eroare la redenumire.');
      }
    });
  }

  get totalVars(): number {
    return this.groups().reduce(
      (sum, app) => sum + app.instances.reduce((s, inst) => s + inst.variables.length, 0),
      0
    );
  }

  toggleReveal(id: string): void {
    this.revealedEnvIds.update(ids => ({ ...ids, [id]: !ids[id] }));
  }

  // --- Add / edit variable ---
  openAddForm(instanceId: string): void {
    this.jsonInstance.set(null);
    const wasOpen = this.addFormInstance() === instanceId && this.editingEnvId() === null;
    this.addFormInstance.set(wasOpen ? null : instanceId);
    this.editingEnvId.set(null);
    this.formKey.set('');
    this.formVal.set('');
    this.formIsSecret.set(false);
  }

  // Open the add form prefilled to edit an existing var (key locked; value blank for secrets).
  startEditEnv(instanceId: string, env: EnvResponse): void {
    this.jsonInstance.set(null);
    this.addFormInstance.set(instanceId);
    this.editingEnvId.set(env.id);
    this.formKey.set(env.key);
    this.formVal.set(env.isSecret ? '' : (env.value ?? ''));
    this.formIsSecret.set(env.isSecret);
  }

  cancelAddForm(): void {
    this.addFormInstance.set(null);
    this.editingEnvId.set(null);
    this.formKey.set('');
    this.formVal.set('');
    this.formIsSecret.set(false);
  }

  saveEnv(): void {
    const instanceId = this.addFormInstance();
    // When editing a secret, allow an empty value only if the user really wants to
    // blank it; otherwise require a value. Key is always required.
    if (!instanceId || !this.formKey().trim()) return;

    const wasEditing = this.editingEnvId() !== null;
    this.savingEnv.set(true);
    this.projectService.setEnvVariable({
      appInstanceId: instanceId,
      key: this.formKey().trim(),
      value: this.formVal(),
      isSecret: this.formIsSecret()
    }).subscribe({
      next: () => {
        this.savingEnv.set(false);
        this.cancelAddForm();
        this.toast.success(wasEditing ? 'Variabila a fost actualizată!' : 'Variabila a fost salvată!');
        this.load();
      },
      error: (err) => {
        this.savingEnv.set(false);
        this.toast.error(err.error?.message || 'Eroare la salvare.');
      }
    });
  }

  // --- JSON editor ---
  openJsonEditor(instanceId: string, variables: EnvResponse[]): void {
    this.addFormInstance.set(null);
    if (this.jsonInstance() === instanceId) {
      this.jsonInstance.set(null);
      return;
    }
    const obj: Record<string, string> = {};
    for (const env of variables) {
      obj[env.key] = env.isSecret ? '' : (env.value ?? '');
    }
    this.jsonText.set(JSON.stringify(obj, null, 2));
    this.jsonInstance.set(instanceId);
  }

  saveJson(): void {
    const instanceId = this.jsonInstance();
    if (!instanceId) return;

    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(this.jsonText());
    } catch {
      this.toast.error('JSON invalid. Verificați sintaxa.');
      return;
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      this.toast.error('JSON-ul trebuie să fie un obiect { "CHEIE": "valoare" }.');
      return;
    }

    const variables = Object.entries(parsed).map(([key, value]) => ({
      key,
      value: value === null || value === undefined ? '' : String(value),
      isSecret: false
    }));

    this.savingJson.set(true);
    this.projectService.setEnvsBulk(instanceId, variables).subscribe({
      next: () => {
        this.savingJson.set(false);
        this.jsonInstance.set(null);
        this.toast.success('Variabilele au fost actualizate.');
        this.load();
      },
      error: (err) => {
        this.savingJson.set(false);
        this.toast.error(err.error?.message || 'Eroare la salvarea variabilelor.');
      }
    });
  }

  async deleteEnv(envId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Variabilă de Mediu',
      message: 'Sigur doriți să ștergeți această variabilă? Va fi aplicată la următoarea redeploiere.',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteEnvVariable(envId).subscribe({
      next: () => {
        this.toast.success('Variabila a fost ștearsă.');
        this.load();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergere.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }
}
