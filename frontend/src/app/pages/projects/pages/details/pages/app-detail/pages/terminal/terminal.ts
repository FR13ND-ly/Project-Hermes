import { 
  Component, 
  inject, 
  OnInit, 
  OnDestroy, 
  AfterViewInit, 
  HostListener 
} from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { environment } from '../../../../../../../../../environments/environment';

@Component({
  selector: 'app-app-terminal',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './terminal.html',
  styles: `
    :host ::ng-deep .xterm {
      padding: 8px;
      height: 100%;
    }
    :host ::ng-deep .xterm-viewport {
      background-color: #000000 !important;
    }
  `,
})
export class AppTerminalComponent implements OnInit, OnDestroy, AfterViewInit {
  readonly parent = inject(AppDetailComponent);

  private socket: WebSocket | null = null;
  private term: Terminal | null = null;
  private fitAddon: FitAddon | null = null;

  ngOnInit(): void {
    // Handled in ngAfterViewInit
  }

  ngAfterViewInit(): void {
    setTimeout(() => {
      this.initTerminal();
    }, 50);
  }

  ngOnDestroy(): void {
    this.disconnect();
    if (this.term) {
      this.term.dispose();
    }
  }

  @HostListener('window:resize')
  onResize(): void {
    if (this.fitAddon && this.term) {
      this.fitAddon.fit();
      this.sendResize();
    }
  }

  private initTerminal(): void {
    const container = document.getElementById('terminal-container');
    if (!container) return;

    this.term = new Terminal({
      cursorBlink: true,
      theme: {
        background: '#000000',
        foreground: '#e0e0e0',
        cursor: '#ffffff',
        selectionBackground: '#5c5c5c',
        black: '#000000',
        red: '#cd3131',
        green: '#0dbc79',
        yellow: '#e5e510',
        blue: '#2472c8',
        magenta: '#bc3fbc',
        cyan: '#11a8cd',
        white: '#e5e5e5'
      },
      fontFamily: 'Consolas, "Fira Code", Monaco, "Courier New", Courier, monospace',
      fontSize: 12,
      rows: 24,
      cols: 80
    });

    this.fitAddon = new FitAddon();
    this.term.loadAddon(this.fitAddon);
    this.term.open(container);
    this.fitAddon.fit();

    this.term.onData(data => {
      if (this.socket && this.socket.readyState === WebSocket.OPEN) {
        this.socket.send(data);
      }
    });

    // Initial message
    this.term.write('\x1b[33m*** Connecting to pod terminal... ***\x1b[0m\r\n');

    this.connectWebSocket();
  }

  private connectWebSocket(): void {
    this.disconnect();

    const appId = this.parent.appId();
    const instanceId = this.parent.activeInstanceId();
    if (!appId || !instanceId) {
      this.term?.write('\x1b[31mError: No active instance selected.\x1b[0m\r\n');
      return;
    }

    const token = localStorage.getItem('hermes_token') || '';
    const wsUrl = `${environment.wsBaseUrl}/apps/${appId}/instances/${instanceId}/terminal/ws?token=${encodeURIComponent(token)}`;

    try {
      this.socket = new WebSocket(wsUrl);
      this.socket.binaryType = 'arraybuffer';

      this.socket.onopen = () => {
        this.term?.reset();
        this.term?.write('\x1b[32m*** Terminal session established ***\x1b[0m\r\n\r\n');
        this.sendResize();
      };

      this.socket.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) {
          this.term?.write(new Uint8Array(event.data));
        } else {
          this.term?.write(event.data);
        }
      };

      this.socket.onerror = (err) => {
        this.term?.write('\r\n\x1b[31m[WebSocket Error] Failed to connect to terminal backend.\x1b[0m\r\n');
      };

      this.socket.onclose = () => {
        this.term?.write('\r\n\x1b[31m*** Terminal session closed ***\x1b[0m\r\n');
      };
    } catch (e) {
      this.term?.write('\r\n\x1b[31m[Error] WebSocket creation failed.\x1b[0m\r\n');
    }
  }

  private disconnect(): void {
    if (this.socket) {
      this.socket.close();
      this.socket = null;
    }
  }

  onInstanceChange(id: string): void {
    this.parent.activeInstanceId.set(id);
    if (this.term) {
      this.term.reset();
      this.term.write('\x1b[33m*** Connecting to new instance... ***\x1b[0m\r\n');
    }
    this.connectWebSocket();
  }

  private sendResize(): void {
    if (this.socket && this.socket.readyState === WebSocket.OPEN && this.term) {
      const resize = {
        cols: this.term.cols,
        rows: this.term.rows
      };
      this.socket.send(JSON.stringify(resize));
    }
  }
}
