import { Injectable, signal } from '@angular/core';

/**
 * Lets a resource-detail page (app, database, bucket, cron, function, route,
 * auth service) publish the display name of the resource it is showing, so the
 * project-level breadcrumb can render "Projects / project / Section / name"
 * instead of stopping at the project. Detail components set it once their
 * resource loads and clear it on destroy; the breadcrumb falls back to an
 * ellipsis while the name is unknown.
 */
@Injectable({ providedIn: 'root' })
export class BreadcrumbService {
  readonly resourceName = signal<string | null>(null);

  set(name: string | null | undefined): void {
    this.resourceName.set(name || null);
  }

  clear(): void {
    this.resourceName.set(null);
  }
}
