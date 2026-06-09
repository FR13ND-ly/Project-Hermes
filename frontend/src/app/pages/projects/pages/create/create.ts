import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';
import { ProjectService } from '../../../../core/services/project.service';

@Component({
  selector: 'app-create',
  imports: [FormsModule, RouterLink],
  templateUrl: './create.html',
  styleUrl: './create.css',
})
export class Create {
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);

  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  readonly projectName = signal('');

  onCreateProject(): void {
    if (!this.projectName().trim()) {
      this.error.set('Vă rugăm să introduceți numele proiectului.');
      return;
    }

    this.loading.set(true);
    this.error.set(null);

    this.projectService.createProject(this.projectName().trim(), null).subscribe({
      next: (res) => {
        this.loading.set(false);
        this.router.navigate(['/projects', res.id]);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la crearea proiectului.');
        this.loading.set(false);
      }
    });
  }
}
