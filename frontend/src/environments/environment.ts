// Default (development) environment.
// Production values live in environment.prod.ts and are swapped in via the
// angular.json `production` fileReplacements during `ng build`.
export const environment = {
  production: false,
  apiOrigin: 'http://localhost:8000',
  apiBaseUrl: 'http://localhost:8000/api/v1',
  wsBaseUrl: 'ws://localhost:8000/api/v1',
};
