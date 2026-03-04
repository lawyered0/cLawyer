// --- Settings ---

function complianceStateClass(state) {
  var normalized = (state || 'partial').toLowerCase();
  if (normalized === 'compliant') return 'state-compliant';
  if (normalized === 'needs_review') return 'state-needs-review';
  return 'state-partial';
}

function complianceStateLabel(state) {
  var normalized = (state || 'partial').toLowerCase();
  if (normalized === 'compliant') return 'Compliant';
  if (normalized === 'needs_review') return 'Needs Review';
  return 'Partial';
}

function setComplianceExpanded(expanded) {
  complianceExpanded = !!expanded;
  var breakdown = byId('settings-compliance-breakdown');
  var caret = byId('settings-compliance-caret');
  if (!breakdown) return;
  breakdown.classList.toggle('is-hidden', !complianceExpanded);
  if (caret) caret.textContent = complianceExpanded ? '▴' : '▾';
}

function renderComplianceBreakdown(data) {
  var container = byId('settings-compliance-breakdown');
  if (!container || !data) return;
  var sections = [
    { key: 'govern', label: 'Govern' },
    { key: 'map', label: 'Map' },
    { key: 'measure', label: 'Measure' },
    { key: 'manage', label: 'Manage' },
  ];

  var html = '';
  for (var i = 0; i < sections.length; i++) {
    var section = sections[i];
    var value = data[section.key] || {};
    var checks = Array.isArray(value.checks) ? value.checks : [];
    var sectionStateClass = complianceStateClass(value.status);
    html += '<div class="compliance-function">';
    html += '<div class="compliance-function-header">';
    html += '<span>' + section.label + '</span>';
    html += '<span class="compliance-state-badge ' + sectionStateClass + '">'
      + complianceStateLabel(value.status)
      + '</span>';
    html += '</div>';
    if (!checks.length) {
      html += '<div class="empty-state">No checks returned.</div>';
    } else {
      html += '<ul class="compliance-check-list">';
      for (var j = 0; j < checks.length; j++) {
        var check = checks[j] || {};
        var checkStateClass = complianceStateClass(check.status);
        html += '<li class="compliance-check-item ' + checkStateClass + '">';
        html += '<div class="compliance-check-title">' + escapeHtml(check.label || check.id || 'Check') + '</div>';
        html += '<div class="compliance-check-detail">' + escapeHtml(check.detail || '') + '</div>';
        html += '</li>';
      }
      html += '</ul>';
    }
    html += '</div>';
  }

  var gaps = Array.isArray(data.data_gaps) ? data.data_gaps : [];
  if (gaps.length) {
    html += '<div class="compliance-data-gaps"><strong>Data gaps:</strong><ul>';
    for (var k = 0; k < gaps.length; k++) {
      html += '<li>' + escapeHtml(gaps[k]) + '</li>';
    }
    html += '</ul></div>';
  }

  container.innerHTML = html;
}

function renderComplianceStatus(data) {
  var dot = byId('settings-compliance-dot');
  var label = byId('settings-compliance-label');
  var meta = byId('settings-compliance-meta');
  if (!dot || !label || !meta) return;

  var overallState = data && data.overall ? data.overall : 'partial';
  var dotClass = complianceStateClass(overallState);
  dot.className = 'dot ' + dotClass;
  label.textContent = 'NIST AI RMF: ' + complianceStateLabel(overallState);

  var metrics = (data && data.metrics) || {};
  var classified = metrics.matters_classified != null ? metrics.matters_classified : 0;
  var total = metrics.matters_total != null ? metrics.matters_total : 0;
  var tools = metrics.tools_total != null ? metrics.tools_total : 0;
  var audit = metrics.audit_events_total == null ? 'unavailable' : String(metrics.audit_events_total);
  meta.textContent = 'Matters classified: ' + classified + '/' + total + ' · Tools: ' + tools + ' · Audit events: ' + audit;

  if (complianceExpanded) {
    renderComplianceBreakdown(data);
    setComplianceExpanded(true);
  } else {
    setComplianceExpanded(false);
  }
}

function loadComplianceStatus() {
  var requestVersion = beginRequest('complianceStatus');
  var label = byId('settings-compliance-label');
  var meta = byId('settings-compliance-meta');
  if (label) label.textContent = 'Loading compliance status…';
  if (meta) meta.textContent = '';

  apiFetch('/api/compliance/status').then(function(data) {
    if (!isCurrentRequest('complianceStatus', requestVersion)) return;
    complianceStatusCache = data;
    renderComplianceStatus(data);
  }).catch(function(err) {
    if (!isCurrentRequest('complianceStatus', requestVersion)) return;
    complianceStatusCache = null;
    var dot = byId('settings-compliance-dot');
    if (dot) dot.className = 'dot state-needs-review';
    if (label) label.textContent = 'Compliance status unavailable';
    if (meta) meta.textContent = err.message;
    setComplianceExpanded(false);
    var breakdown = byId('settings-compliance-breakdown');
    if (breakdown) breakdown.innerHTML = '<div class="empty-state">Unable to load compliance checks.</div>';
  });
}

function toggleComplianceBreakdown() {
  if (!complianceStatusCache) {
    loadComplianceStatus();
    return;
  }
  setComplianceExpanded(!complianceExpanded);
  if (complianceExpanded) {
    renderComplianceBreakdown(complianceStatusCache);
  }
}

function closeComplianceLetterModal() {
  var existing = byId('compliance-letter-modal-overlay');
  if (existing) existing.remove();
}

function openComplianceLetterRequestModal() {
  closeComplianceLetterModal();
  var overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.id = 'compliance-letter-modal-overlay';
  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeComplianceLetterModal();
  });

  var modal = document.createElement('div');
  modal.className = 'configure-modal';

  var title = document.createElement('h3');
  title.textContent = 'Generate Compliance Letter';
  modal.appendChild(title);

  var form = document.createElement('form');
  form.className = 'configure-form';

  var frameworkField = document.createElement('div');
  frameworkField.className = 'configure-field';
  var frameworkLabel = document.createElement('label');
  frameworkLabel.textContent = 'Framework';
  frameworkLabel.setAttribute('for', 'compliance-letter-framework');
  frameworkField.appendChild(frameworkLabel);
  var frameworkRow = document.createElement('div');
  frameworkRow.className = 'configure-input-row';
  var frameworkSelect = document.createElement('select');
  frameworkSelect.id = 'compliance-letter-framework';
  [
    { value: 'nist', label: 'NIST AI RMF' },
    { value: 'colorado-sb205', label: 'Colorado SB205' },
    { value: 'eu-ai-act', label: 'EU AI Act' },
  ].forEach(function(optionDef) {
    var option = document.createElement('option');
    option.value = optionDef.value;
    option.textContent = optionDef.label;
    frameworkSelect.appendChild(option);
  });
  frameworkRow.appendChild(frameworkSelect);
  frameworkField.appendChild(frameworkRow);
  form.appendChild(frameworkField);

  var firmField = document.createElement('div');
  firmField.className = 'configure-field';
  var firmLabel = document.createElement('label');
  firmLabel.textContent = 'Firm name (optional)';
  firmLabel.setAttribute('for', 'compliance-letter-firm');
  firmField.appendChild(firmLabel);
  var firmRow = document.createElement('div');
  firmRow.className = 'configure-input-row';
  var firmInput = document.createElement('input');
  firmInput.id = 'compliance-letter-firm';
  firmInput.type = 'text';
  firmInput.placeholder = 'Acme Law LLP';
  firmRow.appendChild(firmInput);
  firmField.appendChild(firmRow);
  form.appendChild(firmField);

  var actions = document.createElement('div');
  actions.className = 'configure-actions';

  var cancelBtn = document.createElement('button');
  cancelBtn.type = 'button';
  cancelBtn.className = 'btn-ext';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', closeComplianceLetterModal);
  actions.appendChild(cancelBtn);

  var generateBtn = document.createElement('button');
  generateBtn.type = 'submit';
  generateBtn.className = 'btn-ext activate';
  generateBtn.textContent = 'Generate';
  actions.appendChild(generateBtn);

  form.appendChild(actions);
  modal.appendChild(form);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);
  firmInput.focus();

  form.addEventListener('submit', function(e) {
    e.preventDefault();
    var payload = {
      framework: frameworkSelect.value || 'nist',
      firm_name: firmInput.value.trim() || null,
    };
    generateBtn.disabled = true;
    generateBtn.textContent = 'Generating…';

    apiFetch('/api/compliance/letter', {
      method: 'POST',
      body: payload,
    }).then(function(response) {
      openComplianceLetterModal(response);
    }).catch(function(err) {
      showToast('Compliance letter failed: ' + err.message, 'error');
      generateBtn.disabled = false;
      generateBtn.textContent = 'Generate';
    });
  });
}

function openComplianceLetterModal(response) {
  closeComplianceLetterModal();
  var overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.id = 'compliance-letter-modal-overlay';
  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeComplianceLetterModal();
  });

  var modal = document.createElement('div');
  modal.className = 'configure-modal';

  var title = document.createElement('h3');
  title.textContent = 'Compliance Letter';
  modal.appendChild(title);

  var meta = document.createElement('div');
  meta.className = 'compliance-status-meta';
  meta.textContent = 'Framework: ' + (response.framework || 'nist') + ' · Model: ' + (response.model || 'unknown');
  modal.appendChild(meta);

  var body = document.createElement('div');
  body.className = 'compliance-letter-body';
  body.innerHTML = renderMarkdown(response.markdown || '');
  modal.appendChild(body);

  var actions = document.createElement('div');
  actions.className = 'configure-actions';

  var copyBtn = document.createElement('button');
  copyBtn.className = 'btn-ext';
  copyBtn.textContent = 'Copy';
  copyBtn.addEventListener('click', function() {
    navigator.clipboard.writeText(response.markdown || '').then(function() {
      showToast('Compliance letter copied', 'success');
    }).catch(function() {
      showToast('Failed to copy compliance letter', 'error');
    });
  });
  actions.appendChild(copyBtn);

  var closeBtn = document.createElement('button');
  closeBtn.className = 'btn-ext activate';
  closeBtn.textContent = 'Close';
  closeBtn.addEventListener('click', closeComplianceLetterModal);
  actions.appendChild(closeBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);
}

function generateComplianceLetter() {
  openComplianceLetterRequestModal();
}

function loadSettings() {
  var requestVersion = beginRequest('settings');
  var el = document.getElementById('settings-list');
  el.innerHTML = '<div class="empty-state">Loading\u2026</div>';
  apiFetch('/api/settings').then(function(data) {
    if (!isCurrentRequest('settings', requestVersion)) return;
    if (!data.settings || data.settings.length === 0) {
      el.innerHTML = '<div class="empty-state">No settings configured. Use \u201c+ New\u201d to add one.</div>';
      return;
    }
    var tbl = document.createElement('table');
    tbl.className = 'routines-table';
    tbl.innerHTML = '<thead><tr><th>Key</th><th>Value</th><th>Updated</th><th></th></tr></thead>';
    var tbody = document.createElement('tbody');
    data.settings.forEach(function(s) {
      tbody.appendChild(renderSettingRow(s));
    });
    tbl.appendChild(tbody);
    el.innerHTML = '';
    el.appendChild(tbl);
  }).catch(function(err) {
    if (!isCurrentRequest('settings', requestVersion)) return;
    el.innerHTML = '<div class="empty-state">Failed to load: ' + escapeHtml(err.message) + '</div>';
  });
}

function renderSettingRow(s) {
  var tr = document.createElement('tr');
  tr.style.cursor = 'pointer';
  tr.addEventListener('click', function(e) {
    if (e.target.tagName === 'BUTTON') return;
    openSettingModal(s.key, s.value);
  });

  var keyTd = document.createElement('td');
  keyTd.style.fontFamily = 'var(--font-mono)';
  keyTd.textContent = s.key;

  var valTd = document.createElement('td');
  valTd.style.maxWidth = '340px';
  valTd.style.overflow = 'hidden';
  valTd.style.textOverflow = 'ellipsis';
  valTd.style.whiteSpace = 'nowrap';
  valTd.style.color = 'var(--text-secondary)';
  valTd.style.fontFamily = 'var(--font-mono)';
  var display = typeof s.value === 'string' ? s.value : JSON.stringify(s.value);
  valTd.textContent = display;

  var timeTd = document.createElement('td');
  timeTd.style.color = 'var(--text-secondary)';
  timeTd.textContent = formatDate(s.updated_at);

  var actionTd = document.createElement('td');
  var delBtn = document.createElement('button');
  delBtn.className = 'btn-ext remove';
  delBtn.textContent = 'Delete';
  delBtn.addEventListener('click', function() { deleteSetting(s.key); });
  actionTd.appendChild(delBtn);

  tr.appendChild(keyTd);
  tr.appendChild(valTd);
  tr.appendChild(timeTd);
  tr.appendChild(actionTd);
  return tr;
}

function openSettingModal(key, value) {
  closeSettingModal();
  var isNew = key === null;

  var overlay = document.createElement('div');
  overlay.className = 'configure-overlay';
  overlay.id = 'setting-modal-overlay';
  overlay.addEventListener('click', function(e) {
    if (e.target === overlay) closeSettingModal();
  });

  var modal = document.createElement('div');
  modal.className = 'configure-modal';

  var header = document.createElement('h3');
  header.textContent = isNew ? 'New Setting' : 'Edit Setting';
  modal.appendChild(header);

  var form = document.createElement('div');
  form.className = 'configure-form';

  // Key field
  var keyField = document.createElement('div');
  keyField.className = 'configure-field';
  var keyLabel = document.createElement('label');
  keyLabel.textContent = 'Key';
  keyField.appendChild(keyLabel);
  var keyInput;
  if (isNew) {
    keyInput = document.createElement('input');
    keyInput.type = 'text';
    keyInput.className = 'configure-input';
    keyInput.placeholder = 'e.g. model, system_prompt, max_tokens';
    keyInput.style.fontFamily = 'var(--font-mono)';
  } else {
    keyInput = document.createElement('div');
    keyInput.style.fontFamily = 'var(--font-mono)';
    keyInput.style.color = 'var(--text-secondary)';
    keyInput.style.padding = '8px 0';
    keyInput.style.fontSize = '13px';
    keyInput.textContent = key;
  }
  keyField.appendChild(keyInput);
  form.appendChild(keyField);

  // Value field
  var valField = document.createElement('div');
  valField.className = 'configure-field';
  var valLabel = document.createElement('label');
  valLabel.textContent = 'Value (JSON)';
  valField.appendChild(valLabel);
  var valInput = document.createElement('textarea');
  valInput.className = 'configure-input';
  valInput.rows = 6;
  valInput.style.fontFamily = 'var(--font-mono)';
  valInput.style.fontSize = '12px';
  valInput.style.resize = 'vertical';
  if (!isNew) {
    valInput.value = typeof value === 'string' ? JSON.stringify(value) : JSON.stringify(value, null, 2);
  }
  valField.appendChild(valInput);

  // Inline error
  var errMsg = document.createElement('div');
  errMsg.style.color = 'var(--danger)';
  errMsg.style.fontSize = '12px';
  errMsg.style.marginTop = '4px';
  errMsg.style.display = 'none';
  valField.appendChild(errMsg);
  form.appendChild(valField);
  modal.appendChild(form);

  // Actions
  var actions = document.createElement('div');
  actions.className = 'configure-actions';

  var saveBtn = document.createElement('button');
  saveBtn.className = 'btn-ext activate';
  saveBtn.textContent = 'Save';
  saveBtn.addEventListener('click', function() {
    var resolvedKey = isNew ? keyInput.value.trim() : key;
    if (!resolvedKey) {
      errMsg.textContent = 'Key is required.';
      errMsg.style.display = 'block';
      keyInput.focus();
      return;
    }
    var parsed;
    try {
      parsed = JSON.parse(valInput.value);
    } catch (e) {
      errMsg.textContent = 'Invalid JSON: ' + e.message;
      errMsg.style.display = 'block';
      valInput.focus();
      return;
    }
    errMsg.style.display = 'none';
    saveBtn.disabled = true;
    saveBtn.textContent = 'Saving\u2026';
    apiFetch('/api/settings/' + encodeURIComponent(resolvedKey), {
      method: 'PUT',
      body: { value: parsed },
    }).then(function() {
      closeSettingModal();
      showToast('Setting saved', 'success');
      loadSettings();
    }).catch(function(err) {
      saveBtn.disabled = false;
      saveBtn.textContent = 'Save';
      errMsg.textContent = 'Save failed: ' + err.message;
      errMsg.style.display = 'block';
    });
  });
  actions.appendChild(saveBtn);

  var cancelBtn = document.createElement('button');
  cancelBtn.className = 'btn-ext';
  cancelBtn.textContent = 'Cancel';
  cancelBtn.addEventListener('click', closeSettingModal);
  actions.appendChild(cancelBtn);

  modal.appendChild(actions);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  // Focus the right input
  if (isNew) {
    keyInput.focus();
  } else {
    valInput.focus();
    valInput.setSelectionRange(0, 0);
  }
}

function closeSettingModal() {
  var existing = document.getElementById('setting-modal-overlay');
  if (existing) existing.remove();
}

function deleteSetting(key) {
  if (!confirm('Delete setting \u201c' + key + '\u201d?')) return;
  apiFetch('/api/settings/' + encodeURIComponent(key), { method: 'DELETE' })
    .then(function() {
      showToast('Setting deleted', 'success');
      loadSettings();
    })
    .catch(function(err) {
      showToast('Delete failed: ' + err.message, 'error');
    });
}

function exportSettings() {
  apiFetch('/api/settings/export').then(function(data) {
    var blob = new Blob([JSON.stringify(data.settings, null, 2)], { type: 'application/json' });
    var url = URL.createObjectURL(blob);
    var a = document.createElement('a');
    a.href = url;
    a.download = 'settings.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }).catch(function(err) {
    showToast('Export failed: ' + err.message, 'error');
  });
}

function importSettings(file) {
  var reader = new FileReader();
  reader.onload = function(e) {
    var parsed;
    try {
      parsed = JSON.parse(e.target.result);
    } catch (err) {
      showToast('Invalid JSON file', 'error');
      return;
    }
    // Accept either { settings: {...} } (export format) or a plain object
    var settingsMap = (parsed && typeof parsed.settings === 'object' && !Array.isArray(parsed.settings))
      ? parsed.settings
      : parsed;
    if (settingsMap === null || typeof settingsMap !== 'object' || Array.isArray(settingsMap)) {
      showToast('Unrecognised format — expected a JSON object', 'error');
      return;
    }
    var count = Object.keys(settingsMap).length;
    apiFetch('/api/settings/import', {
      method: 'POST',
      body: { settings: settingsMap },
    }).then(function() {
      showToast('Imported ' + count + ' setting' + (count === 1 ? '' : 's'), 'success');
      loadSettings();
    }).catch(function(err) {
      showToast('Import failed: ' + err.message, 'error');
    });
  };
  reader.readAsText(file);
}

function renderBackupResult(lines, tone) {
  var container = document.getElementById('backups-last-result');
  if (!container) return;
  container.textContent = lines.join('\n');
  container.style.color = tone === 'error' ? 'var(--error)' : 'var(--text-secondary)';
}

function createBackup() {
  renderBackupResult(['Creating encrypted backup…'], 'normal');
  apiFetch('/api/backups/create', {
    method: 'POST',
    body: { include_ai_packets: false },
  }).then(function(data) {
    var artifact = data && data.artifact ? data.artifact : null;
    if (!artifact) throw new Error('Backup create response missing artifact');
    lastBackupId = artifact.id;
    var lines = [
      'Backup created',
      'id: ' + artifact.id,
      'path: ' + artifact.path,
      'size: ' + artifact.size_bytes + ' bytes',
      'sha256: ' + artifact.plaintext_sha256,
    ];
    if (Array.isArray(data.warnings) && data.warnings.length) {
      lines.push('warnings:');
      for (var i = 0; i < data.warnings.length; i++) {
        lines.push('- ' + data.warnings[i]);
      }
    }
    renderBackupResult(lines, 'normal');
    showToast('Backup created: ' + artifact.id, 'success');

    var a = document.createElement('a');
    a.href = '/api/backups/' + encodeURIComponent(artifact.id) + '/download?token=' + encodeURIComponent(token);
    a.download = artifact.id + '.clawyerbak';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }).catch(function(err) {
    renderBackupResult(['Backup creation failed', String(err.message || err)], 'error');
    showToast('Backup creation failed: ' + err.message, 'error');
  });
}

function verifyLastBackup() {
  if (!lastBackupId) {
    showToast('Create a backup first or restore one before verify.', 'error');
    return;
  }
  renderBackupResult(['Verifying backup ' + lastBackupId + '…'], 'normal');
  apiFetch('/api/backups/verify', {
    method: 'POST',
    body: { backup_id: lastBackupId },
  }).then(function(data) {
    var lines = [
      'Backup verification: ' + (data.valid ? 'PASS' : 'FAIL'),
      'backup_id: ' + lastBackupId,
    ];
    if (Array.isArray(data.warnings) && data.warnings.length) {
      lines.push('warnings:');
      for (var i = 0; i < data.warnings.length; i++) {
        lines.push('- ' + data.warnings[i]);
      }
    }
    renderBackupResult(lines, data.valid ? 'normal' : 'error');
    showToast(data.valid ? 'Backup verification passed' : 'Backup verification failed', data.valid ? 'success' : 'error');
  }).catch(function(err) {
    renderBackupResult(['Backup verify failed', String(err.message || err)], 'error');
    showToast('Backup verify failed: ' + err.message, 'error');
  });
}

function triggerBackupRestoreInput() {
  var input = document.getElementById('backups-restore-file');
  if (!input) return;
  input.click();
}

function restoreBackupFromFile(file, apply) {
  if (!file) return;
  var mode = apply ? 'apply' : 'dry-run';
  renderBackupResult(['Running backup restore (' + mode + ') for ' + file.name + '…'], 'normal');

  var form = new FormData();
  form.append('file', file);
  form.append('apply', apply ? 'true' : 'false');
  form.append('protect_identity_files', 'true');

  apiFetch('/api/backups/restore', {
    method: 'POST',
    body: form,
  }).then(function(data) {
    var lines = [
      'Backup restore complete',
      'mode: ' + (data.applied ? 'APPLIED' : 'DRY-RUN'),
      'restored settings: ' + data.restored_settings,
      'restored workspace files: ' + data.restored_workspace_files,
      'skipped workspace files: ' + data.skipped_workspace_files,
    ];
    if (Array.isArray(data.warnings) && data.warnings.length) {
      lines.push('warnings:');
      for (var i = 0; i < data.warnings.length; i++) {
        lines.push('- ' + data.warnings[i]);
      }
    }
    renderBackupResult(lines, 'normal');
    showToast(data.applied ? 'Backup restore applied' : 'Backup restore dry-run complete', 'success');
  }).catch(function(err) {
    renderBackupResult(['Backup restore failed', String(err.message || err)], 'error');
    showToast('Backup restore failed: ' + err.message, 'error');
  });
}

// Wire up Settings action buttons after DOM is ready
document.addEventListener('DOMContentLoaded', function() {
  var addBtn = document.getElementById('settings-add-btn');
  if (addBtn) addBtn.addEventListener('click', function() { openSettingModal(null, null); });

  var exportBtn = document.getElementById('settings-export-btn');
  if (exportBtn) exportBtn.addEventListener('click', exportSettings);

  var importBtn = document.getElementById('settings-import-btn');
  var importFile = document.getElementById('settings-import-file');
  if (importBtn && importFile) {
    importBtn.addEventListener('click', function() { importFile.click(); });
    importFile.addEventListener('change', function() {
      if (importFile.files && importFile.files[0]) {
        importSettings(importFile.files[0]);
        importFile.value = '';
      }
    });
  }

  var backupRestoreFile = document.getElementById('backups-restore-file');
  if (backupRestoreFile) {
    backupRestoreFile.addEventListener('change', function() {
      if (backupRestoreFile.files && backupRestoreFile.files[0]) {
        var apply = confirm('Apply restore changes now? Click Cancel to run dry-run only.');
        restoreBackupFromFile(backupRestoreFile.files[0], apply);
        backupRestoreFile.value = '';
      }
    });
  }
});

