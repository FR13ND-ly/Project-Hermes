import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { StorageService } from '../../../../../../core/services/storage.service';
import { ToastService } from '../../../../../../core/services/toast.service';

@Component({
  selector: 'app-storage-create',
  imports: [FormsModule],
  templateUrl: './storage-create.html',
  styleUrl: './storage-create.css',
})
export class StorageCreate {
  readonly parent = inject(Details);
  private readonly storageService = inject(StorageService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);

  readonly creatingBucket = signal(false);
  readonly newBucketName = signal('');
  readonly maxBucketSizeGb = signal<number>(1);
  readonly maxFileSizeMb = signal<number>(0);
  readonly allowCustomProcessing = signal<boolean>(false);
  readonly isPublicToggle = signal<boolean>(false);
  readonly publishAppId = signal(false);
  readonly appIdEnvKeyName = signal('');
  readonly publishSecretKey = signal(false);
  readonly secretKeyEnvKeyName = signal('');

  onCreateBucket(): void {
    if (!this.newBucketName().trim()) {
      this.toast.error('Bucket name is required.');
      return;
    }

    if (this.publishAppId() && !this.appIdEnvKeyName().trim()) {
      this.toast.error('The environment variable name for App ID is required when checked.');
      return;
    }
    if (this.publishSecretKey() && !this.secretKeyEnvKeyName().trim()) {
      this.toast.error('The environment variable name for Secret Key is required when checked.');
      return;
    }

    this.creatingBucket.set(true);
    this.storageService.createBucket({
      name: this.newBucketName().trim(),
      projectId: this.parent.projectId() || undefined,
      isPublic: this.isPublicToggle(),
      maxBucketSizeBytes: this.maxBucketSizeGb() * 1024 * 1024 * 1024,
      maxFileSizeBytes: Math.max(0, Math.round(this.maxFileSizeMb())) * 1024 * 1024,
      allowCustomProcessing: this.allowCustomProcessing(),
      publishAppId: this.publishAppId(),
      appIdEnvKey: this.publishAppId() && this.appIdEnvKeyName().trim() ? this.appIdEnvKeyName().trim() : undefined,
      publishSecretKey: this.publishSecretKey(),
      secretKeyEnvKey: this.publishSecretKey() && this.secretKeyEnvKeyName().trim() ? this.secretKeyEnvKeyName().trim() : undefined
    }).subscribe({
      next: (newBucket) => {
        this.toast.success(`Bucket "${newBucket.name}" has been successfully created.`);
        this.creatingBucket.set(false);
        this.router.navigate(['/projects', this.parent.projectId(), 'storages']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error creating bucket.');
        this.creatingBucket.set(false);
      }
    });
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'storages']);
  }
}
