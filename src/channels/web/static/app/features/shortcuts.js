// --- Keyboard shortcuts ---

document.addEventListener('keydown', (e) => {
  const mod = e.metaKey || e.ctrlKey;
  const tag = (e.target.tagName || '').toLowerCase();
  const inInput = tag === 'input' || tag === 'textarea';

  if (isMatterCreateModalOpen()) {
    if (e.key === 'Escape') {
      e.preventDefault();
      closeMatterCreateModal();
      return;
    }
    if (e.key === 'Tab') {
      trapMatterCreateModalFocus(e);
      return;
    }
  }

  if (isMatterActionModalOpen()) {
    if (e.key === 'Escape') {
      e.preventDefault();
      closeMatterActionModal();
      return;
    }
    if (e.key === 'Tab') {
      trapMatterActionModalFocus(e);
      return;
    }
  }

  // Mod+1-8: switch tabs (primary then overflow)
  if (mod && e.key >= '1' && e.key <= '8') {
    e.preventDefault();
    const idx = parseInt(e.key) - 1;
    if (SHORTCUT_TABS[idx]) switchTab(SHORTCUT_TABS[idx]);
    return;
  }

  // Mod+9: open Settings -> Logs & Audit subsection
  if (mod && e.key === '9') {
    e.preventDefault();
    switchTab('settings', { settingsSection: 'logs' });
    return;
  }

  // Mod+K: focus chat input or memory search
  if (mod && e.key === 'k') {
    e.preventDefault();
    if (currentTab === 'memory') {
      document.getElementById('memory-search').focus();
    } else {
      document.getElementById('chat-input').focus();
    }
    return;
  }

  // Mod+N: new thread
  if (mod && e.key === 'n' && currentTab === 'chat') {
    e.preventDefault();
    createNewThread();
    return;
  }

  // Escape: close job detail or blur input
  if (e.key === 'Escape') {
    if (currentJobId) {
      closeJobDetail();
    } else if (inInput) {
      e.target.blur();
    }
    return;
  }
});

