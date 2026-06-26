import { Component, inject, signal, computed, OnInit, effect } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink, RouterLinkActive } from '@angular/router';
import { AuthService } from '../../../core/services/auth';
import { ToastService } from '../../../core/services/toast.service';

@Component({
  selector: 'app-admin-logs',
  standalone: true,
  imports: [CommonModule, FormsModule, RouterLink, RouterLinkActive],
  templateUrl: './logs.html',
  styleUrl: './logs.css',
})
export class AdminLogs implements OnInit {
  private readonly authService = inject(AuthService);
  private readonly toast = inject(ToastService);
  private readonly router = inject(Router);

  readonly activeTab = signal<'system' | 'auth'>('system');
  readonly loading = signal(false);

  // Raw fetched logs
  readonly systemLogs = signal<string>('');
  readonly authEvents = signal<any[]>([]);

  // Search filter & UI signals
  readonly logSearchQuery = signal('');
  readonly autoScroll = signal(true);

  // Format auth events as console lines
  readonly formattedAuthLogs = computed(() => {
    return this.authEvents().map(event => {
      const date = new Date(event.created_at).toISOString().replace('T', ' ').substring(0, 19);
      const isFailed = event.action.includes('FAILED');
      const level = isFailed ? 'WARN' : 'INFO';
      let msg = `[${date}] [${level}] ${event.action} | Identity: ${event.identity}`;
      if (event.client_ip) msg += ` | IP: ${event.client_ip}`;
      if (event.user_agent) msg += ` | UA: ${event.user_agent}`;
      return { text: msg, type: isFailed ? 'warn' : 'info' };
    });
  });

  // Filtered views
  readonly filteredSystemLogs = computed(() => {
    const query = this.logSearchQuery().trim().toLowerCase();
    const raw = this.systemLogs();
    if (!query) return raw;
    return raw.split('\n').filter(line => line.toLowerCase().includes(query)).join('\n');
  });

  readonly filteredAuthLogs = computed(() => {
    const query = this.logSearchQuery().trim().toLowerCase();
    const list = this.formattedAuthLogs();
    if (!query) return list;
    return list.filter(item => item.text.toLowerCase().includes(query));
  });

  constructor() {
    // Security check: super admins only
    const user = this.authService.currentUser();
    if (!user || !user.is_super_admin) {
      this.router.navigate(['/dashboard']);
    }

    // Auto-scroll effect
    effect(() => {
      // Access values to trigger dependency
      const active = this.activeTab();
      const sys = this.filteredSystemLogs();
      const auth = this.filteredAuthLogs();
      
      if (this.autoScroll()) {
        setTimeout(() => {
          const el = document.getElementById('admin-console-box');
          if (el) el.scrollTop = el.scrollHeight;
        }, 50);
      }
    });
  }

  ngOnInit(): void {
    this.loadLogs();
  }

  onTabChange(tab: 'system' | 'auth'): void {
    this.activeTab.set(tab);
    this.logSearchQuery.set('');
    this.loadLogs();
  }

  loadLogs(): void {
    this.loading.set(true);
    if (this.activeTab() === 'system') {
      this.authService.getSystemLogs().subscribe({
        next: (res) => {
          this.systemLogs.set(res.logs || 'No logs returned.');
          this.loading.set(false);
        },
        error: (err) => {
          this.systemLogs.set('Failed to load system logs from Hermes backend.');
          this.loading.set(false);
          this.toast.error(err.error?.message || 'Failed to load system logs.');
        }
      });
    } else {
      this.authService.getAuthLogs().subscribe({
        next: (res) => {
          this.authEvents.set(res || []);
          this.loading.set(false);
        },
        error: (err) => {
          this.authEvents.set([]);
          this.loading.set(false);
          this.toast.error(err.error?.message || 'Failed to load audit logs.');
        }
      });
    }
  }

  onDownloadLogs(): void {
    let text = '';
    let filename = '';
    
    if (this.activeTab() === 'system') {
      text = this.filteredSystemLogs();
      filename = 'hermes-system.log';
    } else {
      text = this.filteredAuthLogs().map(i => i.text).join('\n');
      filename = 'hermes-auth-audit.log';
    }

    const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  toggleAutoScroll(): void {
    this.autoScroll.update(v => !v);
  }
}
