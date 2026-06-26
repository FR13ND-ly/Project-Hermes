import { Component, inject, signal, OnInit, OnDestroy, AfterViewChecked, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { DbDetailComponent } from '../../db-detail';
import { DatabaseService } from '../../../../../../../../core/services/database.service';

@Component({
  selector: 'app-db-logs',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './logs.html',
  styles: ``,
})
export class DbLogsComponent implements OnInit, OnDestroy, AfterViewChecked {
  readonly dbDetail = inject(DbDetailComponent);
  private readonly dbService = inject(DatabaseService);

  readonly logs = signal<string[]>([]);
  readonly sseConnected = signal(false);
  readonly autoScroll = signal(true);
  private eventSource: EventSource | null = null;

  constructor() {
    effect(() => {
      const id = this.dbDetail.dbId();
      if (id) {
        this.connectLogs(id);
      } else {
        this.disconnectLogs();
      }
    });
  }

  ngOnInit(): void {
    // Connection is managed by the effect when dbId is resolved
  }

  ngOnDestroy(): void {
    this.disconnectLogs();
  }

  ngAfterViewChecked(): void {
    if (this.autoScroll()) {
      this.scrollLogsToBottom();
    }
  }

  connectLogs(id: string): void {
    this.disconnectLogs();
    this.logs.set(['[Console] Connecting to Kubernetes log stream...']);

    const streamUrl = this.dbService.getLogsStreamUrl(id);
    this.eventSource = new EventSource(streamUrl);

    this.eventSource.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] Connection established. Reading logs from pod:']);
    };

    this.eventSource.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => [...lines, event.data]);
        if (this.autoScroll()) {
          this.scrollLogsToBottom();
        }
      }
    };

    this.eventSource.onerror = () => {
      this.sseConnected.set(false);
      this.logs.update(lines => [...lines, '[Notice] Connection interrupted. Attempting to reconnect...']);
      this.disconnectLogs();
    };
  }

  disconnectLogs(): void {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
    this.sseConnected.set(false);
  }

  scrollLogsToBottom(): void {
    const el = document.getElementById('db-logs-window');
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }

  toggleAutoScroll(): void {
    this.autoScroll.update(val => !val);
    if (this.autoScroll()) {
      this.scrollLogsToBottom();
    }
  }
}
