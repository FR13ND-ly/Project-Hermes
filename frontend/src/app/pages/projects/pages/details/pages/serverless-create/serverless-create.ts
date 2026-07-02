import { Component, inject, signal } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { ProjectService } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';

@Component({
  selector: 'app-serverless-create',
  imports: [FormsModule],
  templateUrl: './serverless-create.html',
  styleUrl: './serverless-create.css',
})
export class ServerlessCreate {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);

  readonly creating = signal(false);
  readonly newName = signal('');
  readonly newRuntime = signal('nodejs-cjs');
  readonly newMemory = signal(0); // 0 = unlimited

  onCreateInstance(): void {
    const projId = this.parent.projectId();
    if (!projId) return;
    const name = this.newName().trim();
    if (!name) {
      this.toast.error('Instance name is required.');
      return;
    }
    this.creating.set(true);
    this.projectService.createInstance(projId, {
      name,
      runtime: this.newRuntime(),
      memoryLimitMb: this.newMemory()
    }).subscribe({
      next: (res) => {
        this.creating.set(false);
        this.toast.success('Serverless instance created.');
        this.router.navigate(['/projects', projId, 'serverless']);
      },
      error: (err) => {
        this.creating.set(false);
        this.toast.error(err.error?.message || 'Error creating instance.');
      }
    });
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'serverless']);
  }
}
