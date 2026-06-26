import { Component, inject, signal, AfterViewChecked } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { DbDetailComponent } from '../../db-detail';
import { DatabaseService } from '../../../../../../../../core/services/database.service';

@Component({
  selector: 'app-db-console',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './console.html',
  styles: ``,
})
export class DbConsoleComponent implements AfterViewChecked {
  readonly dbDetail = inject(DbDetailComponent);
  private readonly dbService = inject(DatabaseService);

  readonly queryInput = signal('');
  readonly queryLoading = signal(false);
  readonly queryHistory = signal<{ query: string; output: string; isError: boolean; timestamp: Date }[]>([]);

  ngAfterViewChecked(): void {
    this.scrollConsoleToBottom();
  }

  onRunQuery(): void {
    const id = this.dbDetail.dbId();
    const query = this.queryInput().trim();
    if (!id || !query) return;

    this.queryLoading.set(true);
    this.dbService.runQuery(id, query).subscribe({
      next: (res) => {
        this.queryHistory.update(history => [
          ...history,
          {
            query,
            output: res.output,
            isError: res.isError,
            timestamp: new Date()
          }
        ]);
        this.queryInput.set('');
        this.queryLoading.set(false);
        setTimeout(() => this.scrollConsoleToBottom(), 50);
      },
      error: (err) => {
        this.queryHistory.update(history => [
          ...history,
          {
            query,
            output: err.error?.message || 'Failed to communicate with the database.',
            isError: true,
            timestamp: new Date()
          }
        ]);
        this.queryLoading.set(false);
        setTimeout(() => this.scrollConsoleToBottom(), 50);
      }
    });
  }

  clearConsole(): void {
    this.queryHistory.set([]);
  }

  scrollConsoleToBottom(): void {
    const el = document.getElementById('query-terminal-window');
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }
}
