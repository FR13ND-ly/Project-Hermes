import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { AuthService } from '../services/auth';

/**
 * Blocks routes that require an authenticated session. Unauthenticated visitors
 * are redirected to /auth instead of briefly flashing a protected page.
 */
export const authGuard: CanActivateFn = () => {
  const auth = inject(AuthService);
  const router = inject(Router);
  return auth.isAuthenticated() ? true : router.createUrlTree(['/auth']);
};

/**
 * Restricts routes to super admins. Non-admins (and guests) are sent to the
 * dashboard / login rather than seeing a forbidden page.
 */
export const superAdminGuard: CanActivateFn = () => {
  const auth = inject(AuthService);
  const router = inject(Router);
  if (!auth.isAuthenticated()) {
    return router.createUrlTree(['/auth']);
  }
  return auth.currentUser()?.is_super_admin === true
    ? true
    : router.createUrlTree(['/dashboard']);
};

/**
 * For guest-only routes (the auth page): already-authenticated users are sent
 * straight to the dashboard.
 */
export const guestGuard: CanActivateFn = () => {
  const auth = inject(AuthService);
  const router = inject(Router);
  return auth.isAuthenticated() ? router.createUrlTree(['/dashboard']) : true;
};
