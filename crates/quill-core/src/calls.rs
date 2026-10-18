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
/// blip, short enough that little of the meeting is missed before the prompt.
/// A long voice search can still bring it up; it goes away on its own.
pub const SETTLE: Duration = Duration::from_secs(2);
/// A call app letting go of the mic for at most this long keeps its place, so
/// going from a preview screen into the call doesn't restart [`SETTLE`].
pub const BLIP: Duration = Duration::from_secs(3);
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
    /// When each call app using the mic (or briefly not, up to [`BLIP`])
    /// was first seen, and last seen.
    since: HashMap<&'static str, (Instant, Instant)>,
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
        for app in &active {
            self.since.entry(app).or_insert((now, now)).1 = now;
        }
        self.since
            .retain(|_, (_, last_seen)| now.duration_since(*last_seen) <= BLIP);

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

        // The app using the mic that's held it longest, once that's long enough.
        let (app, (since, _)) = self
            .since
            .iter()
            .filter(|(app, _)| active.contains(app))
            .min_by_key(|(_, (since, _))| *since)?;
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

    /// Polls twice a second, as the app does, from `from` to `to` seconds
    /// after `start` with `ids` in use. Events are reported in seconds.
    fn poll(
        d: &mut Detector,
        start: Instant,
        from: f64,
        to: f64,
        ids: &[&str],
    ) -> Vec<(f64, CallEvent)> {
        let ticks = ((to - from) * 2.0).round() as u32;
        (0..ticks)
            .filter_map(|i| {
                let s = from + f64::from(i) / 2.0;
                d.update(start + Duration::from_secs_f64(s), ids.iter().copied(), OWN)
                    .map(|e| (s, e))
            })
            .collect()
    }

    #[test]
    fn a_call_starts_after_settling_and_ends_after_the_grace_period() {
        let t = Instant::now();
        let mut d = Detector::new();
        assert_eq!(
            poll(&mut d, t, 0.0, 30.0, &["us.zoom.xos", OWN]),
            [(2.0, CallEvent::Started { app: "Zoom" })]
        );
        assert_eq!(d.current(), Some("Zoom"));
        // A short gap doesn't end it...
        assert!(poll(&mut d, t, 30.0, 40.0, &[]).is_empty());
        assert!(poll(&mut d, t, 40.0, 50.0, &["us.zoom.xos"]).is_empty());
        // ...but 15 seconds without the mic does, once.
        assert_eq!(
            poll(&mut d, t, 50.0, 80.0, &[]),
            [(64.5, CallEvent::Ended { app: "Zoom" })]
        );
        assert_eq!(d.current(), None);
    }

    #[test]
    fn brief_mic_use_and_quill_itself_never_start_a_call() {
        let t = Instant::now();
        let mut d = Detector::new();
        // A blip: Chrome for a second and a half.
        assert!(poll(&mut d, t, 0.0, 1.5, &["com.google.Chrome.helper"]).is_empty());
        assert!(poll(&mut d, t, 1.5, 20.0, &[]).is_empty());
        // Dictation, and an app that isn't for calls.
        assert!(poll(&mut d, t, 20.0, 60.0, &[OWN, "com.apple.VoiceMemos"]).is_empty());
    }

    #[test]
    fn the_first_app_on_the_mic_names_the_call() {
        let t = Instant::now();
        let mut d = Detector::new();
        assert!(poll(&mut d, t, 0.0, 1.0, &["com.tinyspeck.slackmacgap"]).is_empty());
        let events = poll(
            &mut d,
            t,
            1.0,
            10.0,
            &["com.tinyspeck.slackmacgap", "us.zoom.xos"],
        );
        assert_eq!(events, [(2.0, CallEvent::Started { app: "Slack" })]);
    }

    #[test]
    fn a_brief_gap_keeps_the_clock_running() {
        // Meet's preview screen holds the mic, lets go for a moment on
        // joining, then takes it again: the call started when it first did.
        let t = Instant::now();
        let mut d = Detector::new();
        let meet = ["com.google.Chrome.helper"];
        let chrome = CallEvent::Started {
            app: "Google Chrome",
        };
        assert!(poll(&mut d, t, 0.0, 1.5, &meet).is_empty());
        assert!(poll(&mut d, t, 1.5, 2.5, &[]).is_empty());
        assert_eq!(poll(&mut d, t, 2.5, 5.0, &meet), [(2.5, chrome.clone())]);

        // A longer gap starts the clock again.
        let mut d = Detector::new();
        assert!(poll(&mut d, t, 0.0, 1.5, &meet).is_empty());
        assert!(poll(&mut d, t, 1.5, 6.0, &[]).is_empty());
        assert_eq!(poll(&mut d, t, 6.0, 10.0, &meet), [(8.0, chrome)]);
    }
}
