import { Component, inject } from '@angular/core';
import { NetworkingDetail } from '../../networking-detail';

@Component({
  selector: 'app-networking-detail-logs',
  imports: [],
  templateUrl: './logs.html',
})
export class NetworkingDetailLogs {
  readonly parent = inject(NetworkingDetail);

  get route() {
    return this.parent.selectedRoute()!;
  }
}
