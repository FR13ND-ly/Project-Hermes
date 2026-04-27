import { ComponentFixture, TestBed, fakeAsync, tick } from '@angular/core/testing';
import { ReactiveFormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { of, throwError } from 'rxjs';

import { Auth } from './auth';
import { AuthService } from '../../core/services/auth';

class MockAuthService {
  login = jasmine.createSpy('login').and.returnValue(of({ token: 'mock-jwt-token' }));
}

class MockRouter {
  navigate = jasmine.createSpy('navigate');
}

describe('Auth', () => {
  let component: Auth;
  let fixture: ComponentFixture<Auth>;
  let mockAuthService: MockAuthService;
  let mockRouter: MockRouter;

  beforeEach(async () => {
    mockAuthService = new MockAuthService();
    mockRouter = new MockRouter();

    await TestBed.configureTestingModule({
      imports: [Auth, ReactiveFormsModule],
      providers: [
        { provide: AuthService, useValue: mockAuthService },
        { provide: Router, useValue: mockRouter }
      ]
    })
    .compileComponents();

    fixture = TestBed.createComponent(Auth);
    component = fixture.componentInstance;
    
    fixture.detectChanges(); 
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('should have an invalid form on initialization with empty fields', () => {
    expect(component.loginForm.valid).toBeFalse();
    
    const submitBtn = fixture.nativeElement.querySelector('[data-testid="btn-login"]');
    expect(submitBtn.disabled).toBeTrue();
  });

  it('should validate the email format correctly', () => {
    const emailControl = component.loginForm.get('email');
    
    emailControl?.setValue('invalid-email-format');
    expect(emailControl?.valid).toBeFalse();
    
    emailControl?.setValue('developer@hermes.com');
    expect(emailControl?.valid).toBeTrue();
  });

  it('should call AuthService and navigate to /dashboard on successful login', fakeAsync(() => {
    component.loginForm.controls['email'].setValue('developer@hermes.com');
    component.loginForm.controls['password'].setValue('securePassword123!');
    fixture.detectChanges();

    const submitBtn = fixture.nativeElement.querySelector('[data-testid="btn-login"]');
    expect(submitBtn.disabled).toBeFalse();

    submitBtn.click();
    tick();

    expect(mockAuthService.login).toHaveBeenCalledWith('developer@hermes.com', 'securePassword123!');
    expect(mockRouter.navigate).toHaveBeenCalledWith(['/dashboard']);
  }));

  it('should display an error banner if the login request fails', fakeAsync(() => {
    mockAuthService.login.and.returnValue(throwError(() => new Error('Invalid credentials')));
    
    component.loginForm.controls['email'].setValue('developer@hermes.com');
    component.loginForm.controls['password'].setValue('wrong-password');
    
    component.onSubmit();
    tick();
    fixture.detectChanges();

    const errorBanner = fixture.nativeElement.querySelector('[data-testid="login-error-message"]');
    expect(errorBanner).toBeTruthy();
    expect(errorBanner.textContent).toContain('Invalid credentials');
    
    expect(mockRouter.navigate).not.toHaveBeenCalled();
  }));

  it('should disable the form and show a loading state during the API request', () => {
    component.isLoading.set(true);
    fixture.detectChanges();

    const submitBtn = fixture.nativeElement.querySelector('[data-testid="btn-login"]');
    expect(submitBtn.disabled).toBeTrue();
  });
});