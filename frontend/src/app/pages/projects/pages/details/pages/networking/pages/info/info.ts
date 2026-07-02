import { Component, inject } from '@angular/core';
import { NetworkingDetail } from '../../networking-detail';

@Component({
  selector: 'app-networking-detail-info',
  imports: [],
  templateUrl: './info.html',
})
export class NetworkingDetailInfo {
  readonly parent = inject(NetworkingDetail);

  get route() {
    return this.parent.selectedRoute()!;
  }
}
