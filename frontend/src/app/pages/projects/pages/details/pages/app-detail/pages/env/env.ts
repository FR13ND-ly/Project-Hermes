import { Component, inject, OnInit } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-env',
  imports: [FormsModule],
  templateUrl: './env.html',
  styles: ``,
})
export class AppEnvComponent implements OnInit {
  readonly parent = inject(AppDetailComponent);

  ngOnInit(): void {
    this.parent.loadEnvVariables();
  }
}
