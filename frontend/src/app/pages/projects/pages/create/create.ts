import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';
import { ProjectService } from '../../../../core/services/project.service';
import { CloudflareService, CloudflareCredential } from '../../../../core/services/cloudflare.service';

@Component({
  selector: 'app-create',
  imports: [FormsModule, RouterLink],
  templateUrl: './create.html',
  styleUrl: './create.css',
})
export class Create implements OnInit {
  private readonly projectService = inject(ProjectService);
  private readonly cloudflareService = inject(CloudflareService);
  private readonly router = inject(Router);

  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  readonly projectName = signal('');

  // Optional Cloudflare credential to associate at creation (changeable later).
  readonly cloudflareCredentials = signal<CloudflareCredential[]>([]);
  readonly selectedCloudflareId = signal('');

  ngOnInit(): void {
    this.cloudflareService.listCredentials().subscribe({
      next: (list) => this.cloudflareCredentials.set(list || []),
      error: () => this.cloudflareCredentials.set([])
    });
  }

  onCreateProject(): void {
    if (!this.projectName().trim()) {
      this.error.set('Please enter the project name.');
      return;
    }

    this.loading.set(true);
    this.error.set(null);

    this.projectService.createProject(this.projectName().trim(), null, this.selectedCloudflareId() || null).subscribe({
      next: (res) => {
        this.loading.set(false);
        this.router.navigate(['/projects', res.id]);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Error creating the project.');
        this.loading.set(false);
      }
    });
  }
}
