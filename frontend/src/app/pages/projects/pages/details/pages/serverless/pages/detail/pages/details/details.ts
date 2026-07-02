import { Component, inject } from '@angular/core';

import { ServerlessDetailComponent } from '../../detail';
import { ServerlessRoute } from '../../../../../../../../../../core/services/project.service';

@Component({
  selector: 'app-serverless-details',
  imports: [],
  templateUrl: './details.html',
  styles: ``,
})
export class ServerlessDetailsComponent {
  readonly detailParent = inject(ServerlessDetailComponent);

  invokeUrl(r: ServerlessRoute): string {
    const inst = this.detailParent.selectedInstance();
    if (!inst) return '';
    const base = inst.assignedDomain ? `https://${inst.assignedDomain}` : (inst.externalPort ? `http://localhost:${inst.externalPort}` : '');
    return base + r.routePath;
  }
}
