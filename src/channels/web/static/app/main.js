// cLawyer Web Gateway - ESM bootstrap.
// Loads core ESM primitives, then feature-bounded legacy scripts in order.

import { bindChange, bindClick, bindKeydown, byId, delegate } from '/app/core/dom.js';
import { apiFetch, beginRequest, isCurrentRequest } from '/app/core/http.js';
import {
  appState,
  setAuthToken,
  setCurrentSettingsSection,
  setCurrentTab,
} from '/app/core/state.js';
import { OVERFLOW_TABS, PRIMARY_TABS, SHORTCUT_TABS } from '/app/core/tabs.js';

/** @type {any} */
const globalWindow = window;

function loadLegacyScript(src) {
  return new Promise((resolve, reject) => {
    /** @type {HTMLScriptElement | null} */
    const existing = /** @type {HTMLScriptElement | null} */ (
      document.querySelector(`script[data-clawyer-legacy="1"][src="${src}"]`)
    );
    if (existing) {
      if (existing.dataset.loaded === '1') {
        resolve();
        return;
      }
      existing.addEventListener('load', () => resolve(), { once: true });
      existing.addEventListener('error', () => reject(new Error(`Failed to load ${src}`)), { once: true });
      return;
    }

    const script = document.createElement('script');
    script.src = src;
    script.defer = true;
    script.dataset.clawyerLegacy = '1';
    script.addEventListener(
      'load',
      () => {
        script.dataset.loaded = '1';
        resolve();
      },
      { once: true }
    );
    script.addEventListener('error', () => reject(new Error(`Failed to load ${src}`)), {
      once: true,
    });
    document.body.appendChild(script);
  });
}

const FEATURE_SCRIPTS = [
  '/app/features/globals.js',
  '/app/features/utilities.js',
  '/app/features/toasts.js',
  '/app/features/activity.js',
  '/app/features/chat.js',
  '/app/features/auth_card.js',
  '/app/features/threads.js',
  '/app/features/tabs.js',
  '/app/features/memory.js',
  '/app/features/logs.js',
  '/app/features/extensions.js',
  '/app/features/pairing.js',
  '/app/features/jobs.js',
  '/app/features/job_activity.js',
  '/app/features/routines.js',
  '/app/features/gateway_status.js',
  '/app/features/tee.js',
  '/app/features/extension_install.js',
  '/app/features/skills.js',
  '/app/features/matters.js',
  '/app/features/memory_upload.js',
  '/app/features/settings.js',
  '/app/features/routine_create.js',
  '/app/features/shortcuts.js',
  '/app/features/sse.js',
  '/app/features/auth.js',
];

(async function bootstrap() {
  try {
    if (globalWindow.__clawyerClientBootstrapped) return;
    globalWindow.__clawyerClientBootstrapped = true;

    globalWindow.__clawyerCore = {
      appState,
      setAuthToken,
      setCurrentTab,
      setCurrentSettingsSection,
      byId,
      bindClick,
      bindChange,
      bindKeydown,
      delegate,
      apiFetch,
      beginRequest,
      isCurrentRequest,
      PRIMARY_TABS,
      OVERFLOW_TABS,
      SHORTCUT_TABS,
    };

    for (const script of FEATURE_SCRIPTS) {
      // Sequential load keeps legacy cross-file globals deterministic.
      // eslint-disable-next-line no-await-in-loop
      await loadLegacyScript(script);
    }
  } catch (err) {
    console.error('cLawyer bootstrap failed:', err);
    const authError = document.getElementById('auth-error');
    if (authError) {
      authError.textContent = 'Failed to load web client assets.';
    }
  }
})();
