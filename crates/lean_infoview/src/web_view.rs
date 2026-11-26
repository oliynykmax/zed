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

pub struct Resource {
    pub mime_type: Cow<'static, str>,
    pub bytes: Cow<'static, [u8]>,
}

/// Serves the page's files by URL path, such as `/index.html`.
pub type Resolve = Box<dyn Fn(&str) -> Option<Resource>>;

struct HandlerIvars {
    resolve: Resolve,
    messages: mpsc::UnboundedSender<String>,
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
            self.ivars().messages.unbounded_send(body.to_string()).ok();
        }
    }
);

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
                return;
            };
            let data = NSData::with_bytes(&resource.bytes);
            let _: () = msg_send![task, didReceiveResponse: &*response];
            let _: () = msg_send![task, didReceiveData: &*data];
            let _: () = msg_send![task, didFinish];
        }
    }
}

pub struct WebView {
    native_view: NativeView,
    web_view: Retained<AnyObject>,
}

impl WebView {
    /// Creates the view and starts loading the page. Every string the page posts to
    /// `window.webkit.messageHandlers.zed` arrives on `messages`.
    pub fn new(
        resolve: Resolve,
        messages: mpsc::UnboundedSender<String>,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Self> {
        let handler = Handler::new(HandlerIvars { resolve, messages });
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
