// Shared HTTP helpers.

import { appState } from '/app/core/state.js';

export function beginRequest(key) {
  appState.requestVersions[key] = (appState.requestVersions[key] || 0) + 1;
  return appState.requestVersions[key];
}

export function isCurrentRequest(key, version) {
  return appState.requestVersions[key] === version;
}

export function apiFetch(path, options) {
  const opts = options || {};
  opts.headers = opts.headers || {};
  opts.headers.Authorization = 'Bearer ' + appState.authToken;
  if (opts.body instanceof FormData) {
    // Let the browser set Content-Type + multipart boundary automatically.
  } else if (opts.body && typeof opts.body === 'object') {
    opts.headers['Content-Type'] = 'application/json';
    opts.body = JSON.stringify(opts.body);
  }
  return fetch(path, opts).then((res) => {
    if (!res.ok) {
      return res.text().then(function(body) {
        throw new Error(body || res.status + ' ' + res.statusText);
      });
    }
    if (res.status === 204) return null;
    return res.text().then(function(body) {
      if (!body) return null;
      try {
        return JSON.parse(body);
      } catch (_) {
        return body;
      }
    });
  });
}
