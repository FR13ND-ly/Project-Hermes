import { HttpInterceptorFn, HttpRequest, HttpHandlerFn, HttpEvent, HttpErrorResponse } from '@angular/common/http';
import { Observable, throwError } from 'rxjs';
import { catchError } from 'rxjs/operators';

/**
 * Normalizes backend error responses from { error: { message, code } }
 * to a flat structure so components can access err.error.message directly.
 * 
 * Backend returns: { error: { code: "VALIDATION_ERROR", message: "...", request_id: "..." } }
 * Angular HttpClient puts this in: err.error = { error: { message: "..." } }
 * Components expect:                err.error.message
 * 
 * This interceptor flattens it so err.error = { message: "...", code: "...", request_id: "..." }
 */
export const errorNormalizerInterceptor: HttpInterceptorFn = (
  req: HttpRequest<unknown>,
  next: HttpHandlerFn
): Observable<HttpEvent<unknown>> => {
  return next(req).pipe(
    catchError((error: HttpErrorResponse) => {
      if (error.error?.error?.message) {
        // Flatten the nested error structure
        const normalized = new HttpErrorResponse({
          error: {
            message: error.error.error.message,
            code: error.error.error.code,
            request_id: error.error.error.request_id,
          },
          headers: error.headers,
          status: error.status,
          statusText: error.statusText,
          url: error.url ?? undefined,
        });
        return throwError(() => normalized);
      }
      return throwError(() => error);
    })
  );
};
