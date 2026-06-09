import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export type BucketAccessType = 'static_website' | 'public_assets' | 'private_storage' | 'app_bounded';
export type StorageStatus = 'pending_upload' | 'ready' | 'processing' | 'failed';
export type CompressionType = 'none' | 'gzip' | 'brotli';

export interface ImageProcessingOptions {
  convertTo: 'original' | 'webp' | 'avif';
  quality: number;
  generateVariants: string[];
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
  accessType: BucketAccessType;
  isPublic?: boolean;
  allowedFileTypes?: string[];
  maxBucketSizeBytes?: number;
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
  private readonly baseUrl = 'http://localhost:8000/api/v1';

  listBuckets(): Observable<StorageBucket[]> {
    return this.api.get<StorageBucket[]>('/storage/buckets');
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

  listObjects(bucketSlug: string): Observable<StorageObject[]> {
    return this.api.get<StorageObject[]>(`/storage/buckets/${bucketSlug}/objects`);
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
