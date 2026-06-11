import { Component, inject, signal, OnInit, OnDestroy, effect, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { StorageService, StorageBucket, StorageObject, BucketAccessType, ImageVariant, ProjectVolume } from '../../../../../../core/services/storage.service';
import { VolumeService, VolumeFileItem } from '../../../../../../core/services/volume.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { HttpEvent, HttpEventType } from '@angular/common/http';

export interface VirtualItem {
  id?: string;
  name: string;
  isFolder: boolean;
  filePath: string;
  sizeBytes?: number;
  mimeType?: string;
  etag?: string;
  status?: string;
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
  imports: [CommonModule, FormsModule, DatePipe],
  templateUrl: './storages.html',
  styleUrl: './storages.css',
})
export class Storages implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly storageService = inject(StorageService);
  private readonly volumeService = inject(VolumeService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  // PVCs (app volumes) listed + browsed centrally in Storage
  readonly pvcs = signal<ProjectVolume[]>([]);
  readonly loadingPvcs = signal(false);
  readonly pvcExplorer = signal<ProjectVolume | null>(null);
  readonly pvcFiles = signal<VolumeFileItem[]>([]);
  readonly loadingPvcFiles = signal(false);
  readonly pvcPath = signal<string>('/');

  readonly buckets = signal<StorageBucket[]>([]);
  readonly selectedBucket = signal<StorageBucket | null>(null);
  readonly allFiles = signal<StorageObject[]>([]);
  
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
  readonly newBucketAccessType = signal<BucketAccessType>('public_assets');
  readonly maxBucketSizeGb = signal<number>(1);
  readonly isPublicToggle = signal<boolean>(false);

  // Upload states
  readonly uploading = signal(false);
  readonly uploadProgress = signal<number>(0);

  // Virtual Folder Creation Form states
  readonly showFolderForm = signal(false);
  readonly newFolderName = signal('');

  // Edit Bucket Settings Form States
  readonly editName = signal('');
  readonly editAccessType = signal<BucketAccessType>('public_assets');
  readonly editMaxSizeGb = signal<number>(1);
  readonly editIsPublic = signal<boolean>(false);
  readonly savingSettings = signal(false);

  // Advanced Image Rules
  readonly convertImageTo = signal<'original' | 'webp' | 'avif'>('original');
  readonly imageQuality = signal<number>(85);
  readonly forceSquare = signal<boolean>(false);
  readonly genThumbnail = signal<boolean>(false);
  readonly genSmall = signal<boolean>(false);
  readonly genMedium = signal<boolean>(false);
  readonly genLarge = signal<boolean>(false);

  // Text Compression
  readonly compressBrotli = signal<boolean>(false);
  readonly compressGzip = signal<boolean>(false);

  // Integration API Token
  readonly integrationToken = signal<string | null>(null);
  readonly integrationTokenExpiry = signal<string | null>(null);
  readonly generatingToken = signal(false);

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

  openPvcExplorer(pvc: ProjectVolume): void {
    this.pvcExplorer.set(pvc);
    this.pvcPath.set('/');
    this.loadPvcFiles(pvc.id, '/');
  }

  closePvcExplorer(): void {
    this.pvcExplorer.set(null);
    this.pvcFiles.set([]);
    this.pvcPath.set('/');
  }

  loadPvcFiles(volumeId: string, path: string): void {
    this.loadingPvcFiles.set(true);
    this.volumeService.listFiles(volumeId, path).subscribe({
      next: (res) => { this.pvcFiles.set(res || []); this.loadingPvcFiles.set(false); },
      error: () => { this.pvcFiles.set([]); this.loadingPvcFiles.set(false); }
    });
  }

  pvcNavigateTo(folderName: string): void {
    const pvc = this.pvcExplorer();
    if (!pvc) return;
    const base = this.pvcPath().endsWith('/') ? this.pvcPath() : this.pvcPath() + '/';
    const next = `${base}${folderName}`;
    this.pvcPath.set(next);
    this.loadPvcFiles(pvc.id, next);
  }

  pvcNavigateUp(): void {
    const pvc = this.pvcExplorer();
    if (!pvc) return;
    const parts = this.pvcPath().split('/').filter(p => p.length > 0);
    parts.pop();
    const parent = '/' + parts.join('/');
    this.pvcPath.set(parent);
    this.loadPvcFiles(pvc.id, parent);
  }

  pvcBreadcrumbs(): { name: string; path: string }[] {
    const parts = this.pvcPath().split('/').filter(p => p.length > 0);
    const crumbs: { name: string; path: string }[] = [];
    let acc = '';
    for (const p of parts) {
      acc += '/' + p;
      crumbs.push({ name: p, path: acc });
    }
    return crumbs;
  }

  pvcGoToCrumb(path: string): void {
    const pvc = this.pvcExplorer();
    if (!pvc) return;
    this.pvcPath.set(path);
    this.loadPvcFiles(pvc.id, path);
  }

  pvcDownload(item: VolumeFileItem): void {
    const pvc = this.pvcExplorer();
    if (!pvc || item.isDir) return;
    const base = this.pvcPath().endsWith('/') ? this.pvcPath() : this.pvcPath() + '/';
    const url = this.volumeService.downloadFileUrl(pvc.id, `${base}${item.name}`);
    const a = document.createElement('a');
    a.href = url;
    a.download = item.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
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
    this.selectedBucket.set(bucket);
    this.activeTab.set('files');
    this.currentPath.set('/');

    // Populate form states
    this.editName.set(bucket.name);
    this.editAccessType.set(bucket.accessType);
    this.editMaxSizeGb.set(Math.round(bucket.maxBucketSizeBytes / (1024 * 1024 * 1024)));
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
      const vars = imgOpts.generateVariants || [];
      this.genThumbnail.set(vars.includes('xs'));
      this.genSmall.set(vars.includes('s'));
      this.genMedium.set(vars.includes('md'));
      this.genLarge.set(vars.includes('lg'));
    } else {
      this.convertImageTo.set('original');
      this.imageQuality.set(85);
      this.forceSquare.set(false);
      this.genThumbnail.set(false);
      this.genSmall.set(false);
      this.genMedium.set(false);
      this.genLarge.set(false);
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
    this.storageService.listObjects(bucket.slug).subscribe({
      next: (res) => {
        this.allFiles.set(res || []);
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

  ngOnDestroy(): void {
    this.stopPolling();
  }

  private checkAndStartPolling(): void {
    const files = this.allFiles();
    const hasActiveFiles = files.some(f => f.status === 'processing' || f.status === 'pending_upload');

    if (hasActiveFiles) {
      if (!this.pollingInterval) {
        this.pollingInterval = setInterval(() => {
          const bucket = this.selectedBucket();
          if (!bucket) {
            this.stopPolling();
            return;
          }
          this.storageService.listObjects(bucket.slug).subscribe({
            next: (res) => {
              this.allFiles.set(res || []);
              this.checkAndStartPolling();
            },
            error: () => {
              this.stopPolling();
            }
          });
        }, 2000);
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

    // Build image variants list
    const variants: string[] = [];
    if (this.genThumbnail()) variants.push('xs');
    if (this.genSmall()) variants.push('s');
    if (this.genMedium()) variants.push('md');
    if (this.genLarge()) variants.push('lg');

    // Build processing rules payload
    const payload = {
      name: this.editName().trim(),
      isPublic: this.editIsPublic(),
      maxBucketSizeBytes: this.editMaxSizeGb() * 1024 * 1024 * 1024,
      allowedFileTypes: allowed.length > 0 ? allowed : null,
      defaultProcessingRules: {
        imageOptions: {
          convertTo: this.convertImageTo(),
          quality: this.imageQuality(),
          generateVariants: variants,
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
  }

  onNavigateBack(): void {
    const path = this.currentPath();
    if (path === '/') return;
    const parts = path.split('/').filter(p => p.length > 0);
    parts.pop();
    if (parts.length === 0) {
      this.currentPath.set('/');
    } else {
      this.currentPath.set(`/${parts.join('/')}/`);
    }
  }

  onNavigateBreadcrumb(index: number): void {
    const parts = this.pathParts();
    if (index === -1) {
      this.currentPath.set('/');
    } else {
      const targetParts = parts.slice(0, index + 1);
      this.currentPath.set(`/${targetParts.join('/')}/`);
    }
    this.searchQuery.set('');
  }

  onCreateBucket(): void {
    if (!this.newBucketName().trim()) {
      this.toast.error('Numele bucket-ului este obligatoriu.');
      return;
    }

    this.creatingBucket.set(true);
    this.storageService.createBucket({
      name: this.newBucketName().trim(),
      projectId: this.parent.projectId() || undefined,
      isPublic: this.isPublicToggle(),
      maxBucketSizeBytes: this.maxBucketSizeGb() * 1024 * 1024 * 1024
    }).subscribe({
      next: (newBucket) => {
        this.toast.success(`Bucket-ul "${newBucket.name}" a fost creat cu succes.`);
        this.newBucketName.set('');
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

  resolveVirtualUrl(url: string): string {
    if (!url) return '';
    if (url.startsWith('/')) {
      const token = localStorage.getItem('hermes_token') || '';
      return `http://localhost:8000${url}?token=${encodeURIComponent(token)}`;
    }
    return url;
  }

  resolveVariantUrl(variant: ImageVariant): string {
    const bucket = this.selectedBucket();
    if (!bucket || !variant.filePath) return '';
    // All buckets are private — serve through the API with an auth token.
    const token = localStorage.getItem('hermes_token') || '';
    return `http://localhost:8000/storage/assets/${this.parent.project()?.workspace_id}/${bucket.slug}/${variant.filePath}?token=${encodeURIComponent(token)}`;
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

  generateToken(): void {
    const bucket = this.selectedBucket();
    if (!bucket) return;

    this.generatingToken.set(true);
    this.storageService.generateBucketToken(bucket.id).subscribe({
      next: (res) => {
        this.integrationToken.set(res.token);
        this.integrationTokenExpiry.set(res.expiresAt);
        this.generatingToken.set(false);
        this.toast.success('Tokenul de integrare a fost generat cu succes.');
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la generarea token-ului.');
        this.generatingToken.set(false);
      }
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
    const token = this.integrationToken() || '';
    return `# Configurare Hermes Storage\nHERMES_STORAGE_URL=http://localhost:8000/storage\nHERMES_STORAGE_TOKEN=${token}`;
  }

  getUploadSnippet(): string {
    const slug = this.selectedBucket()?.slug || '';
    return `# 1. Inițializare upload\ncurl -X POST http://localhost:8000/api/v1/storage/upload/init \\\n  -H "Authorization: Bearer YOUR_TOKEN" \\\n  -H "Content-Type: application/json" \\\n  -d '{"bucketSlug": "${slug}", "filePath": "/images/photo.jpg", "mimeType": "image/jpeg"}' \n\n# 2. Upload cu ID-ul primit\ncurl -X PUT http://localhost:8000/api/v1/storage/upload/{file_id} \\\n  -H "Content-Type: application/octet-stream" \\\n  --data-binary @photo.jpg`;
  }

  getNodeSnippet(): string {
    return `const HERMES_URL = 'http://localhost:8000/api/v1/storage';
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
    return `curl -X GET http://localhost:8000/api/v1/storage/buckets/${slug}/objects \\\n  -H "Authorization: Bearer YOUR_TOKEN"`;
  }

  getDownloadSnippet(): string {
    return `curl -X GET http://localhost:8000/api/v1/storage/private/{file_id}?token=YOUR_TOKEN -o output_file`;
  }
}
