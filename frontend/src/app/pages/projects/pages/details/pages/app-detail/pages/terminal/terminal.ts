import { Component, inject, OnInit } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-terminal',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './terminal.html',
  styles: ``,
})
export class AppTerminalComponent implements OnInit {
  readonly parent = inject(AppDetailComponent);

  ngOnInit(): void {
    this.parent.focusTerminalInput();
    this.parent.initializeTerminalCwd();
  }
}
