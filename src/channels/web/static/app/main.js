// cLawyer Web Gateway - ESM bootstrap
// Phase 1 keeps runtime behavior identical by loading the existing legacy app.js.

function loadLegacyScript(src) {
  return new Promise((resolve, reject) => {
    const existing = document.querySelector(`script[data-clawyer-legacy="1"][src="${src}"]`);
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
    script.addEventListener('load', () => {
      script.dataset.loaded = '1';
      resolve();
    }, { once: true });
    script.addEventListener('error', () => reject(new Error(`Failed to load ${src}`)), { once: true });
    document.body.appendChild(script);
  });
}

(async function bootstrap() {
  try {
    await loadLegacyScript('/app.js');
  } catch (err) {
    console.error('cLawyer bootstrap failed:', err);
    const authError = document.getElementById('auth-error');
    if (authError) {
      authError.textContent = 'Failed to load web client assets.';
    }
  }
})();
