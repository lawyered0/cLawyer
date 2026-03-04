// cLawyer Web Gateway - Client

const core = window.__clawyerCore || {};
const appState = core.appState;
const setAuthToken = core.setAuthToken;
const setCurrentTab = core.setCurrentTab;
const setCurrentSettingsSection = core.setCurrentSettingsSection;
const byId = core.byId;
const bindClick = core.bindClick;
const bindChange = core.bindChange;
const bindKeydown = core.bindKeydown;
const delegate = core.delegate;
const apiFetch = core.apiFetch;
const beginRequest = core.beginRequest;
const isCurrentRequest = core.isCurrentRequest;
const PRIMARY_TABS = core.PRIMARY_TABS;
const OVERFLOW_TABS = core.OVERFLOW_TABS;
const SHORTCUT_TABS = core.SHORTCUT_TABS;

if (!appState || !setAuthToken || !setCurrentTab || !setCurrentSettingsSection || !byId || !bindClick || !bindChange || !bindKeydown || !delegate || !apiFetch || !beginRequest || !isCurrentRequest || !PRIMARY_TABS || !OVERFLOW_TABS || !SHORTCUT_TABS) {
  throw new Error('cLawyer core bootstrap missing (window.__clawyerCore)');
}

let token = appState.authToken || '';
let eventSource = null;
let logEventSource = null;
let currentTab = appState.currentTab || 'chat';
let currentThreadId = null;
let assistantThreadId = null;
let hasMore = false;
let oldestTimestamp = null;
let loadingOlder = false;
let sseHasConnectedBefore = false;
let jobEvents = new Map(); // job_id -> Array of events
let jobListRefreshTimer = null;
const JOB_EVENTS_CAP = 500;
const MEMORY_SEARCH_QUERY_MAX_LENGTH = 100;
let lastBackupId = null;
let currentSettingsSection = appState.currentSettingsSection || 'general';
let matterCreateModalLastFocus = null;
let complianceStatusCache = null;
let complianceExpanded = false;

// --- Tool Activity State ---
let _activeGroup = null;
let _activeToolCards = {};
let _activityThinking = null;

