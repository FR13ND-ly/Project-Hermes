import { Component, inject, OnInit } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-env',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './env.html',
  styles: ``,
})
export class AppEnvComponent implements OnInit {
  readonly parent = inject(AppDetailComponent);

  ngOnInit(): void {
    this.parent.loadEnvVariables();
  }
}
