// cLawyer legacy compatibility loader.
// Kept for route compatibility at /app.js.

(function loadModuleBootstrap() {
  if (window.__clawyerClientBootstrapped) return;

  var existing = document.querySelector('script[data-clawyer-main="1"]');
  if (existing) return;

  var script = document.createElement('script');
  script.type = 'module';
  script.src = '/app/main.js';
  script.setAttribute('data-clawyer-main', '1');
  document.body.appendChild(script);
})();
