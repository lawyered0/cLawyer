//! Anti-detection JavaScript patches for headless Chrome.
//!
//! Injects scripts via `Page.addScriptToEvaluateOnNewDocument` to suppress
//! common bot-detection signals. Handles ~80% of detection for legitimate
//! browsing (not adversarial scraping against Cloudflare Enterprise).
//!
//! What we patch:
//! - `navigator.webdriver` (trivial but still checked)
//! - `navigator.plugins` (headless has empty plugin list)
//! - `navigator.languages` (match system locale)
//! - `chrome.runtime` (looks like a real extension API)
//! - `HeadlessChrome` user-agent substring (suppressed via launch flags)

/// Chrome launch arguments that reduce detection surface.
pub fn stealth_args() -> Vec<&'static str> {
    vec![
        "--disable-blink-features=AutomationControlled",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-infobars",
        "--disable-background-networking",
        "--disable-prompt-on-repost",
        "--disable-hang-monitor",
        "--disable-sync",
        "--metrics-recording-only",
        "--no-service-autorun",
    ]
}

/// JavaScript injected before any page scripts run.
///
/// This covers the most common fingerprinting checks. Each patch is
/// a self-contained IIFE so failures in one don't break the others.
pub fn stealth_js() -> &'static str {
    r#"
// --- navigator.webdriver ---
// CDP sets this to true; real browsers have it undefined or false.
(() => {
    Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
        configurable: true,
    });
})();

// --- navigator.plugins ---
// Headless Chrome reports an empty plugin array. Real Chrome on desktop
// always has at least these two. We fake the array shape.
(() => {
    const pluginData = [
        { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer',
          description: 'Portable Document Format' },
        { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai',
          description: '' },
    ];

    const makeMimeType = (type_, suffixes, desc, plugin) => {
        const mt = Object.create(MimeType.prototype);
        Object.defineProperties(mt, {
            type: { get: () => type_ },
            suffixes: { get: () => suffixes },
            description: { get: () => desc },
            enabledPlugin: { get: () => plugin },
        });
        return mt;
    };

    const makePlugin = (data) => {
        const p = Object.create(Plugin.prototype);
        const mimes = [makeMimeType('application/pdf', 'pdf', 'Portable Document Format', p)];
        Object.defineProperties(p, {
            name: { get: () => data.name },
            filename: { get: () => data.filename },
            description: { get: () => data.description },
            length: { get: () => mimes.length },
            0: { get: () => mimes[0] },
        });
        p.item = (i) => mimes[i] || null;
        p.namedItem = (name) => mimes.find(m => m.type === name) || null;
        return p;
    };

    const plugins = pluginData.map(makePlugin);
    const pluginArray = Object.create(PluginArray.prototype);
    Object.defineProperties(pluginArray, {
        length: { get: () => plugins.length },
        0: { get: () => plugins[0] },
        1: { get: () => plugins[1] },
    });
    pluginArray.item = (i) => plugins[i] || null;
    pluginArray.namedItem = (name) => plugins.find(p => p.name === name) || null;
    pluginArray.refresh = () => {};
    pluginArray[Symbol.iterator] = function* () { yield* plugins; };

    Object.defineProperty(navigator, 'plugins', {
        get: () => pluginArray,
        configurable: true,
    });
})();

// --- navigator.languages ---
// Headless sometimes reports just ['en'] instead of a realistic list.
(() => {
    Object.defineProperty(navigator, 'languages', {
        get: () => ['en-US', 'en'],
        configurable: true,
    });
})();

// --- chrome.runtime ---
// Bot detectors check for chrome.runtime to see if it's a real Chrome
// extension environment. CDP-controlled Chrome has a broken stub.
(() => {
    if (!window.chrome) window.chrome = {};
    if (!window.chrome.runtime) {
        window.chrome.runtime = {
            connect: () => {},
            sendMessage: () => {},
            id: undefined,
        };
    }
})();

// --- Permissions API ---
// Headless reports 'denied' for notification permissions by default,
// which is a known fingerprinting signal.
(() => {
    const originalQuery = window.Permissions?.prototype?.query;
    if (originalQuery) {
        window.Permissions.prototype.query = function(params) {
            if (params?.name === 'notifications') {
                return Promise.resolve({ state: 'prompt', onchange: null });
            }
            return originalQuery.call(this, params);
        };
    }
})();
"#
}

#[cfg(test)]
mod tests {
    use crate::tools::builtin::browser::stealth;

    #[test]
    fn stealth_js_is_not_empty() {
        let js = stealth::stealth_js();
        assert!(js.len() > 100);
        assert!(js.contains("navigator"));
        assert!(js.contains("webdriver"));
    }

    #[test]
    fn stealth_args_are_valid_flags() {
        for arg in stealth::stealth_args() {
            assert!(arg.starts_with("--"), "arg should start with --: {}", arg);
        }
    }
}
