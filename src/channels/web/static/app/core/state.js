// Shared frontend state surface for extracted core infrastructure.

export const appState = {
  authToken: '',
  currentTab: 'chat',
  currentSettingsSection: 'general',
  requestVersions: {
    memoryTree: 0,
    memorySearch: 0,
    memoryRead: 0,
    memoryDirectory: 0,
    jobsList: 0,
    jobDetail: 0,
    routinesList: 0,
    routineDetail: 0,
    matters: 0,
    matterDetail: 0,
    matterDetailWork: 0,
    matterDetailFinance: 0,
    legalAudit: 0,
    settings: 0,
    complianceStatus: 0,
    extensions: 0,
    skills: 0,
    gatewayStatus: 0,
  },
};

export function setAuthToken(token) {
  appState.authToken = token || '';
}

export function setCurrentTab(tab) {
  appState.currentTab = tab;
}

export function setCurrentSettingsSection(section) {
  appState.currentSettingsSection = section;
}
