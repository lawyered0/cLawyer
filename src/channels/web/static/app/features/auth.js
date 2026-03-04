// --- Auth ---

function authenticate() {
  token = document.getElementById('token-input').value.trim();
  setAuthToken(token);
  if (!token) {
    document.getElementById('auth-error').textContent = 'Token required';
    return;
  }

  // Test the token against the health-ish endpoint (chat/threads requires auth)
  apiFetch('/api/chat/threads')
    .then(() => {
      sessionStorage.setItem('clawyer_token', token);
      document.getElementById('auth-screen').style.display = 'none';
      document.getElementById('app').style.display = 'flex';
      // Strip token and log_level from URL so they're not visible in the address bar
      const cleaned = new URL(window.location);
      const urlLogLevel = cleaned.searchParams.get('log_level');
      cleaned.searchParams.delete('token');
      cleaned.searchParams.delete('log_level');
      window.history.replaceState({}, '', cleaned.pathname + cleaned.search);
      connectSSE();
      connectLogSSE();
      startGatewayStatusPolling();
      checkTeeStatus();
      loadThreads();
      loadMemoryTree();
      loadJobs();
      refreshActiveMatterState();
      // Apply URL log_level param if present, otherwise just sync the dropdown
      if (urlLogLevel) {
        setServerLogLevel(urlLogLevel);
      } else {
        loadServerLogLevel();
      }
    })
    .catch(() => {
      setAuthToken('');
      sessionStorage.removeItem('clawyer_token');
      document.getElementById('auth-screen').style.display = '';
      document.getElementById('app').style.display = 'none';
      document.getElementById('auth-error').textContent = 'Invalid token';
    });
}

bindKeydown('token-input', (e) => {
  if (e.key === 'Enter') authenticate();
});
bindClick('auth-connect-btn', authenticate);

// Auto-authenticate from URL param or saved session
(function autoAuth() {
  const params = new URLSearchParams(window.location.search);
  const urlToken = params.get('token');
  if (urlToken) {
    document.getElementById('token-input').value = urlToken;
    authenticate();
    return;
  }
  const saved = sessionStorage.getItem('clawyer_token');
  if (saved) {
    document.getElementById('token-input').value = saved;
    // Hide auth screen immediately to prevent flash, authenticate() will
    // restore it if the token turns out to be invalid.
    document.getElementById('auth-screen').style.display = 'none';
    document.getElementById('app').style.display = 'flex';
    authenticate();
  }
})();

(function bindStaticUiEvents() {
  bindClick('matter-badge', function() { switchTab('matters'); });
  bindClick('chat-active-matter-banner', function() { switchTab('matters'); });
  bindClick('tab-overflow-trigger', toggleTabOverflowMenu);
  bindClick('thread-new-btn', createNewThread);
  bindClick('thread-toggle-btn', toggleThreadSidebar);
  bindClick('send-btn', sendMessage);
  bindClick('memory-edit-btn', startMemoryEdit);
  bindClick('memory-upload-btn', triggerMemoryUpload);
  bindClick('memory-save-btn', saveMemoryEdit);
  bindClick('memory-cancel-btn', cancelMemoryEdit);
  bindChange('logs-server-level', function(e) { setServerLogLevel(e.target.value); });
  bindClick('logs-pause-btn', toggleLogsPause);
  bindClick('logs-clear-btn', clearLogs);
  bindClick('wasm-install-btn', installWasmExtension);
  bindClick('mcp-install-btn', addMcpServer);
  bindClick('skill-search-btn', searchClawHub);
  bindClick('skill-install-btn', installSkillFromForm);
  bindClick('backups-create-btn', createBackup);
  bindClick('backups-verify-btn', verifyLastBackup);
  bindClick('backups-restore-btn', triggerBackupRestoreInput);
  bindClick('settings-section-general-btn', function() { openSettingsSection('general'); });
  bindClick('settings-section-logs-btn', function() { openSettingsSection('logs'); });
  bindClick('settings-compliance-refresh-btn', loadComplianceStatus);
  bindClick('settings-compliance-letter-btn', generateComplianceLetter);
  bindClick('settings-compliance-toggle', toggleComplianceBreakdown);
  bindClick('matters-new-btn', openMatterCreateModal);
  bindClick('matter-create-modal-close', closeMatterCreateModal);
  bindClick('matter-create-cancel-btn', closeMatterCreateModal);
  bindClick('matter-action-modal-close', closeMatterActionModal);
  bindClick('matter-action-cancel-btn', closeMatterActionModal);
  bindClick('matters-clear-btn', clearActiveMatter);
  bindClick('matters-conflict-clear-btn', clearMatterConflictCheck);
  bindClick('legal-audit-refresh-btn', function() { loadLegalAudit(0); });
  bindClick('legal-audit-prev-btn', function() {
    var next = Math.max(0, legalAuditOffset - legalAuditLimit);
    loadLegalAudit(next);
  });
  bindClick('legal-audit-next-btn', function() {
    if (legalAuditNextOffset == null) return;
    loadLegalAudit(legalAuditNextOffset);
  });
  bindChange('matters-group-by-client', function(e) {
    mattersGroupByClient = !!(e && e.target && e.target.checked);
    localStorage.setItem(MATTERS_GROUP_KEY, mattersGroupByClient ? '1' : '0');
    renderMatters();
  });
  var mattersCreateForm = byId('matters-create-form');
  if (mattersCreateForm) {
    mattersCreateForm.addEventListener('submit', function(e) {
      e.preventDefault();
      createMatterFromForm();
    });
    mattersCreateForm.addEventListener('input', handleMatterCreateFormMutation);
    mattersCreateForm.addEventListener('change', handleMatterCreateFormMutation);
  }
  bindClick('matters-review-btn', function(e) {
    e.preventDefault();
    reviewMatterCreateConflicts();
  });
  bindChange('matter-create-conflict-decision', function() {
    syncMatterCreateActionState();
  });
  bindChange('matter-create-confidentiality', syncMatterSelectOtherFields);
  bindChange('matter-create-retention', syncMatterSelectOtherFields);
  var conflictNote = byId('matter-create-conflict-note');
  if (conflictNote) {
    conflictNote.addEventListener('input', function() {
      syncMatterCreateActionState();
    });
  }
  var matterCreateModal = byId('matter-create-modal');
  if (matterCreateModal) {
    matterCreateModal.addEventListener('click', function(e) {
      if (e.target === matterCreateModal) closeMatterCreateModal();
    });
  }
  var matterActionModal = byId('matter-action-modal');
  if (matterActionModal) {
    matterActionModal.addEventListener('click', function(e) {
      if (e.target === matterActionModal) closeMatterActionModal();
    });
  }
  var matterActionForm = byId('matter-action-form');
  if (matterActionForm) {
    matterActionForm.addEventListener('submit', function(e) {
      e.preventDefault();
      submitMatterActionForm();
    });
  }
  var mattersConflictForm = byId('matters-conflict-form');
  if (mattersConflictForm) {
    mattersConflictForm.addEventListener('submit', function(e) {
      e.preventDefault();
      runMatterConflictCheck();
    });
  }
  delegate(byId('matters-list'), 'click', 'button[data-matter-action]', function(e, button) {
    e.preventDefault();
    var idx = parseInt(button.getAttribute('data-matter-index'), 10);
    if (isNaN(idx) || !mattersCache[idx]) return;
    var matter = mattersCache[idx];
    var action = button.getAttribute('data-matter-action');
    if (action === 'select') {
      selectMatter(matter.id);
      return;
    }
    if (action === 'browse') {
      viewMatterInMemory(matter.id);
      return;
    }
    if (action === 'detail') {
      openMatterDetail(matter.id);
    }
  });
  delegate(byId('matter-detail-panel'), 'click', 'button[data-matter-detail-action]', function(e, button) {
    e.preventDefault();
    var action = button.getAttribute('data-matter-detail-action');
    if (action === 'open-doc') {
      var path = button.getAttribute('data-path');
      if (path) openMatterPathInMemory(path);
      return;
    }
    if (action === 'preview-template') {
      var templatePath = button.getAttribute('data-path');
      if (templatePath) openMatterPathInMemory(templatePath);
      return;
    }
    if (action === 'apply-template') {
      var templateName = button.getAttribute('data-template-name');
      if (templateName) applyMatterTemplate(templateName);
      return;
    }
    if (action === 'build-filing-package') {
      buildMatterFilingPackage();
      return;
    }
    if (action === 'export-retrieval-packet') {
      exportMatterRetrievalPacket(false);
      return;
    }
    if (action === 'open-thread') {
      var threadId = button.getAttribute('data-thread-id');
      if (threadId) {
        switchTab('chat');
        switchThread(threadId);
      }
      return;
    }
    if (action === 'create-task') {
      openMatterActionModal('task');
      return;
    }
    if (action === 'create-note') {
      openMatterActionModal('note');
      return;
    }
    if (action === 'create-time-entry') {
      openMatterActionModal('time');
      return;
    }
    if (action === 'create-expense-entry') {
      openMatterActionModal('expense');
      return;
    }
    if (action === 'record-trust-deposit') {
      openMatterActionModal('deposit');
      return;
    }
    if (action === 'open-invoice') {
      var invoiceId = button.getAttribute('data-invoice-id');
      if (invoiceId) openMatterInvoiceDetail(invoiceId);
    }
  });
  delegate(byId('matter-detail-panel'), 'click', 'button[data-matter-detail-section]', function(e, button) {
    e.preventDefault();
    var section = button.getAttribute('data-matter-detail-section');
    if (!section) return;
    setMatterDetailSection(section);
  });
  delegate(byId('legal-audit-list'), 'click', 'button[data-audit-index]', function(e, button) {
    e.preventDefault();
    var idx = parseInt(button.getAttribute('data-audit-index'), 10);
    if (isNaN(idx) || !legalAuditEvents[idx]) return;
    renderLegalAuditDetail(legalAuditEvents[idx]);
  });
  document.addEventListener('click', handleDocumentClickForMenus);
  document.addEventListener('keydown', function(event) {
    if (event.key === 'Escape') {
      closeTabOverflowMenu();
    }
  });
  window.addEventListener('resize', positionTabOverflowMenu);
  window.addEventListener('scroll', positionTabOverflowMenu, true);
  syncMatterSelectOtherFields();
})();

