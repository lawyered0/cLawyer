// --- Tabs ---

document.querySelectorAll('.tab-bar [data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => {
    const tab = btn.getAttribute('data-tab');
    const settingsSection = btn.getAttribute('data-settings-section');
    switchTab(tab, settingsSection ? { settingsSection: settingsSection } : null);
  });
});

function closeTabOverflowMenu() {
  var menu = byId('tab-overflow-menu');
  if (!menu) return;
  menu.classList.add('is-hidden');
  menu.classList.remove('floating');
  menu.style.left = '';
  menu.style.top = '';
}

function toggleTabOverflowMenu(event) {
  if (event) event.preventDefault();
  var menu = byId('tab-overflow-menu');
  if (!menu) return;
  var wasHidden = menu.classList.contains('is-hidden');
  menu.classList.toggle('is-hidden');
  if (wasHidden) {
    positionTabOverflowMenu();
  } else {
    closeTabOverflowMenu();
  }
}

function positionTabOverflowMenu() {
  var menu = byId('tab-overflow-menu');
  var trigger = byId('tab-overflow-trigger');
  if (!menu || !trigger || menu.classList.contains('is-hidden')) return;
  menu.classList.add('floating');
  var triggerRect = trigger.getBoundingClientRect();
  var menuWidth = Math.max(200, Math.min(menu.offsetWidth || 200, window.innerWidth - 16));
  var left = Math.max(8, Math.min(triggerRect.right - menuWidth, window.innerWidth - menuWidth - 8));
  var top = Math.max(8, Math.min(triggerRect.bottom + 6, window.innerHeight - menu.offsetHeight - 8));
  menu.style.left = left + 'px';
  menu.style.top = top + 'px';
}

function handleDocumentClickForMenus(event) {
  var overflow = byId('tab-overflow');
  var menu = byId('tab-overflow-menu');
  if (overflow && menu && !menu.classList.contains('is-hidden') && !overflow.contains(event.target)) {
    closeTabOverflowMenu();
  }
}

function openSettingsSection(section) {
  var next = section === 'logs' ? 'logs' : 'general';
  currentSettingsSection = next;
  setCurrentSettingsSection(next);
  var generalBtn = byId('settings-section-general-btn');
  var logsBtn = byId('settings-section-logs-btn');
  var generalSection = byId('settings-section-general');
  var logsSection = byId('settings-section-logs');

  if (generalBtn) generalBtn.classList.toggle('active', next === 'general');
  if (logsBtn) logsBtn.classList.toggle('active', next === 'logs');
  if (generalSection) generalSection.classList.toggle('active', next === 'general');
  if (logsSection) logsSection.classList.toggle('active', next === 'logs');

  if (next === 'general') {
    loadSettings();
    loadComplianceStatus();
    return;
  }
  applyLogFilters();
  loadLegalAudit(0);
}

function switchTab(tab, options) {
  if (!tab) return;
  currentTab = tab;
  setCurrentTab(tab);

  var settingsSection = options && options.settingsSection
    ? options.settingsSection
    : currentSettingsSection;

  document.querySelectorAll('.tab-bar > button[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.getAttribute('data-tab') === tab);
  });
  document.querySelectorAll('#tab-overflow-menu button[data-tab]').forEach((b) => {
    var buttonTab = b.getAttribute('data-tab');
    var buttonSection = b.getAttribute('data-settings-section') || 'general';
    var active = buttonTab === tab;
    if (tab === 'settings') {
      active = active && buttonSection === settingsSection;
    }
    b.classList.toggle('active', active);
  });

  var overflowTrigger = byId('tab-overflow-trigger');
  if (overflowTrigger) {
    overflowTrigger.classList.toggle('active', OVERFLOW_TABS.indexOf(tab) !== -1);
  }

  document.querySelectorAll('.tab-panel').forEach((p) => {
    p.classList.toggle('active', p.id === 'tab-' + tab);
  });

  closeTabOverflowMenu();

  if (tab === 'memory') loadMemoryTree();
  if (tab === 'jobs') loadJobs();
  if (tab === 'routines') loadRoutines();
  if (tab === 'extensions') loadExtensions();
  if (tab === 'skills') loadSkills();
  if (tab === 'matters') loadMatters();
  if (tab === 'settings') openSettingsSection(settingsSection);
}

