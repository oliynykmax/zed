//! A `WKWebView` that serves its page from memory through a custom URL scheme
//! and passes string messages both ways.

use std::{borrow::Cow, ptr::NonNull};

use anyhow::{Context as _, Result};
use futures::channel::mpsc;
use gpui::{App, NativeView, Window};
use objc2::{
    AllocAnyThread as _, DefinedClass as _, class, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyObject, NSObject},
};
use objc2_foundation::{NSData, NSDictionary, NSRect, NSString, NSURL};

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

pub const SCHEME: &str = "zed-lean-infoview";
const PAGE_URL: &str = "zed-lean-infoview://infoview/index.html";
const MESSAGE_HANDLER: &str = "zed";

pub enum WebViewEvent {
    /// A string the page posted to `window.webkit.messageHandlers.zed`.
    Message(String),
    /// A web link the page tried to follow, which belongs in the browser.
    Link(String),
}

pub struct Resource {
    pub mime_type: Cow<'static, str>,
    pub bytes: Cow<'static, [u8]>,
}

/// Serves the page's files by URL path, such as `/index.html`.
pub type Resolve = Box<dyn Fn(&str) -> Option<Resource>>;

struct HandlerIvars {
    resolve: Resolve,
    events: mpsc::UnboundedSender<WebViewEvent>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and `Handler` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = HandlerIvars]
    #[name = "ZedLeanInfoviewHandler"]
    struct Handler;

    impl Handler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start_url_scheme_task(&self, _web_view: &AnyObject, task: &AnyObject) {
            // SAFETY: WebKit passes a live `WKURLSchemeTask` on the main thread.
            unsafe { self.serve(task) }
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop_url_scheme_task(&self, _web_view: &AnyObject, _task: &AnyObject) {}

        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive_script_message(&self, _controller: &AnyObject, message: &AnyObject) {
            // SAFETY: WebKit passes a live `WKScriptMessage`.
            let body: Option<Retained<AnyObject>> = unsafe { msg_send![message, body] };
            let Some(body) = body.and_then(|body| body.downcast::<NSString>().ok()) else {
                return;
            };
            // Nothing is receiving once the panel is gone.
            self.ivars()
                .events
                .unbounded_send(WebViewEvent::Message(body.to_string()))
                .ok();
        }

        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy_for_navigation_action(
            &self,
            _web_view: &AnyObject,
            action: &AnyObject,
            decision_handler: &block2::DynBlock<dyn Fn(isize)>,
        ) {
            const CANCEL: isize = 0;
            const ALLOW: isize = 1;
            // SAFETY: WebKit passes a live `WKNavigationAction`.
            let navigation = unsafe { navigation(action) };
            match navigation {
                Navigation::Allow => decision_handler.call((ALLOW,)),
                Navigation::Cancel => decision_handler.call((CANCEL,)),
                Navigation::OpenInBrowser(url) => {
                    self.ivars().events.unbounded_send(WebViewEvent::Link(url)).ok();
                    decision_handler.call((CANCEL,));
                }
            }
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn web_content_process_did_terminate(&self, web_view: &AnyObject) {
            log::warn!("Lean infoview's web content process terminated, reloading");
            // SAFETY: WebKit passes the live `WKWebView` on the main thread.
            let _: Option<Retained<AnyObject>> = unsafe { msg_send![web_view, reload] };
        }
    }
);

#[derive(Debug, PartialEq)]
enum Navigation {
    Allow,
    Cancel,
    OpenInBrowser(String),
}

/// # Safety
///
/// `action` must be a live `WKNavigationAction`.
unsafe fn navigation(action: &AnyObject) -> Navigation {
    const LINK_ACTIVATED: isize = 0;
    unsafe {
        let request: Retained<AnyObject> = msg_send![action, request];
        let url: Option<Retained<NSURL>> = msg_send![&*request, URL];
        let url = url
            .and_then(|url| url.absoluteString())
            .map(|url| url.to_string());
        // A nil target frame is a request for a new window.
        let frame: Option<Retained<AnyObject>> = msg_send![action, targetFrame];
        let main_frame = frame.is_none_or(|frame| msg_send![&*frame, isMainFrame]);
        let navigation_type: isize = msg_send![action, navigationType];
        decide(
            url.as_deref(),
            main_frame,
            navigation_type == LINK_ACTIVATED,
        )
    }
}

/// Keeps the main frame on the page, and sends web links the user clicks to the browser.
fn decide(url: Option<&str>, main_frame: bool, link_activated: bool) -> Navigation {
    if !main_frame {
        return Navigation::Allow;
    }
    match url {
        Some(url) if url.split('#').next() == Some(PAGE_URL) => Navigation::Allow,
        Some(url) if link_activated && is_web_link(url) => Navigation::OpenInBrowser(url.into()),
        _ => Navigation::Cancel,
    }
}

fn is_web_link(url: &str) -> bool {
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
}

impl Handler {
    fn new(ivars: HandlerIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        // SAFETY: `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// # Safety
    ///
    /// `task` must be a live `WKURLSchemeTask`, and this must run on the main thread.
    unsafe fn serve(&self, task: &AnyObject) {
        unsafe {
            let request: Retained<AnyObject> = msg_send![task, request];
            let url: Option<Retained<NSURL>> = msg_send![&*request, URL];
            let Some(url) = url else {
                fail(task);
                return;
            };
            let path = url.path().map(|path| path.to_string()).unwrap_or_default();
            let (status, resource) = match (self.ivars().resolve)(&path) {
                Some(resource) => (200, resource),
                None => (
                    404,
                    Resource {
                        mime_type: "text/plain".into(),
                        bytes: Cow::Borrowed(b"not found"),
                    },
                ),
            };
            let headers = NSDictionary::from_slices(
                &[&*NSString::from_str("Content-Type")],
                &[&*NSString::from_str(&resource.mime_type)],
            );
            let response: Allocated<AnyObject> = msg_send![class!(NSHTTPURLResponse), alloc];
            let response: Option<Retained<AnyObject>> = msg_send![
                response,
                initWithURL: &*url,
                statusCode: status as isize,
                HTTPVersion: &*NSString::from_str("HTTP/1.1"),
                headerFields: &*headers
            ];
            let Some(response) = response else {
                fail(task);
                return;
            };
            let data = NSData::with_bytes(&resource.bytes);
            let _: () = msg_send![task, didReceiveResponse: &*response];
            let _: () = msg_send![task, didReceiveData: &*data];
            let _: () = msg_send![task, didFinish];
        }
    }
}

/// # Safety
///
/// `task` must be a live `WKURLSchemeTask` that has not finished, on the main thread.
unsafe fn fail(task: &AnyObject) {
    const NS_URL_ERROR_UNKNOWN: isize = -1;
    unsafe {
        let error: Retained<AnyObject> = msg_send![
            class!(NSError),
            errorWithDomain: &*NSString::from_str("NSURLErrorDomain"),
            code: NS_URL_ERROR_UNKNOWN,
            userInfo: std::ptr::null::<AnyObject>()
        ];
        let _: () = msg_send![task, didFailWithError: &*error];
    }
}

pub struct WebView {
    native_view: NativeView,
    web_view: Retained<AnyObject>,
}

impl WebView {
    /// Creates the view and starts loading the page.
    pub fn new(
        resolve: Resolve,
        events: mpsc::UnboundedSender<WebViewEvent>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Self> {
        let handler = Handler::new(HandlerIvars { resolve, events });
        // SAFETY: These are WebKit classes from the linked framework, and GPUI creates
        // views on the main thread.
        let web_view = unsafe {
            let configuration: Retained<AnyObject> = msg_send![class!(WKWebViewConfiguration), new];
            let _: () = msg_send![
                &*configuration,
                setURLSchemeHandler: &*handler,
                forURLScheme: &*NSString::from_str(SCHEME)
            ];
            let controller: Retained<AnyObject> = msg_send![&*configuration, userContentController];
            let _: () = msg_send![
                &*controller,
                addScriptMessageHandler: &*handler,
                name: &*NSString::from_str(MESSAGE_HANDLER)
            ];

            let web_view: Allocated<AnyObject> = msg_send![class!(WKWebView), alloc];
            let web_view: Retained<AnyObject> = msg_send![
                web_view,
                initWithFrame: NSRect::ZERO,
                configuration: &*configuration
            ];
            // Without this, the web view paints the system's text background where
            // the page has not painted yet, which shows as a sliver while it resizes.
            // The web view keeps its navigation delegate weakly. The configuration and
            // content controller above keep the handler alive.
            let _: () = msg_send![&*web_view, setNavigationDelegate: &*handler];
            let draws_background: Retained<AnyObject> =
                msg_send![class!(NSNumber), numberWithBool: false];
            let _: () = msg_send![
                &*web_view,
                setValue: &*draws_background,
                forKey: &*NSString::from_str("drawsBackground")
            ];
            let can_inspect: bool =
                msg_send![&*web_view, respondsToSelector: objc2::sel!(setInspectable:)];
            if can_inspect {
                let _: () = msg_send![&*web_view, setInspectable: true];
            }

            let url = NSURL::URLWithString(&NSString::from_str(PAGE_URL))
                .context("invalid infoview URL")?;
            let request: Retained<AnyObject> =
                msg_send![class!(NSURLRequest), requestWithURL: &*url];
            let _: Option<Retained<AnyObject>> = msg_send![&*web_view, loadRequest: &*request];
            web_view
        };
        // SAFETY: `web_view` is a live `NSView` that is in no view hierarchy yet.
        let native_view = unsafe { NativeView::new(NonNull::from(&*web_view).cast(), window, cx) }?;
        Ok(Self {
            native_view,
            web_view,
        })
    }

    pub fn native_view(&self) -> &NativeView {
        &self.native_view
    }

    pub fn evaluate(&self, script: &str) {
        // SAFETY: The web view is live, and GPUI runs this on the main thread.
        unsafe {
            let _: () = msg_send![
                &*self.web_view,
                evaluateJavaScript: &*NSString::from_str(script),
                completionHandler: None::<&block2::DynBlock<dyn Fn(*mut AnyObject, *mut AnyObject)>>
            ];
        }
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        // The content controller would otherwise keep the handler, and its sender, alive.
        // SAFETY: The web view is live, and GPUI drops views on the main thread.
        unsafe {
            let configuration: Retained<AnyObject> = msg_send![&*self.web_view, configuration];
            let controller: Retained<AnyObject> = msg_send![&*configuration, userContentController];
            let _: () = msg_send![
                &*controller,
                removeScriptMessageHandlerForName: &*NSString::from_str(MESSAGE_HANDLER)
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_main_frame_on_the_page() {
        let page = "zed-lean-infoview://infoview/index.html";
        let docs = "https://leanprover-community.github.io/mathlib4_docs";
        assert_eq!(decide(Some(page), true, false), Navigation::Allow);
        assert_eq!(
            decide(Some(&format!("{page}#top")), true, true),
            Navigation::Allow
        );
        assert_eq!(
            decide(Some(docs), true, true),
            Navigation::OpenInBrowser(docs.into())
        );
        assert_eq!(
            decide(Some("mailto:someone@example.com"), true, true),
            Navigation::OpenInBrowser("mailto:someone@example.com".into())
        );
        assert_eq!(decide(Some(docs), true, false), Navigation::Cancel);
        assert_eq!(
            decide(
                Some("zed-lean-infoview://infoview/nowhere.html"),
                true,
                true
            ),
            Navigation::Cancel
        );
        assert_eq!(decide(Some("about:blank"), true, false), Navigation::Cancel);
        assert_eq!(decide(None, true, true), Navigation::Cancel);
        assert_eq!(decide(Some(docs), false, false), Navigation::Allow);
        assert_eq!(decide(Some(docs), false, true), Navigation::Allow);
    }
}
