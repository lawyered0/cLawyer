// Shared DOM helpers used across cLawyer web features.

export function byId(id) {
  return document.getElementById(id);
}

export function bindClick(id, handler) {
  const el = byId(id);
  if (el) el.addEventListener('click', handler);
}

export function bindChange(id, handler) {
  const el = byId(id);
  if (el) el.addEventListener('change', handler);
}

export function bindKeydown(id, handler) {
  const el = byId(id);
  if (el) el.addEventListener('keydown', handler);
}

export function delegate(container, eventType, selector, handler) {
  if (!container) return;
  container.addEventListener(eventType, function(event) {
    const target = event.target.closest(selector);
    if (!target || !container.contains(target)) return;
    handler(event, target);
  });
}
