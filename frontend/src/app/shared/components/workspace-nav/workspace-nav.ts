import { Component } from '@angular/core';
import { RouterLink, RouterLinkActive } from '@angular/router';

/**
 * In-page navigation for the workspace context: Projects ↔ Workspace Settings.
 * Lives at the top of the Dashboard and Workspace Settings pages so these two views
 * are reachable from inside the workspace rather than from the global app header.
 */
@Component({
  selector: 'app-workspace-nav',
  standalone: true,
  imports: [RouterLink, RouterLinkActive],
  template: `
    <nav class="flex items-center gap-6 border-b border-zinc-900 mb-6" aria-label="Workspace">
      <a
        routerLink="/dashboard"
        routerLinkActive="text-zinc-50! border-zinc-50!"
        [routerLinkActiveOptions]="{ exact: true }"
        class="text-[13px] font-semibold text-zinc-400 hover:text-zinc-200 transition-colors py-2.5 border-b-2 border-transparent -mb-px"
      >
        Projects
      </a>
      <a
        routerLink="/workspace/settings"
        routerLinkActive="text-zinc-50! border-zinc-50!"
        class="text-[13px] font-semibold text-zinc-400 hover:text-zinc-200 transition-colors py-2.5 border-b-2 border-transparent -mb-px"
      >
        Workspace Settings
      </a>
    </nav>
  `,
})
export class WorkspaceNav {}
