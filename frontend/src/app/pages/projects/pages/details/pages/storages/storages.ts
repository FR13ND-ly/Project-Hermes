import { Component, inject, signal, OnInit, OnDestroy, effect, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { StorageService, StorageBucket, StorageObject, ImageVariant, ImageVariantSpec, ImageFormatTarget, ProjectVolume } from '../../../../../../core/services/storage.service';
import { VolumeService, VolumeFileItem } from '../../../../../../core/services/volume.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';
import { HttpEvent, HttpEventType } from '@angular/common/http';
import { environment } from '../../../../../../../environments/environment';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

export interface VirtualItem {
  id?: string;
  name: string;
  isFolder: boolean;
  filePath: string;
  sizeBytes?: number;
  mimeType?: string;
  etag?: string;
  status?: string;
  processingStage?: string | null;
  compression?: string;
  originalSizeBytes?: number | null;
  isOptimized?: boolean;
  imageDimensions?: string | null;
  hasVariants?: boolean;
  variants?: Record<string, ImageVariant> | null;
  virtualUrl?: string;
  createdAt?: string;
}

@Component({
  selector: 'app-storages',
  standalone: true,
  imports: [CommonModule, FormsModule, DatePipe, Pagination],
  templateUrl: './storages.html',
  styleUrl: './storages.css',
})
export class Storages implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly storageService = inject(StorageService);
  private readonly volumeService = inject(VolumeService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);
  private readonly sub = new Subscription();

  // PVCs (app volumes) listed + browsed centrally in Storage
  readonly pvcs = signal<ProjectVolume[]>([]);
  readonly loadingPvcs = signal(false);
  // A selected PVC is browsed through the SAME detail view as a bucket.
  readonly selectedPvc = signal<ProjectVolume | null>(null);
  readonly pvcDirItems = signal<VolumeFileItem[]>([]);

  readonly buckets = signal<StorageBucket[]>([]);
  readonly selectedBucket = signal<StorageBucket | null>(null);
  readonly rotatingCreds = signal(false);
  readonly rotatedSecret = signal<string | null>(null);
  readonly allFiles = signal<StorageObject[]>([]);

  // Pagination for the bucket's object list.
  readonly objectsPage = signal(1);
  readonly objectsPageSize = signal(DEFAULT_PAGE_SIZE);
  readonly objectsTotal = signal(0);
  
  readonly loading = signal(false);
  readonly loadingFiles = signal(false);
  readonly error = signal<string | null>(null);

  // Tab navigation inside selected bucket details
  readonly activeTab = signal<'files' | 'logs' | 'settings' | 'api'>('files');

  // Explorer active navigation path
  readonly currentPath = signal<string>('/');
  readonly searchQuery = signal<string>('');
  readonly viewMode = signal<'grid' | 'list'>('grid');

  // Expanded variants tracking (file IDs whose variant panels are open)
  readonly expandedVariants = signal<Set<string>>(new Set());

  // Bucket Creation Form states
  readonly showCreateForm = signal(false);
  readonly creatingBucket = signal(false);
  readonly newBucketName = signal('');
  readonly maxBucketSizeGb = signal<number>(1);
  // Per-file size limit (MB; 0 = unlimited) for creation.
  readonly maxFileSizeMb = signal<number>(0);
  // Allow the uploading client (via API/token) to override processing rules.
  readonly allowCustomProcessing = signal<boolean>(false);
  readonly isPublicToggle = signal<boolean>(false);
  readonly publishAppId = signal(false);
  readonly appIdEnvKeyName = signal('');
  readonly publishSecretKey = signal(false);
  readonly secretKeyEnvKeyName = signal('');

  // Upload states
  readonly uploading = signal(false);
  readonly uploadProgress = signal<number>(0);

  // Virtual Folder Creation Form states
  readonly showFolderForm = signal(false);
  readonly newFolderName = signal('');

  // Edit Bucket Settings Form States
  readonly editName = signal('');
  readonly editMaxSizeGb = signal<number>(1);
  readonly editMaxFileSizeMb = signal<number>(0);
  readonly editAllowCustomProcessing = signal<boolean>(false);
  readonly editIsPublic = signal<boolean>(false);
  readonly savingSettings = signal(false);

  // Advanced Image Rules
  readonly convertImageTo = signal<ImageFormatTarget>('original');
  readonly imageQuality = signal<number>(85);
  readonly forceSquare = signal<boolean>(false);
  // Custom output variants (name + max width + per-variant format) — replaces the
  // old fixed xs/s/md/lg presets.
  readonly customVariants = signal<ImageVariantSpec[]>([]);

  // Text Compression
  readonly compressBrotli = signal<boolean>(false);
  readonly compressGzip = signal<boolean>(false);

  // Bucket access uses the app_id/secret_key key pair (published to the project env
  // pool, rotatable in Settings). The old long-lived JWT token was removed.

  // Allowed file types checkboxes
  readonly allowImages = signal<boolean>(true);
  readonly allowTextCssJs = signal<boolean>(true);
  readonly allowPdfs = signal<boolean>(true);
  readonly allowArchives = signal<boolean>(true);
  readonly allowOthers = signal<boolean>(true);

  private pollingInterval: any = null;

  constructor() {
    effect(() => {
      // Reload objects list when bucket is changed
      const bucket = this.selectedBucket();
      if (bucket) {
        this.currentPath.set('/');
        this.loadObjects();
      }
    });
  }

  ngOnInit(): void {
    this.loadBuckets();
    this.loadPvcs();

    // Live refresh when a file finishes processing (Ready/Failed) in the bucket
    // currently open. The poll below stays only as a reconciliation safety-net.
    this.sub.add(
      this.wsService.onEvent<{ bucket_id: string }>('storage_object_updated').subscribe((payload) => {
        const bucket = this.selectedBucket();
        if (bucket && payload?.bucket_id === bucket.id) {
          this.reloadObjectsSilent();
        }
      })
    );
  }

  // Silent reload (no loading flicker) for WS-driven and poll-driven refreshes.
  private reloadObjectsSilent(): void {
    const bucket = this.selectedBucket();
    if (!bucket) {
      this.stopPolling();
      return;
    }
    this.storageService.listObjects(bucket.slug, this.objectsPage(), this.objectsPageSize()).subscribe({
      next: (res) => {
        this.allFiles.set(res?.items || []);
        this.objectsTotal.set(res?.total || 0);
        this.checkAndStartPolling();
      },
      error: () => this.stopPolling()
    });
  }

  // --- PVCs (read-only browse from Storage) ---
  loadPvcs(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.loadingPvcs.set(true);
    this.storageService.listProjectVolumes(projectId).subscribe({
      next: (res) => { this.pvcs.set(res || []); this.loadingPvcs.set(false); },
      error: () => { this.pvcs.set([]); this.loadingPvcs.set(false); }
    });
  }

  // Open a PVC in the shared bucket-style detail view.
  selectPvc(pvc: ProjectVolume): void {
    this.selectedBucket.set(null);
    this.selectedPvc.set(pvc);
    this.activeTab.set('files');
    this.searchQuery.set('');
    this.currentPath.set('/');
    this.loadPvcDir('/');
  }

  loadPvcDir(path: string): void {
    const pvc = this.selectedPvc();
    if (!pvc) return;
    this.loadingFiles.set(true);
    this.volumeService.listFiles(pvc.id, path).subscribe({
      next: (res) => { this.pvcDirItems.set(res || []); this.loadingFiles.set(false); },
      error: () => { this.pvcDirItems.set([]); this.loadingFiles.set(false); }
    });
  }

  downloadPvcItem(item: VirtualItem): void {
    const pvc = this.selectedPvc();
    if (!pvc || item.isFolder) return;
    const path = item.filePath.startsWith('/') ? item.filePath : `/${item.filePath}`;
    const url = this.volumeService.downloadFileUrl(pvc.id, path);
    const a = document.createElement('a');
    a.href = url;
    a.download = item.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  onPvcFileSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    if (file) this.uploadPvcFile(file);
    input.value = '';
  }

  uploadPvcFile(file: File): void {
    const pvc = this.selectedPvc();
    if (!pvc) return;
    this.uploading.set(true);
    this.uploadProgress.set(0);
    this.volumeService.uploadFileProgress(pvc.id, this.currentPath(), file).subscribe({
      next: (event: HttpEvent<any>) => {
        if (event.type === HttpEventType.UploadProgress) {
          this.uploadProgress.set(event.total ? Math.round((100 * event.loaded) / event.total) : 0);
        } else if (event.type === HttpEventType.Response) {
          this.uploading.set(false);
          this.uploadProgress.set(0);
          this.toast.success(`Fișierul "${file.name}" a fost încărcat.`);
          this.loadPvcDir(this.currentPath());
        }
      },
      error: (err) => {
        this.uploading.set(false);
        this.uploadProgress.set(0);
        this.toast.error(err.error?.message || 'Eroare la încărcarea fișierului.');
      }
    });
  }

  createPvcFolder(): void {
    const pvc = this.selectedPvc();
    const name = this.newFolderName().trim().replace(/[\/\\]/g, '');
    if (!pvc || !name) return;
    this.volumeService.createFolder(pvc.id, this.currentPath(), name).subscribe({
      next: () => {
        this.newFolderName.set('');
        this.showFolderForm.set(false);
        this.toast.success('Directorul a fost creat.');
        this.loadPvcDir(this.currentPath());
      },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la crearea directorului.')
    });
  }

  async deletePvcItem(item: VirtualItem): Promise<void> {
    const pvc = this.selectedPvc();
    if (!pvc) return;
    const confirmed = await this.confirm.ask({
      title: 'Ștergere',
      message: `Sigur ștergi "${item.name}"?`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;
    const path = item.filePath.startsWith('/') ? item.filePath : `/${item.filePath}`;
    this.volumeService.deleteFile(pvc.id, path).subscribe({
      next: () => { this.toast.success('Șters cu succes.'); this.loadPvcDir(this.currentPath()); },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la ștergere.')
    });
  }

  loadBuckets(): void {
    this.loading.set(true);
    this.error.set(null);

    this.storageService.listBuckets().subscribe({
      next: (res) => {
        this.buckets.set(res || []);
        // update selected bucket reference if it's currently open
        const currentSelected = this.selectedBucket();
        if (currentSelected) {
          const updated = res.find(b => b.id === currentSelected.id);
          if (updated) {
            this.selectedBucket.set(updated);
          }
        }
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea bucket-urilor.');
        this.loading.set(false);
      }
    });
  }

  selectBucket(bucket: StorageBucket): void {
    this.objectsPage.set(1);
    this.selectedBucket.set(bucket);
    this.activeTab.set('files');
    this.currentPath.set('/');

    // Populate form states
    this.editName.set(bucket.name);
    this.editMaxSizeGb.set(Math.round(bucket.maxBucketSizeBytes / (1024 * 1024 * 1024)));
    this.editMaxFileSizeMb.set(Math.round((bucket.maxFileSizeBytes || 0) / (1024 * 1024)));
    this.editAllowCustomProcessing.set(bucket.allowCustomProcessing || false);
    this.editIsPublic.set(bucket.isPublic);

    // Parse allowed file types
    const allowed = bucket.allowedFileTypes || [];
    this.allowImages.set(allowed.length === 0 || allowed.some(t => t.startsWith('image/')));
    this.allowTextCssJs.set(allowed.length === 0 || allowed.some(t => t.startsWith('text/') || t === 'application/javascript'));
    this.allowPdfs.set(allowed.length === 0 || allowed.some(t => t === 'application/pdf'));
    this.allowArchives.set(allowed.length === 0 || allowed.some(t => t === 'application/zip' || t.includes('tar') || t.includes('gzip')));
    this.allowOthers.set(allowed.length === 0 || allowed.some(t => !t.startsWith('image/') && !t.startsWith('text/') && t !== 'application/javascript' && t !== 'application/pdf' && !t.includes('zip') && !t.includes('tar')));

    // Parse default processing rules
    const rules = bucket.defaultProcessingRules || {};
    const imgOpts = rules.imageOptions || null;
    if (imgOpts) {
      this.convertImageTo.set(imgOpts.convertTo || 'original');
      this.imageQuality.set(imgOpts.quality || 85);
      this.forceSquare.set(imgOpts.forceSquare || false);
      // Deep-copy so edits don't mutate the bucket reference held in the list.
      this.customVariants.set((imgOpts.variants || []).map(v => ({ ...v })));
    } else {
      this.convertImageTo.set('original');
      this.imageQuality.set(85);
      this.forceSquare.set(false);
      this.customVariants.set([]);
    }

    const textOpts = rules.textOptions || null;
    if (textOpts) {
      this.compressBrotli.set(textOpts.preCompressBrotli || false);
      this.compressGzip.set(textOpts.preCompressGzip || false);
    } else {
      this.compressBrotli.set(false);
      this.compressGzip.set(false);
    }
  }

  deselectBucket(): void {
    this.selectedBucket.set(null);
    this.selectedPvc.set(null);
    this.pvcDirItems.set([]);
    this.stopPolling();
  }

  loadObjects(): void {
    const bucket = this.selectedBucket();
    if (!bucket) {
      this.allFiles.set([]);
      this.stopPolling();
      return;
    }

    this.loadingFiles.set(true);
    this.storageService.listObjects(bucket.slug, this.objectsPage(), this.objectsPageSize()).subscribe({
      next: (res) => {
        this.allFiles.set(res?.items || []);
        this.objectsTotal.set(res?.total || 0);
        this.loadingFiles.set(false);
        this.checkAndStartPolling();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea fișierelor.');
        this.loadingFiles.set(false);
        this.stopPolling();
      }
    });
  }

  onObjectsPageChange(page: number): void {
    this.objectsPage.set(page);
    this.loadObjects();
  }

  ngOnDestroy(): void {
    this.stopPolling();
    this.sub.unsubscribe();
  }

  private checkAndStartPolling(): void {
    const files = this.allFiles();
    const hasActiveFiles = files.some(f => f.status === 'processing' || f.status === 'pending_upload');

    if (hasActiveFiles) {
      // `storage_object_updated` (WS, see ngOnInit) drives instant refresh when a
      // file finishes; this poll is only a fallback while files are still active,
      // so a slow 10s tick is enough.
      if (!this.pollingInterval) {
        this.pollingInterval = setInterval(() => this.reloadObjectsSilent(), 10000);
      }
    } else {
      this.stopPolling();
    }
  }

  private stopPolling(): void {
    if (this.pollingInterval) {
      clearInterval(this.pollingInterval);
      this.pollingInterval = null;
    }
  }

  onSaveSettings(): void {
    const bucket = this.selectedBucket();
    if (!bucket) return;

    this.savingSettings.set(true);

    // Build allowed file types list
    const allowed: string[] = [];
    if (this.allowImages()) {
      allowed.push('image/png', 'image/jpeg', 'image/webp', 'image/svg+xml');
    }
    if (this.allowTextCssJs()) {
      allowed.push('text/plain', 'text/css', 'text/html', 'application/javascript');
    }
    if (this.allowPdfs()) {
      allowed.push('application/pdf');
    }
    if (this.allowArchives()) {
      allowed.push('application/zip', 'application/x-tar', 'application/gzip');
    }
    if (this.allowOthers()) {
      allowed.push('application/octet-stream');
    }

    // Build custom image variants (drop incomplete rows)
    const variants = this.sanitizedVariants();

    // Build processing rules payload
    const payload = {
      name: this.editName().trim(),
      isPublic: this.editIsPublic(),
      maxBucketSizeBytes: this.editMaxSizeGb() * 1024 * 1024 * 1024,
      maxFileSizeBytes: Math.max(0, Math.round(this.editMaxFileSizeMb())) * 1024 * 1024,
      allowCustomProcessing: this.editAllowCustomProcessing(),
      allowedFileTypes: allowed.length > 0 ? allowed : null,
      defaultProcessingRules: {
        imageOptions: {
          convertTo: this.convertImageTo(),
          quality: this.imageQuality(),
          variants,
          forceSquare: this.forceSquare()
        },
        textOptions: {
          preCompressBrotli: this.compressBrotli(),
          preCompressGzip: this.compressGzip()
        }
      }
    };

    this.storageService.updateBucket(bucket.id, payload).subscribe({
      next: (updatedBucket) => {
        this.toast.success('Setările bucket-ului au fost salvate cu succes.');
        this.selectedBucket.set(updatedBucket);
        this.savingSettings.set(false);
        this.loadBuckets(); // reload list
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea setărilor.');
        this.savingSettings.set(false);
      }
    });
  }

  // Activity Log builder (Dynamic audit trail)
  readonly activityLogs = computed(() => {
    const files = this.allFiles();
    const logs: { time: Date; message: string; type: 'info' | 'success' | 'warning' }[] = [];
    
    const bucket = this.selectedBucket();
    if (bucket) {
      logs.push({
        time: new Date(bucket.createdAt),
        message: `Sesiune inițializată: Bucket-ul "${bucket.name}" a fost creat cu tip acces "${bucket.accessType.toUpperCase()}"`,
        type: 'info'
      });
    }

    files.forEach(file => {
      const uploadTime = new Date(file.createdAt);
      logs.push({
        time: uploadTime,
        message: `Fișier încărcat cu succes: "${file.filePath}" (${this.formatBytes(file.sizeBytes)} | Type: ${file.mimeType})`,
        type: 'success'
      });

      if (file.isOptimized) {
        logs.push({
          time: new Date(uploadTime.getTime() + 1500),
          message: `Procesare imagine finalizată: "${file.filePath}" optimizat la WebP/Avif`,
          type: 'info'
        });
      }

      if (file.compression && file.compression !== 'none') {
        logs.push({
          time: new Date(uploadTime.getTime() + 800),
          message: `Optimizare text: "${file.filePath}" pre-comprimat în format ${file.compression.toUpperCase()}`,
          type: 'info'
        });
      }
    });

    return logs.sort((a, b) => b.time.getTime() - a.time.getTime());
  });

  // Parses the flat files list to output files and subfolders in active directory path
  readonly currentItems = computed<VirtualItem[]>(() => {
    // PVCs are browsed per-directory: the server already returns the current
    // directory's contents, so we map them straight to the shared VirtualItem shape.
    const pvc = this.selectedPvc();
    if (pvc) {
      const cp = this.currentPath();
      const q = this.searchQuery().trim().toLowerCase();
      let dir = this.pvcDirItems();
      if (q) dir = dir.filter(it => it.name.toLowerCase().includes(q));
      return dir.map(it => ({
        name: it.name,
        isFolder: it.isDir,
        filePath: `${cp}${it.name}${it.isDir ? '/' : ''}`,
        sizeBytes: it.sizeBytes,
      }));
    }

    const files = this.allFiles();
    const activePath = this.currentPath();
    const query = this.searchQuery().trim().toLowerCase();

    // Check if the current path matches a file folder, e.g. /path/to/file.ext/
    const cleanActivePathForImage = activePath.trim().replace(/^\//, '').replace(/\/$/, '');
    const imageFolderFile = files.find(f => f.hasVariants && f.filePath === cleanActivePathForImage);

    if (imageFolderFile) {
      const items: VirtualItem[] = [];
      
      // 1. Original file
      items.push({
        id: imageFolderFile.id,
        name: `Original (${imageFolderFile.imageDimensions || 'dimensiuni necunoscute'})`,
        isFolder: false,
        filePath: imageFolderFile.filePath,
        sizeBytes: imageFolderFile.sizeBytes,
        mimeType: imageFolderFile.mimeType,
        etag: imageFolderFile.etag,
        status: imageFolderFile.status,
        compression: imageFolderFile.compression,
        originalSizeBytes: imageFolderFile.originalSizeBytes,
        isOptimized: imageFolderFile.isOptimized,
        imageDimensions: imageFolderFile.imageDimensions,
        virtualUrl: this.resolveVirtualUrl(imageFolderFile.virtualUrl),
        createdAt: imageFolderFile.createdAt
      });

      // 2. Variants
      if (imageFolderFile.variants) {
        Object.entries(imageFolderFile.variants).forEach(([key, variant]) => {
          items.push({
            name: `${this.getVariantLabel(key)} (${variant.dimensions})`,
            isFolder: false,
            filePath: variant.filePath,
            sizeBytes: variant.sizeBytes,
            mimeType: `image/${variant.filePath.split('.').pop() || 'png'}`,
            virtualUrl: this.resolveVariantUrl(variant),
            createdAt: imageFolderFile.createdAt
          });
        });
      }

      return items;
    }

    const foldersSet = new Set<string>();
    const matchedFiles: StorageObject[] = [];

    files.forEach(file => {
      const relativePath = file.filePath;
      
      // Global search bypasses folder structure mapping
      if (query) {
        const fileName = relativePath.split('/').pop() || relativePath;
        if (fileName.toLowerCase().includes(query) || relativePath.toLowerCase().includes(query)) {
          matchedFiles.push(file);
        }
        return;
      }

      const cleanActivePath = activePath === '/' ? '' : activePath.trim().replace(/^\//, '').replace(/\/$/, '') + '/';

      if (cleanActivePath === '') {
        if (relativePath.includes('/')) {
          const topFolder = relativePath.split('/')[0];
          foldersSet.add(topFolder);
        } else {
          matchedFiles.push(file);
        }
      } else {
        if (relativePath.startsWith(cleanActivePath)) {
          const subRelative = relativePath.substring(cleanActivePath.length);
          if (subRelative.includes('/')) {
            const nextFolder = subRelative.split('/')[0];
            foldersSet.add(nextFolder);
          } else if (subRelative.length > 0) {
            matchedFiles.push(file);
          }
        }
      }
    });

    const folders: VirtualItem[] = Array.from(foldersSet).map(name => ({
      name,
      isFolder: true,
      filePath: activePath === '/' ? `/${name}/` : `${activePath}${name}/`
    }));

    const items: VirtualItem[] = [
      ...folders,
      ...matchedFiles.map(f => {
        if (f.hasVariants) {
          return {
            id: f.id,
            name: f.filePath.split('/').pop() || f.filePath,
            isFolder: true,
            filePath: activePath === '/' ? `/${f.filePath}/` : `${activePath}${f.filePath.split('/').pop()}/`,
            sizeBytes: f.sizeBytes,
            mimeType: f.mimeType,
            etag: f.etag,
            status: f.status,
            processingStage: f.processingStage,
            compression: f.compression,
            originalSizeBytes: f.originalSizeBytes,
            isOptimized: f.isOptimized,
            imageDimensions: f.imageDimensions,
            hasVariants: f.hasVariants,
            variants: f.variants,
            virtualUrl: this.resolveVirtualUrl(f.virtualUrl),
            createdAt: f.createdAt
          };
        }
        return {
          id: f.id,
          name: f.filePath.split('/').pop() || f.filePath,
          isFolder: false,
          filePath: f.filePath,
          sizeBytes: f.sizeBytes,
          mimeType: f.mimeType,
          etag: f.etag,
          status: f.status,
          processingStage: f.processingStage,
          compression: f.compression,
          originalSizeBytes: f.originalSizeBytes,
          isOptimized: f.isOptimized,
          imageDimensions: f.imageDimensions,
          hasVariants: f.hasVariants,
          variants: f.variants,
          virtualUrl: this.resolveVirtualUrl(f.virtualUrl),
          createdAt: f.createdAt
        };
      })
    ];

    return items;
  });

  // Breadcrumbs generator
  readonly pathParts = computed<string[]>(() => {
    const path = this.currentPath();
    if (path === '/') return [];
    return path.split('/').filter(p => p.length > 0);
  });

  onNavigate(path: string): void {
    this.currentPath.set(path);
    this.searchQuery.set('');
    if (this.selectedPvc()) this.loadPvcDir(path);
  }

  onNavigateBack(): void {
    const path = this.currentPath();
    if (path === '/') return;
    const parts = path.split('/').filter(p => p.length > 0);
    parts.pop();
    const target = parts.length === 0 ? '/' : `/${parts.join('/')}/`;
    this.currentPath.set(target);
    if (this.selectedPvc()) this.loadPvcDir(target);
  }

  onNavigateBreadcrumb(index: number): void {
    const parts = this.pathParts();
    const target = index === -1 ? '/' : `/${parts.slice(0, index + 1).join('/')}/`;
    this.currentPath.set(target);
    this.searchQuery.set('');
    if (this.selectedPvc()) this.loadPvcDir(target);
  }

  onCreateBucket(): void {
    if (!this.newBucketName().trim()) {
      this.toast.error('Numele bucket-ului este obligatoriu.');
      return;
    }

    if (this.publishAppId() && !this.appIdEnvKeyName().trim()) {
      this.toast.error('Numele variabilei de mediu pentru App ID este obligatoriu dacă este bifat.');
      return;
    }
    if (this.publishSecretKey() && !this.secretKeyEnvKeyName().trim()) {
      this.toast.error('Numele variabilei de mediu pentru Secret Key este obligatoriu dacă este bifat.');
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
        this.toast.success(`Bucket-ul "${newBucket.name}" a fost creat cu succes.`);
        this.newBucketName.set('');
        this.appIdEnvKeyName.set('');
        this.secretKeyEnvKeyName.set('');
        this.publishAppId.set(false);
        this.publishSecretKey.set(false);
        this.maxFileSizeMb.set(0);
        this.allowCustomProcessing.set(false);
        this.showCreateForm.set(false);
        this.creatingBucket.set(false);
        this.storageService.listBuckets().subscribe(list => {
          this.buckets.set(list || []);
          this.selectBucket(newBucket);
        });
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea bucket-ului.');
        this.creatingBucket.set(false);
      }
    });
  }

  async onDeleteBucket(bucket: StorageBucket): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Bucket de Stocare',
      message: `Sigur doriți să ștergeți complet bucket-ul "${bucket.name}"? Toate fișierele conținute, folderele virtuale, configurațiile Nginx și DNS atașate vor fi distruse definitiv!`,
      confirmText: 'Șterge definitiv',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.loading.set(true);
    this.storageService.deleteBucket(bucket.id).subscribe({
      next: () => {
        this.toast.success(`Bucket-ul "${bucket.name}" a fost șters.`);
        this.deselectBucket();
        this.loadBuckets();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea bucket-ului.');
        this.loading.set(false);
      }
    });
  }

  async onRotateBucketCredentials(bucket: StorageBucket): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Rotește credențialele bucket-ului',
      message: `Se generează un secret_key nou pentru "${bucket.name}" (app_id-ul rămâne neschimbat). Aplicațiile care folosesc cheia veche vor fi respinse până le reîncarci manual. Continuați?`,
      confirmText: 'Rotește',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.rotatingCreds.set(true);
    this.rotatedSecret.set(null);
    this.storageService.rotateCredentials(bucket.id).subscribe({
      next: (res) => {
        this.rotatingCreds.set(false);
        this.rotatedSecret.set(res.secret_key);
        this.toast.success('Credențiale rotite. Salvează noul secret_key — nu va mai fi afișat. Aplicațiile au nevoie de reload manual.');
      },
      error: (err) => {
        this.rotatingCreds.set(false);
        this.toast.error(err.error?.message || 'Eroare la rotația credențialelor.');
      }
    });
  }

  onCreateFolder(): void {
    const name = this.newFolderName().trim().replace(/[\/\\]/g, '');
    if (!name) return;

    const activePath = this.currentPath();
    const cleanActivePath = activePath === '/' ? '' : activePath.trim().replace(/^\//, '').replace(/\/$/, '') + '/';
    
    // Create folder by initializing a virtual placeholder .keep file in backend
    const placeholderPath = `${cleanActivePath}${name}/.keep`;
    const bucket = this.selectedBucket();
    if (!bucket) return;

    this.uploading.set(true);
    this.uploadProgress.set(0);

    const fullPath = `${bucket.slug}/${placeholderPath}`;
    
    this.storageService.initializeUpload({
      filePath: fullPath,
      sizeBytes: 0,
      mimeType: 'text/plain'
    }).subscribe({
      next: (res) => {
        const dummyFile = new File([], '.keep', { type: 'text/plain' });
        this.storageService.uploadFileStream(res.uploadUrl, dummyFile).subscribe({
          next: (event: any) => {
            if (event.type === HttpEventType.Response) {
              this.uploading.set(false);
              this.newFolderName.set('');
              this.showFolderForm.set(false);
              this.toast.success(`Dosarul virtual "${name}" a fost creat.`);
              this.loadObjects();
            }
          },
          error: (err) => {
            this.uploading.set(false);
            this.toast.error(err.error?.message || 'Eroare la crearea dosarului.');
          }
        });
      },
      error: (err) => {
        this.uploading.set(false);
        this.toast.error(err.error?.message || 'Eroare la crearea folderului virtual.');
      }
    });
  }

  onFileSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    if (!input.files || input.files.length === 0) return;
    
    const file = input.files[0];
    this.uploadFile(file);
  }

  uploadFile(file: File): void {
    const bucket = this.selectedBucket();
    if (!bucket) return;

    // Enforce the per-file size limit client-side before opening an upload session.
    if (bucket.maxFileSizeBytes > 0 && file.size > bucket.maxFileSizeBytes) {
      this.toast.error(`Fișierul "${file.name}" (${this.formatBytes(file.size)}) depășește limita de ${this.formatBytes(bucket.maxFileSizeBytes)} per fișier.`);
      return;
    }

    const activePath = this.currentPath();
    const cleanActivePath = activePath === '/' ? '' : activePath.trim().replace(/^\//, '').replace(/\/$/, '') + '/';
    const relativePath = `${cleanActivePath}${file.name}`;
    const fullPath = `${bucket.slug}/${relativePath}`;

    this.uploading.set(true);
    this.uploadProgress.set(0);

    this.storageService.initializeUpload({
      filePath: fullPath,
      sizeBytes: file.size,
      mimeType: file.type || 'application/octet-stream'
    }).subscribe({
      next: (res) => {
        this.storageService.uploadFileStream(res.uploadUrl, file).subscribe({
          next: (event: HttpEvent<any>) => {
            if (event.type === HttpEventType.UploadProgress) {
              const percent = event.total ? Math.round(100 * event.loaded / event.total) : 0;
              this.uploadProgress.set(percent);
            } else if (event.type === HttpEventType.Response) {
              this.uploading.set(false);
              this.toast.success(`Fișierul "${file.name}" a fost încărcat cu succes!`);
              this.loadObjects();
            }
          },
          error: (err) => {
            this.uploading.set(false);
            this.toast.error(err.error?.message || 'Eroare la transferul datelor.');
          }
        });
      },
      error: (err) => {
        this.uploading.set(false);
        this.toast.error(err.error?.message || 'Eroare la inițializarea sesiunii.');
      }
    });
  }

  async onDeleteFile(item: VirtualItem): Promise<void> {
    if (!item.id) return;
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Fișier',
      message: `Sigur doriți să ștergeți fișierul "${item.name}"? Această acțiune este ireversibilă și va șterge fișierele comprimate și variantele de pe disk/S3!`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.storageService.deleteObject(item.id).subscribe({
      next: () => {
        this.toast.success(`Fișierul "${item.name}" a fost eliminat.`);
        this.loadObjects();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea fișierului.');
      }
    });
  }

  async onCancelUpload(item: VirtualItem): Promise<void> {
    if (!item.id) return;
    const confirmed = await this.confirm.ask({
      title: 'Anulare Încărcare',
      message: `Sigur doriți să anulați încărcarea/procesarea fișierului "${item.name}"?`,
      confirmText: 'Anulează',
      cancelText: 'Păstrează',
      isDanger: true
    });
    if (!confirmed) return;

    this.storageService.deleteObject(item.id).subscribe({
      next: () => {
        this.toast.success(`Încărcarea fișierului "${item.name}" a fost anulată.`);
        this.loadObjects();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la anularea încărcării.');
      }
    });
  }

  resolveVirtualUrl(url: string): string {
    if (!url) return '';
    if (url.startsWith('/')) {
      const token = localStorage.getItem('hermes_token') || '';
      return `${environment.apiOrigin}${url}?token=${encodeURIComponent(token)}`;
    }
    return url;
  }

  resolveVariantUrl(variant: ImageVariant): string {
    const bucket = this.selectedBucket();
    if (!bucket || !variant.filePath) return '';
    // All buckets are private — serve through the API with an auth token.
    const token = localStorage.getItem('hermes_token') || '';
    return `${environment.apiOrigin}/storage/assets/${this.parent.project()?.workspace_id}/${bucket.slug}/${variant.filePath}?token=${encodeURIComponent(token)}`;
  }

  toggleVariants(itemId: string): void {
    const current = new Set(this.expandedVariants());
    if (current.has(itemId)) {
      current.delete(itemId);
    } else {
      current.add(itemId);
    }
    this.expandedVariants.set(current);
  }

  isVariantsExpanded(itemId: string): boolean {
    return this.expandedVariants().has(itemId);
  }

  getVariantEntries(variants: Record<string, ImageVariant> | null | undefined): { key: string, value: ImageVariant }[] {
    if (!variants) return [];
    return Object.entries(variants).map(([key, value]) => ({ key, value }));
  }

  // --- Custom image variants editor ---
  addVariant(): void {
    this.customVariants.update(list => [...list, { name: '', maxWidth: 400, format: 'webp' as ImageFormatTarget }]);
  }

  removeVariant(index: number): void {
    this.customVariants.update(list => list.filter((_, i) => i !== index));
  }

  updateVariant(index: number, patch: Partial<ImageVariantSpec>): void {
    this.customVariants.update(list => list.map((v, i) => i === index ? { ...v, ...patch } : v));
  }

  // Drop incomplete rows (no name / non-positive width) before persisting.
  private sanitizedVariants(): ImageVariantSpec[] {
    return this.customVariants()
      .map(v => ({ name: (v.name || '').trim(), maxWidth: Math.round(Number(v.maxWidth) || 0), format: v.format }))
      .filter(v => v.name.length > 0 && v.maxWidth > 0);
  }

  // Human label for the live processing stage surfaced via polling.
  stageLabel(stage?: string | null): string {
    if (!stage) return 'Procesare...';
    if (stage.startsWith('variant:')) return `Variantă: ${stage.slice('variant:'.length)}`;
    const map: Record<string, string> = {
      analyzing: 'Analiză fișier',
      converting: 'Conversie format',
      compressing: 'Compresie',
      finalizing: 'Finalizare / sync',
    };
    return map[stage] || 'Procesare...';
  }

  getVariantLabel(key: string): string {
    const labels: Record<string, string> = {
      'xs': 'Thumbnail (150px)',
      's': 'Mic (400px)',
      'md': 'Mediu (800px)',
      'lg': 'Mare (1200px)'
    };
    return labels[key] || key;
  }

  copyPublicUrl(item: VirtualItem): void {
    if (!item.virtualUrl) return;
    navigator.clipboard.writeText(item.virtualUrl).then(() => {
      this.toast.success('Adresa a fost copiată în clipboard!');
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }

  isImage(mimeType?: string): boolean {
    if (!mimeType) return false;
    return mimeType.startsWith('image/') && mimeType !== 'image/gif';
  }

  formatBytes(bytes?: number): string {
    if (bytes === undefined) return '0 B';
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  }

  getEnvSnippet(): string {
    // Credentials are published to the project env pool as BUCKET_<slug>_APP_ID /
    // BUCKET_<slug>_SECRET_KEY (rotate them in the bucket's Settings tab).
    return `# Hermes Storage — credentials come from your project's env pool\n# BUCKET_<slug>_APP_ID, BUCKET_<slug>_SECRET_KEY (rotate in Settings)\nHERMES_STORAGE_URL=${environment.apiOrigin}/storage`;
  }

  getUploadSnippet(): string {
    const slug = this.selectedBucket()?.slug || '';
    return `# 1. Inițializare upload\ncurl -X POST ${environment.apiBaseUrl}/storage/upload/init \\\n  -H "Authorization: Bearer YOUR_TOKEN" \\\n  -H "Content-Type: application/json" \\\n  -d '{"bucketSlug": "${slug}", "filePath": "/images/photo.jpg", "mimeType": "image/jpeg"}' \n\n# 2. Upload cu ID-ul primit\ncurl -X PUT ${environment.apiBaseUrl}/storage/upload/{file_id} \\\n  -H "Content-Type: application/octet-stream" \\\n  --data-binary @photo.jpg`;
  }

  getNodeSnippet(): string {
    return `const HERMES_URL = '${environment.apiBaseUrl}/storage';
const TOKEN = 'YOUR_TOKEN';

async function uploadFile(bucketSlug, filePath, buffer, mimeType) {
  // Step 1: Initialize upload
  const init = await fetch(\`\${HERMES_URL}/upload/init\`, {
    method: "POST",
    headers: {
      "Authorization": \`Bearer \${TOKEN}\`,
      "Content-Type": "application/json"
    },
    body: JSON.stringify({ bucketSlug, filePath, mimeType })
  });
  const { fileId } = await init.json();

  // Step 2: Stream file bytes
  await fetch(\`\${HERMES_URL}/upload/\${fileId}\`, {
    method: "PUT",
    headers: { "Content-Type": "application/octet-stream" },
    body: buffer
  });

  return fileId;
}`;
  }

  getListSnippet(): string {
    const slug = this.selectedBucket()?.slug || '';
    return `curl -X GET ${environment.apiBaseUrl}/storage/buckets/${slug}/objects \\\n  -H "Authorization: Bearer YOUR_TOKEN"`;
  }

  getDownloadSnippet(): string {
    return `curl -X GET ${environment.apiBaseUrl}/storage/private/{file_id}?token=YOUR_TOKEN -o output_file`;
  }
}
