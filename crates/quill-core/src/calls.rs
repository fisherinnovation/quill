//! Noticing calls: when a call app starts using the microphone, so Quill can
//! offer to record, and when it stops, so it can offer to stop.
//!
//! The only input is which apps are using a microphone (their bundle ids,
//! from `quill_audio::mic_users`): never audio, windows or what's on screen.
//! So a call in a browser is "Google Chrome is using your microphone", not
//! "Google Meet": telling Meet from any other tab would mean reading the tab.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Apps whose use of the microphone means a call, by bundle id (macOS) or
/// prefix of one: browsers record from helper processes such as
/// `com.google.Chrome.helper`.
const CALL_APPS: &[(&str, &str)] = &[
    ("us.zoom.xos", "Zoom"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.microsoft.teams", "Microsoft Teams"),
    ("com.cisco.webexmeetingsapp", "Webex"),
    ("Cisco-Systems.Spark", "Webex"),
    ("com.apple.FaceTime", "FaceTime"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.hnc.Discord", "Discord"),
    ("com.google.Chrome", "Google Chrome"),
    ("com.microsoft.edgemac", "Microsoft Edge"),
    ("company.thebrowser.Browser", "Arc"),
    ("com.brave.Browser", "Brave"),
    ("org.mozilla.firefox", "Firefox"),
    ("com.apple.Safari", "Safari"),
    // Safari records from WebKit's shared GPU process.
    ("com.apple.WebKit.GPU", "Safari"),
];

/// The display name of the call app `bundle_id` belongs to, if it's one.
pub fn call_app(bundle_id: &str) -> Option<&'static str> {
    CALL_APPS.iter().find_map(|(id, name)| {
        let rest = bundle_id.strip_prefix(id)?;
        (rest.is_empty() || rest.starts_with('.')).then_some(*name)
    })
}

/// A call holding the mic this long counts as started: long enough to skip a
/// voice search or a browser's mic test.
pub const SETTLE: Duration = Duration::from_secs(5);
/// And it counts as over once the mic has been free this long, so a brief
/// gap (switching devices, rejoining) doesn't end it.
pub const GRACE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallEvent {
    /// `app` has been using the mic for [`SETTLE`].
    Started { app: &'static str },
    /// The call that started has let go of the mic for [`GRACE`].
    Ended { app: &'static str },
}

/// Turns polls of the mic's users into call starts and ends. Time is passed
/// in, so it's testable.
#[derive(Debug, Default)]
pub struct Detector {
    /// When each call app currently using the mic was first seen.
    since: HashMap<&'static str, Instant>,
    /// The call reported as started, and when its app was last seen.
    call: Option<(&'static str, Instant)>,
}

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The call in progress, if one has started and not ended.
    pub fn current(&self) -> Option<&'static str> {
        self.call.map(|(app, _)| app)
    }

    /// One poll: the bundle ids using a microphone at `now`. Quill's own use
    /// (`own_bundle_id`) never counts.
    pub fn update<'a>(
        &mut self,
        now: Instant,
        bundle_ids: impl IntoIterator<Item = &'a str>,
        own_bundle_id: &str,
    ) -> Option<CallEvent> {
        let active: Vec<&'static str> = bundle_ids
            .into_iter()
            .filter(|id| *id != own_bundle_id)
            .filter_map(call_app)
            .collect();
        self.since.retain(|app, _| active.contains(app));
        for app in &active {
            self.since.entry(app).or_insert(now);
        }

        if let Some((app, last_seen)) = self.call {
            if active.contains(&app) {
                self.call = Some((app, now));
                return None;
            }
            if now.duration_since(last_seen) >= GRACE {
                self.call = None;
                return Some(CallEvent::Ended { app });
            }
            return None;
        }

        // The app that's held the mic longest, once it's held it long enough.
        let (app, since) = self.since.iter().min_by_key(|(_, since)| **since)?;
        if now.duration_since(*since) >= SETTLE {
            let app = *app;
            self.call = Some((app, now));
            return Some(CallEvent::Started { app });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "com.fisherinnovation.quill";

    #[test]
    fn knows_call_apps_and_browser_helpers() {
        assert_eq!(call_app("us.zoom.xos"), Some("Zoom"));
        assert_eq!(call_app("com.google.Chrome.helper"), Some("Google Chrome"));
        assert_eq!(call_app("com.google.Chrome"), Some("Google Chrome"));
        assert_eq!(call_app("com.microsoft.teams2"), Some("Microsoft Teams"));
        // A prefix must end at a dot: not a different app that starts the same.
        assert_eq!(call_app("com.google.ChromeRemoteDesktop"), None);
        assert_eq!(call_app("com.apple.VoiceMemos"), None);
        assert_eq!(call_app(OWN), None);
    }

    /// Polls once a second from `start` for `secs` seconds with `ids` in use.
    fn poll(
        d: &mut Detector,
        start: Instant,
        from: u64,
        to: u64,
        ids: &[&str],
    ) -> Vec<(u64, CallEvent)> {
        (from..to)
            .filter_map(|s| {
                d.update(start + Duration::from_secs(s), ids.iter().copied(), OWN)
                    .map(|e| (s, e))
            })
            .collect()
    }

    #[test]
    fn a_call_starts_after_settling_and_ends_after_the_grace_period() {
        let t = Instant::now();
        let mut d = Detector::new();
        assert_eq!(
            poll(&mut d, t, 0, 30, &["us.zoom.xos", OWN]),
            [(5, CallEvent::Started { app: "Zoom" })]
        );
        assert_eq!(d.current(), Some("Zoom"));
        // A short gap doesn't end it...
        assert!(poll(&mut d, t, 30, 40, &[]).is_empty());
        assert!(poll(&mut d, t, 40, 50, &["us.zoom.xos"]).is_empty());
        // ...but 15 seconds without the mic does, once.
        assert_eq!(
            poll(&mut d, t, 50, 80, &[]),
            [(64, CallEvent::Ended { app: "Zoom" })]
        );
        assert_eq!(d.current(), None);
    }

    #[test]
    fn brief_mic_use_and_quill_itself_never_start_a_call() {
        let t = Instant::now();
        let mut d = Detector::new();
        // A voice search: Chrome for 3 seconds.
        assert!(poll(&mut d, t, 0, 3, &["com.google.Chrome.helper"]).is_empty());
        assert!(poll(&mut d, t, 3, 20, &[]).is_empty());
        // Dictation, and an app that isn't for calls.
        assert!(poll(&mut d, t, 20, 60, &[OWN, "com.apple.VoiceMemos"]).is_empty());
    }

    #[test]
    fn the_first_app_on_the_mic_names_the_call() {
        let t = Instant::now();
        let mut d = Detector::new();
        assert!(poll(&mut d, t, 0, 2, &["com.tinyspeck.slackmacgap"]).is_empty());
        let events = poll(
            &mut d,
            t,
            2,
            10,
            &["com.tinyspeck.slackmacgap", "us.zoom.xos"],
        );
        assert_eq!(events, [(5, CallEvent::Started { app: "Slack" })]);
    }
}
