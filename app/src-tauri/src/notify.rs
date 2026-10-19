//! System notifications: a meeting's notes are ready, or couldn't be
//! written. Clicking one opens the meeting in Quill.
//!
//! macOS uses the User Notifications framework directly (asking permission
//! the first time); Windows shows a toast. Only shown when Quill isn't in
//! front: in the app, a toast inside the window says it instead.

use tauri::AppHandle;

/// Shows a notification about meeting `id`.
pub fn meeting(app: &AppHandle, id: &str, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    mac::show(app, id, title, body);
    #[cfg(windows)]
    win::show(app, id, title, body);
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (app, id, title, body);
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    use std::sync::OnceLock;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use tauri::AppHandle;

    /// Request ids are this plus the meeting id, so a click knows which
    /// meeting, and a newer notification for a meeting replaces the older.
    const PREFIX: &str = "quill.meeting.";

    /// For the delegate, which AppKit calls without context.
    static APP: OnceLock<AppHandle> = OnceLock::new();

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the
        // delegate has no ivars.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "QuillNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            // Shown even while Quill is the active app (its window may be
            // hidden or behind the call).
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }

            // Clicked: open the meeting.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                handler: &block2::DynBlock<dyn Fn()>,
            ) {
                let id = response.notification().request().identifier().to_string();
                if let (Some(meeting), Some(app)) = (id.strip_prefix(PREFIX), APP.get()) {
                    crate::open_meeting(app, meeting);
                }
                handler.call(());
            }
        }
    );

    pub fn show(app: &AppHandle, id: &str, title: &str, body: &str) {
        let app2 = app.clone();
        let (id, title, body) = (id.to_string(), title.to_string(), body.to_string());
        let _ = app.run_on_main_thread(move || {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            // The framework needs a real app bundle; a bare development
            // binary has none (and would crash).
            if NSBundle::mainBundle().bundleIdentifier().is_none() {
                tracing::debug!("no notifications outside an app bundle");
                return;
            }
            let center = UNUserNotificationCenter::currentNotificationCenter();
            if APP.set(app2).is_ok() {
                // SAFETY: `init` is NSObject's initialiser, and the class adds
                // no ivars to set up.
                let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
                center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                // The centre holds its delegate weakly; this one lives as
                // long as the app.
                std::mem::forget(delegate);
            }

            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&title));
            content.setBody(&NSString::from_str(&body));
            content.setSound(Some(&UNNotificationSound::defaultSound()));
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&format!("{PREFIX}{id}")),
                &content,
                None,
            );
            // Asks the first time; after that it answers at once.
            let center2 = center.clone();
            let posted = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                if granted.as_bool() {
                    center2.addNotificationRequest_withCompletionHandler(&request, None);
                } else {
                    tracing::info!("notifications aren't allowed; not shown");
                }
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &posted,
            );
        });
    }
}

#[cfg(windows)]
mod win {
    use std::sync::Mutex;

    use tauri::AppHandle;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    /// The last toast, kept so its click handler stays alive.
    static LAST: Mutex<Option<ToastNotification>> = Mutex::new(None);

    pub fn show(app: &AppHandle, id: &str, title: &str, body: &str) {
        if let Err(e) = try_show(app, id, title, body) {
            tracing::warn!("couldn't show a notification: {e}");
        }
    }

    fn try_show(app: &AppHandle, id: &str, title: &str, body: &str) -> windows::core::Result<()> {
        let xml = XmlDocument::new()?;
        xml.LoadXml(&HSTRING::from(format!(
            "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
            escape(title),
            escape(body)
        )))?;
        let toast = ToastNotification::CreateToastNotification(&xml)?;
        toast.SetTag(&HSTRING::from(id))?;
        let app2 = app.clone();
        let meeting = id.to_string();
        toast.Activated(&TypedEventHandler::new(move |_, _| {
            crate::open_meeting(&app2, &meeting);
            Ok(())
        }))?;
        // The installer registers the app under its identifier.
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
            app.config().identifier.as_str(),
        ))?;
        notifier.Show(&toast)?;
        *LAST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(toast);
        Ok(())
    }

    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
}
