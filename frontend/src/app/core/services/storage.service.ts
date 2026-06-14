import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';
import { environment } from '../../../environments/environment';
import { Paginated, DEFAULT_PAGE_SIZE } from '../models/pagination';

export type BucketAccessType = 'static_website' | 'public_assets' | 'private_storage' | 'app_bounded';
export type StorageStatus = 'pending_upload' | 'ready' | 'processing' | 'failed';
export type CompressionType = 'none' | 'gzip' | 'brotli';

export type ImageFormatTarget = 'original' | 'webp' | 'avif' | 'jpg';

export interface ImageVariantSpec {
  name: string;
  maxWidth: number;
  format: ImageFormatTarget;
}

export interface ImageProcessingOptions {
  convertTo: ImageFormatTarget;
  quality: number;
  variants: ImageVariantSpec[];
  forceSquare: boolean;
}

export interface TextProcessingOptions {
  preCompressBrotli: boolean;
  preCompressGzip: boolean;
}

export interface BucketProcessingRules {
  imageOptions?: ImageProcessingOptions | null;
  textOptions?: TextProcessingOptions | null;
}

export interface StorageBucket {
  id: string;
  name: string;
  slug: string;
  accessType: BucketAccessType;
  isPublic: boolean;
  assignedDomain: string | null;
  allowedFileTypes: string[] | null;
  maxBucketSizeBytes: number;
  maxFileSizeBytes: number;
  allowCustomProcessing: boolean;
  defaultProcessingRules: BucketProcessingRules;
  createdAt: string;
}

export interface ImageVariant {
  filePath: string;
  sizeBytes: number;
  dimensions: string;
}

export interface StorageObject {
  id: string;
  bucketId: string;
  filePath: string;
  sizeBytes: number;
  mimeType: string;
  etag: string;
  status: StorageStatus;
  processingStage?: string | null;
  compression: CompressionType;
  originalSizeBytes: number | null;
  isOptimized: boolean;
  imageDimensions: string | null;
  hasVariants: boolean;
  variants: Record<string, ImageVariant> | null;
  virtualUrl: string;
  createdAt: string;
}

export interface CreateBucketRequest {
  name: string;
  projectId?: string;
  isPublic?: boolean;
  allowedFileTypes?: string[];
  maxBucketSizeBytes?: number;
  maxFileSizeBytes?: number;
  allowCustomProcessing?: boolean;
  publishToEnv?: boolean;
  envKey?: string;
}

// A PVC listed in the central Storage interface (auto-created at app build).
export interface ProjectVolume {
  id: string;
  appId: string;
  appName: string;
  name: string;
  containerPath: string;
  hostPath: string;
  isAuto: boolean;
}

export interface InitUploadRequest {
  filePath: string;
  sizeBytes: number;
  mimeType: string;
}

export interface InitUploadResponse {
  fileId: string;
  status: StorageStatus;
  uploadUrl: string;
}

@Injectable({
  providedIn: 'root'
})
export class StorageService {
  private readonly api = inject(ApiService);
  private readonly http = inject(HttpClient);
  private readonly baseUrl = environment.apiBaseUrl;

  listBuckets(): Observable<StorageBucket[]> {
    return this.api.get<StorageBucket[]>('/storage/buckets');
  }

  // PVCs (app volumes) across the project — created only at app build time.
  listProjectVolumes(projectId: string): Observable<ProjectVolume[]> {
    return this.api.get<ProjectVolume[]>(`/projects/${projectId}/volumes`);
  }

  createBucket(payload: CreateBucketRequest): Observable<StorageBucket> {
    return this.api.post<StorageBucket>('/storage/buckets', payload);
  }

  deleteBucket(bucketId: string): Observable<void> {
    return this.api.delete<void>(`/storage/buckets/${bucketId}`);
  }

  updateBucket(bucketId: string, payload: any): Observable<StorageBucket> {
    return this.api.patch<StorageBucket>(`/storage/buckets/${bucketId}`, payload);
  }

  listObjects(bucketSlug: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<StorageObject>> {
    return this.api.get<Paginated<StorageObject>>(`/storage/buckets/${bucketSlug}/objects?page=${page}&pageSize=${pageSize}`);
  }

  deleteObject(objectId: string): Observable<void> {
    return this.api.delete<void>(`/storage/objects/${objectId}`);
  }

  initializeUpload(payload: InitUploadRequest): Observable<InitUploadResponse> {
    return this.api.post<InitUploadResponse>('/storage/upload/init', payload);
  }

  uploadFileStream(uploadUrl: string, file: File): Observable<any> {
    const token = localStorage.getItem('hermes_token');
    let headers = new HttpHeaders();
    if (token) {
      headers = headers.set('Authorization', `Bearer ${token}`);
    }
    headers = headers.set('Content-Type', file.type || 'application/octet-stream');

    return this.http.post(`${this.baseUrl}${uploadUrl}`, file, {
      headers,
      reportProgress: true,
      observe: 'events'
    });
  }

  getUploadProgressStreamUrl(fileId: string): string {
    return this.api.getStreamUrl(`/storage/upload/${fileId}/progress`);
  }

  generateBucketToken(bucketId: string): Observable<{ token: string, expiresAt: string }> {
    return this.api.post<{ token: string, expiresAt: string }>(`/storage/buckets/${bucketId}/token`, {});
  }
}
