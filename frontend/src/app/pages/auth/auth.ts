import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { ReactiveFormsModule, FormBuilder, FormGroup, Validators } from '@angular/forms';

@Component({
  selector: 'app-auth',
  standalone: true,
  imports: [CommonModule, ReactiveFormsModule], 
  templateUrl: './auth.html',
  styleUrl: './auth.css',
})
export class Auth {
  loginForm: FormGroup;
  isPasswordVisible = false; 
  mascotState = 'idle';    

  constructor(private fb: FormBuilder) {
    this.loginForm = this.fb.group({
      email: ['', [Validators.required, Validators.email]],
      password: ['', [Validators.required, Validators.minLength(6)]]
    });
  }

  setMascotState(state: string) {
    if (state === 'covering' && this.isPasswordVisible) {
      this.mascotState = 'peeking';
    } else {
      this.mascotState = state;
    }
  }

  togglePasswordVisibility() {
    this.isPasswordVisible = !this.isPasswordVisible;
   
    if (this.mascotState === 'covering' || this.mascotState === 'peeking') {
      this.mascotState = this.isPasswordVisible ? 'peeking' : 'covering';
    }
  }

  onSubmit() {
    if (this.loginForm.valid) {
      console.log('Autentificare cu succes:', this.loginForm.value);
    } else {
      this.loginForm.markAllAsTouched();
    }
  }
}