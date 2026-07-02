import { Component, inject } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { NetworkingDetail } from '../../networking-detail';

@Component({
  selector: 'app-networking-detail-settings',
  imports: [FormsModule],
  templateUrl: './settings.html',
})
export class NetworkingDetailSettings {
  readonly parent = inject(NetworkingDetail);

  get route() {
    return this.parent.selectedRoute()!;
  }
}
