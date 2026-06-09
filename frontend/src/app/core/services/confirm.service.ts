import { Injectable, signal } from '@angular/core';

export interface ConfirmOptions {
  title: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  isDanger?: boolean;
  matchText?: string;
}

@Injectable({
  providedIn: 'root'
})
export class ConfirmService {
  readonly activeModal = signal<{
    options: ConfirmOptions;
    resolve: (value: boolean) => void;
  } | null>(null);

  readonly typedText = signal('');

  ask(options: ConfirmOptions): Promise<boolean> {
    this.typedText.set('');
    return new Promise<boolean>((resolve) => {
      this.activeModal.set({
        options: {
          confirmText: 'Confirmă',
          cancelText: 'Anulează',
          isDanger: false,
          ...options
        },
        resolve
      });
    });
  }

  confirm(): void {
    const active = this.activeModal();
    if (active) {
      active.resolve(true);
      this.activeModal.set(null);
      this.typedText.set('');
    }
  }

  cancel(): void {
    const active = this.activeModal();
    if (active) {
      active.resolve(false);
      this.activeModal.set(null);
      this.typedText.set('');
    }
  }
}
