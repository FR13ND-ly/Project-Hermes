import { Injectable, inject, effect, DestroyRef } from '@angular/core';
import { AuthService } from './auth';
import { Subject, Observable } from 'rxjs';
import { filter, map } from 'rxjs/operators';
import { environment } from '../../../environments/environment';

export interface InstanceStatusChangedPayload {
  workspace_id: string;
  instance_id: string;
  container_name: string;
  status: string;
}

export interface DatabaseStatusChangedPayload {
  workspace_id: string;
  database_id: string;
  container_name: string;
  status: string;
}

export interface BuildStatusChangedPayload {
  workspace_id: string;
  build_id: string;
  app_id: string;
  status: string;
}

export interface IncidentCreatedPayload {
  workspace_id: string;
  incident_id: string;
  project_id: string;
  message: string;
}

export interface SystemEvent<T> {
  type: string;
  payload: T;
}

@Injectable({
  providedIn: 'root'
})
export class WebSocketService {
  private readonly authService = inject(AuthService);
  private readonly destroyRef = inject(DestroyRef);

  private socket: WebSocket | null = null;
  private readonly eventSubject = new Subject<SystemEvent<any>>();
  private reconnectTimeout: any = null;
  private isConnecting = false;
  private reconnectDelay = 2000;
  private intentionalDisconnect = false;

  constructor() {
    // Watch current user status to connect or disconnect
    effect(() => {
      const user = this.authService.currentUser();
      if (user) {
        this.intentionalDisconnect = false;
        this.connect();
      } else {
        this.intentionalDisconnect = true;
        this.disconnect();
      }
    });

    this.destroyRef.onDestroy(() => {
      this.disconnect();
    });
  }

  private connect(): void {
    if (this.socket && (this.socket.readyState === WebSocket.OPEN || this.socket.readyState === WebSocket.CONNECTING)) {
      this.disconnect();
    }

    const token = localStorage.getItem('hermes_token');
    if (!token) {
      return;
    }

    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }

    const wsUrl = `${environment.wsBaseUrl}/ws?token=${encodeURIComponent(token)}`;

    console.log('[WebSocket] Connecting...');
    this.isConnecting = true;

    try {
      this.socket = new WebSocket(wsUrl);

      this.socket.onopen = () => {
        console.log('[WebSocket] Connected successfully');
        this.isConnecting = false;
        this.reconnectDelay = 2000;
      };

      this.socket.onmessage = (event) => {
        try {
          const parsed = JSON.parse(event.data) as SystemEvent<any>;
          console.log('[WebSocket] Event received:', parsed);
          this.eventSubject.next(parsed);
        } catch (e) {
          console.error('[WebSocket] Failed to parse message:', e);
        }
      };

      this.socket.onerror = (error) => {
        console.error('[WebSocket] Error:', error);
      };

      this.socket.onclose = (event) => {
        console.log('[WebSocket] Closed:', event);
        this.socket = null;
        this.isConnecting = false;
        if (!this.intentionalDisconnect) {
          this.scheduleReconnect();
        }
      };
    } catch (e) {
      console.error('[WebSocket] Creation failed:', e);
      this.isConnecting = false;
      this.scheduleReconnect();
    }
  }

  private disconnect(): void {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
    if (this.socket) {
      this.socket.close();
      this.socket = null;
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimeout || this.intentionalDisconnect) {
      return;
    }
    console.log(`[WebSocket] Reconnecting in ${this.reconnectDelay}ms...`);
    this.reconnectTimeout = setTimeout(() => {
      this.connect();
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, 30000);
    }, this.reconnectDelay);
  }

  /**
   * Listen to system events of a specific type.
   */
  onEvent<T>(type: string): Observable<T> {
    return this.eventSubject.asObservable().pipe(
      filter(event => event && event.type === type),
      map(event => event.payload as T)
    );
  }

  /**
   * Emitted when the server signals the client fell behind and dropped events
   * (broadcast lag). Subscribers should refetch the data they render live so the
   * UI doesn't get stuck on stale state.
   */
  onResync(): Observable<void> {
    return this.eventSubject.asObservable().pipe(
      filter(event => event && event.type === 'resync'),
      map(() => undefined)
    );
  }
}
