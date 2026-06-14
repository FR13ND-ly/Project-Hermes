// Production environment.
// By default all URLs are derived from the browser's current origin, which suits
// a reverse-proxied deploy where the API and the frontend share one domain.
// If the API lives on a different host, replace the values below with absolute URLs.
const origin = (typeof window !== 'undefined' && window.location?.origin) || '';

export const environment = {
  production: true,
  apiOrigin: origin,
  apiBaseUrl: `${origin}/api/v1`,
  wsBaseUrl: `${origin.replace(/^http/, 'ws')}/api/v1`,
};
