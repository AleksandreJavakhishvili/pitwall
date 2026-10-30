//! The Windows backend of [`super::WebView`]: WebView2 (Edge's engine, part
//! of Windows 10/11), a controller hosted in the GPUI window's HWND (its
//! child window sits above GPUI's drawing: see `third_party/gpui/PATCHES.md`).
//!
//! WebView2 creates its environment and controller asynchronously: the view
//! is returned at once and what it is asked meanwhile waits in
//! [`logic::Pending`] and [`logic::Frame`]. Everything, WebView2's callbacks
//! included, runs on the main (UI, STA) thread: gpui's message loop
//! dispatches them; event handlers hold the view weakly.
//!
//! One environment per data folder: views of the same project share it (and
//! its browser process) while any of them lives.

mod logic;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use gpui::{Bounds, Modifiers, Pixels, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    take_pwstr, AcceleratorKeyPressedEventHandler, CapturePreviewCompletedHandler,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DocumentTitleChangedEventHandler, FocusChangedEventHandler, HistoryChangedEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, SourceChangedEventHandler, TrySuspendCompletedHandler,
};
use windows::core::{w, Interface, BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{IStream, STREAM_SEEK_SET};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetFocus, MapVirtualKeyW, SetFocus, ToUnicode, MAPVK_VK_TO_CHAR,
    VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LWIN, VK_RMENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::Shell::{SHCreateMemStream, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetWindowLongPtrW, IsChild, SetWindowLongPtrW, GWLP_WNDPROC, SW_SHOWNORMAL,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_NCDESTROY, WM_RBUTTONDOWN, WM_XBUTTONDOWN, WNDPROC,
};

use super::{Options, Snapshot, WebEvent, WebView};

pub fn create(window: &mut Window, opts: Options) -> Result<Box<dyn WebView>, String> {
    let parent = match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => HWND(h.hwnd.get() as *mut _),
        _ => return Err("The window has no handle to put the preview in".into()),
    };
    runtime()?;
    std::fs::create_dir_all(&opts.data_dir).map_err(|e| {
        format!("Couldn't create the preview's data folder {}: {e}", opts.data_dir.display())
    })?;
    focus_follows_clicks(parent);
    let shared = Rc::new(Shared {
        parent,
        events: opts.events,
        closed: Cell::new(false),
        engine: RefCell::new(Engine::Creating(logic::Pending::new(opts.url))),
        frame: RefCell::new(logic::Frame::default()),
        nav: RefCell::new(None),
        env: RefCell::new(None),
    });
    let weak = Rc::downgrade(&shared);
    environment(
        &opts.data_dir,
        Box::new(move |env| {
            let Some(shared) = weak.upgrade() else { return };
            match env {
                Ok(env) => shared.create_controller(env),
                Err(e) => shared.fail(&e),
            }
        }),
    );
    Ok(Box::new(View { shared }))
}

/// `Err` when the WebView2 Runtime isn't installed (Windows 10 without
/// Edge's updates, or removed): the caller falls back to the browser.
fn runtime() -> Result<(), String> {
    const MISSING: &str = "The Microsoft Edge WebView2 Runtime isn't installed";
    let mut version = PWSTR::null();
    unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) }
        .map_err(|_| MISSING.to_string())?;
    if take_pwstr(version).is_empty() {
        return Err(MISSING.into());
    }
    Ok(())
}

// ---- Environments, one per data folder ----

struct Environment(ICoreWebView2Environment);

type EnvWaiter = Box<dyn FnOnce(Result<Rc<Environment>, String>)>;

enum Env {
    /// Alive while a view holds it.
    Ready(Weak<Environment>),
    /// Being created: who is waiting for it.
    Creating(Vec<EnvWaiter>),
}

thread_local! {
    static ENVS: RefCell<HashMap<PathBuf, Env>> = RefCell::new(HashMap::new());
}

/// The environment of `dir`, shared, or created; `then` runs when there is
/// one (now, if there already is).
fn environment(dir: &Path, then: EnvWaiter) {
    enum Next {
        Use(Rc<Environment>),
        Wait,
        Create,
    }
    let mut then = Some(then);
    let next = ENVS.with_borrow_mut(|envs| {
        if let Some(Env::Ready(env)) = envs.get(dir) {
            if let Some(env) = env.upgrade() {
                return Next::Use(env);
            }
        }
        match envs.get_mut(dir) {
            Some(Env::Creating(waiters)) => {
                waiters.extend(then.take());
                Next::Wait
            }
            _ => {
                envs.insert(dir.to_path_buf(), Env::Creating(then.take().into_iter().collect()));
                Next::Create
            }
        }
    });
    match next {
        Next::Use(env) => {
            if let Some(then) = then {
                then(Ok(env));
            }
        }
        Next::Wait => {}
        Next::Create => {
            let key = dir.to_path_buf();
            let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |result, env| {
                    environment_created(
                        &key,
                        match (result, env) {
                            (Ok(()), Some(env)) => Ok(env),
                            (Err(e), _) => Err(error_text(&e)),
                            (Ok(()), None) => Err("WebView2 gave no environment".into()),
                        },
                    );
                    Ok(())
                },
            ));
            let folder = HSTRING::from(dir.as_os_str());
            // No options: every view of a folder asks for the same
            // environment (WebView2 refuses a second one that differs).
            let started = unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    &folder,
                    None::<&ICoreWebView2EnvironmentOptions>,
                    &handler,
                )
            };
            if let Err(e) = started {
                environment_created(dir, Err(error_text(&e)));
            }
        }
    }
}

fn environment_created(dir: &Path, result: Result<ICoreWebView2Environment, String>) {
    let result = result.map(|env| Rc::new(Environment(env)));
    let waiters = ENVS.with_borrow_mut(|envs| match &result {
        Ok(env) => envs.insert(dir.to_path_buf(), Env::Ready(Rc::downgrade(env))),
        // The next view tries again.
        Err(_) => envs.remove(dir),
    });
    if let Some(Env::Creating(waiters)) = waiters {
        for waiter in waiters {
            waiter(result.clone());
        }
    }
}

// ---- The view ----

struct Shared {
    /// The GPUI window.
    parent: HWND,
    events: Box<dyn Fn(WebEvent)>,
    /// Being closed: nothing more reaches `events`.
    closed: Cell<bool>,
    engine: RefCell<Engine>,
    frame: RefCell<logic::Frame>,
    /// The main-frame navigation under way: its id and URL.
    nav: RefCell<Option<(u64, String)>>,
    /// Keeps the shared environment alive.
    env: RefCell<Option<Rc<Environment>>>,
}

enum Engine {
    Creating(logic::Pending),
    Ready(Ready),
    Failed,
}

#[derive(Clone)]
struct Ready {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

impl Shared {
    fn emit(&self, event: WebEvent) {
        if !self.closed.get() {
            (self.events)(event);
        }
    }

    fn ready(&self) -> Option<Ready> {
        match &*self.engine.borrow() {
            Engine::Ready(r) => Some(r.clone()),
            _ => None,
        }
    }

    /// Creating failed: say so once, with the URL it was to show.
    fn fail(&self, why: &str) {
        let engine = std::mem::replace(&mut *self.engine.borrow_mut(), Engine::Failed);
        if let Engine::Creating(pending) = engine {
            self.emit(WebEvent::Loading(false));
            self.emit(WebEvent::Failed {
                url: pending.url,
                error: format!("The preview couldn't start: {why}"),
            });
        }
    }

    fn create_controller(self: &Rc<Self>, env: Rc<Environment>) {
        let weak = Rc::downgrade(self);
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                match (weak.upgrade(), result, controller) {
                    (Some(shared), Ok(()), Some(controller)) => shared.attach(controller),
                    // The view went first.
                    (None, Ok(()), Some(controller)) => unsafe {
                        let _ = controller.Close();
                    },
                    (Some(shared), Err(e), _) => shared.fail(&error_text(&e)),
                    (Some(shared), Ok(()), None) => shared.fail("WebView2 gave no controller"),
                    (None, ..) => {}
                }
                Ok(())
            },
        ));
        let started = unsafe { env.0.CreateCoreWebView2Controller(self.parent, &handler) };
        *self.env.borrow_mut() = Some(env);
        if let Err(e) = started {
            self.fail(&error_text(&e));
        }
    }

    /// The controller is up: set it up, then apply what waited.
    fn attach(self: &Rc<Self>, controller: ICoreWebView2Controller) {
        let set_up = unsafe { controller.CoreWebView2() }.and_then(|webview| {
            self.configure(&webview);
            self.subscribe(&controller, &webview)?;
            Ok(webview)
        });
        let webview = match set_up {
            Ok(webview) => webview,
            Err(e) => {
                unsafe {
                    let _ = controller.Close();
                }
                return self.fail(&error_text(&e));
            }
        };
        let ready = Ready {
            controller,
            webview,
        };
        let engine =
            std::mem::replace(&mut *self.engine.borrow_mut(), Engine::Ready(ready.clone()));
        let Engine::Creating(pending) = engine else {
            *self.engine.borrow_mut() = engine;
            unsafe {
                let _ = ready.controller.Close();
            }
            return;
        };
        let (bounds, visible) = {
            let frame = self.frame.borrow();
            (frame.bounds, frame.visible)
        };
        unsafe {
            if let Some(rect) = bounds {
                let _ = ready.controller.SetBounds(win_rect(rect));
            }
            // Hidden but not suspended: the first page loads meanwhile.
            let _ = ready.controller.SetIsVisible(visible);
        }
        self.navigate(&ready, &pending.url);
        if pending.focus {
            unsafe {
                let _ = ready.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
        }
        if pending.inspector {
            unsafe {
                let _ = ready.webview.OpenDevToolsWindow();
            }
        }
    }

    fn configure(&self, webview: &ICoreWebView2) {
        unsafe {
            let Ok(settings) = webview.Settings() else { return };
            // No status bar popping over the page's bottom left corner.
            let _ = settings.SetIsStatusBarEnabled(false);
            let _ = settings.SetAreDevToolsEnabled(true);
            // A dev server's login forms aren't the user's real accounts.
            if let Ok(settings) = settings.cast::<ICoreWebView2Settings4>() {
                let _ = settings.SetIsPasswordAutosaveEnabled(false);
                let _ = settings.SetIsGeneralAutofillEnabled(false);
            }
            // Browser accelerators stay on: every Ctrl shortcut but the
            // editing keys is taken for Pitwall in `key` (so no Ctrl+P
            // print, Ctrl+N window…), leaving F5 reload, F12 DevTools,
            // Alt+arrows history.
        }
    }

    fn subscribe(
        self: &Rc<Self>,
        controller: &ICoreWebView2Controller,
        webview: &ICoreWebView2,
    ) -> windows::core::Result<()> {
        let weak = Rc::downgrade(self);
        let mut token = 0i64;
        unsafe {
            webview.add_NavigationStarting(
                &NavigationStartingEventHandler::create(on(&weak, |s, _, args| {
                    if let Some(args) = args {
                        s.starting(&args);
                    }
                })),
                &mut token,
            )?;
            webview.add_SourceChanged(
                &SourceChangedEventHandler::create(on(&weak, |s, webview: ICoreWebView2, _| {
                    s.emit(WebEvent::Url(source(&webview)));
                })),
                &mut token,
            )?;
            webview.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(on(&weak, |s, webview, args| {
                    if let Some(args) = args {
                        s.completed(&webview, &args);
                    }
                })),
                &mut token,
            )?;
            webview.add_DocumentTitleChanged(
                &DocumentTitleChangedEventHandler::create(on(&weak, |s, webview: ICoreWebView2, _| {
                    s.emit(WebEvent::Title(string(|p| webview.DocumentTitle(p))));
                })),
                &mut token,
            )?;
            webview.add_HistoryChanged(
                &HistoryChangedEventHandler::create(on(&weak, |s, webview: ICoreWebView2, _| {
                    let (mut back, mut forward) = (BOOL(0), BOOL(0));
                    let _ = webview.CanGoBack(&mut back);
                    let _ = webview.CanGoForward(&mut forward);
                    s.emit(WebEvent::CanGo {
                        back: back.as_bool(),
                        forward: forward.as_bool(),
                    });
                })),
                &mut token,
            )?;
            webview.add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(on(&weak, |_, webview, args| {
                    if let Some(args) = args {
                        new_window(&webview, &args);
                    }
                })),
                &mut token,
            )?;
            controller.add_GotFocus(
                &FocusChangedEventHandler::create(on(&weak, |s, _, _| s.emit(WebEvent::Focused))),
                &mut token,
            )?;
            controller.add_AcceleratorKeyPressed(
                &AcceleratorKeyPressedEventHandler::create(on(&weak, |s, _, args| {
                    if let Some(args) = args {
                        s.key(&args);
                    }
                })),
                &mut token,
            )?;
        }
        Ok(())
    }

    /// A main-frame navigation is starting: only local dev servers stay in
    /// the preview, the rest goes to the browser.
    fn starting(&self, args: &ICoreWebView2NavigationStartingEventArgs) {
        let uri = string(|p| unsafe { args.Uri(p) });
        if !super::allowed(&uri) {
            unsafe {
                let _ = args.SetCancel(true);
            }
            if logic::browsable(&uri) {
                open_in_browser(&uri);
            }
            return;
        }
        let mut id = 0u64;
        unsafe {
            let _ = args.NavigationId(&mut id);
        }
        *self.nav.borrow_mut() = Some((id, uri));
        self.emit(WebEvent::Loading(true));
    }

    fn completed(&self, webview: &ICoreWebView2, args: &ICoreWebView2NavigationCompletedEventArgs) {
        let (mut ok, mut status, mut id) = (BOOL(0), COREWEBVIEW2_WEB_ERROR_STATUS(0), 0u64);
        unsafe {
            let _ = args.IsSuccess(&mut ok);
            let _ = args.WebErrorStatus(&mut status);
            let _ = args.NavigationId(&mut id);
        }
        let url = {
            let mut nav = self.nav.borrow_mut();
            match nav.take() {
                Some((current, url)) if current == id => Some(url),
                // A newer navigation replaced this one and is still loading.
                Some(newer) => {
                    *nav = Some(newer);
                    return;
                }
                None => None,
            }
        };
        self.emit(WebEvent::Loading(false));
        // Cancelled (by `starting`, a newer navigation, window.stop()) isn't
        // a failure; nor is an HTTP error: the server answered, its page shows.
        if ok.as_bool() || status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
            return;
        }
        let http = args
            .cast::<ICoreWebView2NavigationCompletedEventArgs2>()
            .ok()
            .and_then(|args| {
                let mut code = 0;
                unsafe { args.HttpStatusCode(&mut code) }.ok()?;
                Some(code)
            })
            .unwrap_or(0);
        if http > 0 {
            return;
        }
        self.emit(WebEvent::Failed {
            url: url.unwrap_or_else(|| source(webview)),
            error: web_error(status).into(),
        });
    }

    /// A key while the page has focus: Pitwall's shortcuts go to Pitwall.
    fn key(&self, args: &ICoreWebView2AcceleratorKeyPressedEventArgs) {
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut vk = 0u32;
        let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
        unsafe {
            if args.KeyEventKind(&mut kind).is_err() || args.VirtualKey(&mut vk).is_err() {
                return;
            }
            let _ = args.PhysicalKeyStatus(&mut status);
        }
        let vk = vk as u16;
        let Some(keystroke) =
            logic::shortcut(vk, modifiers(), |shifted| layout_key(vk, status.ScanCode, shifted))
        else {
            return;
        };
        // Up as well as down, so the page sees neither.
        unsafe {
            let _ = args.SetHandled(true);
        }
        if kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
            || kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
        {
            self.emit(WebEvent::Shortcut(keystroke));
        }
    }

    fn navigate(&self, ready: &Ready, url: &str) {
        if unsafe { ready.webview.Navigate(&HSTRING::from(url)) }.is_err() {
            self.emit(WebEvent::Failed {
                url: url.into(),
                error: "Not an address the preview can open".into(),
            });
        }
    }

    /// Shown, or hidden and suspended (timers, animations and rendering
    /// stop; WebView2 resumes it when shown).
    fn show(&self, ready: &Ready, visible: bool) {
        let suspendable = ready.webview.cast::<ICoreWebView2_3>().ok();
        unsafe {
            if visible {
                if let Some(w) = &suspendable {
                    let _ = w.Resume();
                }
                let _ = ready.controller.SetIsVisible(true);
            } else {
                self.release_focus();
                let _ = ready.controller.SetIsVisible(false);
                if let Some(w) = &suspendable {
                    let done = TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
                    let _ = w.TrySuspend(&done);
                }
            }
        }
    }

    /// If the page has keyboard focus, back to the GPUI window: a hidden or
    /// closed child would take it with it.
    fn release_focus(&self) {
        unsafe {
            let focus = GetFocus();
            if !focus.is_invalid() && IsChild(self.parent, focus).as_bool() {
                let _ = SetFocus(Some(self.parent));
            }
        }
    }
}

/// An event handler that reaches the view while it lives.
#[allow(clippy::type_complexity)]
fn on<S: 'static, A: 'static>(
    weak: &Weak<Shared>,
    f: impl Fn(&Shared, S, Option<A>) + 'static,
) -> Box<dyn FnMut(Option<S>, Option<A>) -> windows::core::Result<()>> {
    let weak = weak.clone();
    Box::new(move |sender, args| {
        if let (Some(shared), Some(sender)) = (weak.upgrade(), sender) {
            if !shared.closed.get() {
                f(&shared, sender, args);
            }
        }
        Ok(())
    })
}

/// `window.open` and target=_blank links: local pages in this view, others
/// in the browser; never a new window.
fn new_window(webview: &ICoreWebView2, args: &ICoreWebView2NewWindowRequestedEventArgs) {
    let uri = string(|p| unsafe { args.Uri(p) });
    unsafe {
        let _ = args.SetHandled(true);
        if super::allowed(&uri) {
            let _ = webview.Navigate(&HSTRING::from(uri.as_str()));
            return;
        }
    }
    if logic::browsable(&uri) {
        open_in_browser(&uri);
    }
}

pub struct View {
    shared: Rc<Shared>,
}

impl View {
    /// `pending` while WebView2 is still creating the view, `ready` once it
    /// is (no borrow held while WebView2 runs: its events can come back in).
    fn act(&self, pending: impl FnOnce(&mut logic::Pending), ready: impl FnOnce(&Ready)) {
        let r = match &mut *self.shared.engine.borrow_mut() {
            Engine::Creating(p) => {
                pending(p);
                None
            }
            Engine::Ready(r) => Some(r.clone()),
            Engine::Failed => None,
        };
        if let Some(r) = r {
            ready(&r);
        }
    }
}

impl WebView for View {
    fn navigate(&mut self, url: &str) {
        let shared = self.shared.clone();
        self.act(|p| p.navigate(url), |r| shared.navigate(r, url));
    }

    fn reload(&mut self) {
        self.act(|p| p.history(), |r| unsafe {
            let _ = r.webview.Reload();
        });
    }

    fn back(&mut self) {
        self.act(|p| p.history(), |r| unsafe {
            let _ = r.webview.GoBack();
        });
    }

    fn forward(&mut self) {
        self.act(|p| p.history(), |r| unsafe {
            let _ = r.webview.GoForward();
        });
    }

    fn set_bounds(&mut self, bounds: Bounds<Pixels>) {
        let rect = logic::physical(bounds, scale(self.shared.parent));
        if !self.shared.frame.borrow_mut().set_bounds(rect) {
            return;
        }
        if let Some(r) = self.shared.ready() {
            unsafe {
                let _ = r.controller.SetBounds(win_rect(rect));
                // Popups (a <select>'s list) and IME place themselves from it.
                let _ = r.controller.NotifyParentWindowPositionChanged();
            }
        }
    }

    fn set_visible(&mut self, visible: bool) {
        if !self.shared.frame.borrow_mut().set_visible(visible) {
            return;
        }
        if let Some(r) = self.shared.ready() {
            self.shared.show(&r, visible);
        }
    }

    fn focus(&mut self) {
        self.act(|p| p.focus(true), |r| unsafe {
            let _ = r.controller.NotifyParentWindowPositionChanged();
            let _ = r.controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        });
    }

    fn blur(&mut self) {
        self.act(|p| p.focus(false), |_| ());
        self.shared.release_focus();
    }

    fn open_inspector(&mut self) {
        self.act(|p| p.open_inspector(), |r| unsafe {
            let _ = r.webview.OpenDevToolsWindow();
        });
    }

    fn snapshot(&mut self, done: Box<dyn FnOnce(Option<Snapshot>) + 'static>) {
        let Some(r) = self.shared.ready() else {
            return done(None);
        };
        let Some(stream) = (unsafe { SHCreateMemStream(None) }) else {
            return done(None);
        };
        let done = Rc::new(Cell::new(Some(done)));
        let finish = {
            let done = done.clone();
            let stream = stream.clone();
            CapturePreviewCompletedHandler::create(Box::new(move |result| {
                if let Some(done) = done.take() {
                    done(result.ok().and_then(|()| decode(&stream)));
                }
                Ok(())
            }))
        };
        let started = unsafe {
            r.webview
                .CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &stream, &finish)
        };
        if started.is_err() {
            if let Some(done) = done.take() {
                done(None);
            }
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        // Closing can raise events (focus, a cancelled load): not to the UI,
        // which is dropping this view.
        self.shared.closed.set(true);
        let engine = std::mem::replace(&mut *self.shared.engine.borrow_mut(), Engine::Failed);
        if let Engine::Ready(r) = engine {
            self.shared.release_focus();
            unsafe {
                let _ = r.controller.Close();
            }
        }
    }
}

// ---- Win32 helpers ----

fn win_rect(r: logic::Rect) -> RECT {
    RECT {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    }
}

/// GPUI's scale factor on Windows: the window's DPI / 96 (it follows
/// WM_DPICHANGED the same way).
fn scale(hwnd: HWND) -> f32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 1.,
        dpi => dpi as f32 / 96.,
    }
}

/// A string WebView2 hands over (CoTaskMem, freed here); empty on error.
fn string(get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> String {
    let mut p = PWSTR::null();
    if get(&mut p).is_err() {
        return String::new();
    }
    take_pwstr(p)
}

fn source(webview: &ICoreWebView2) -> String {
    string(|p| unsafe { webview.Source(p) })
}

fn error_text(e: &windows::core::Error) -> String {
    let message = e.message();
    let message = message.trim();
    if message.is_empty() {
        format!("WebView2 error {:#010x}", e.code().0)
    } else {
        message.to_string()
    }
}

/// Why the main frame didn't load, for the pane.
fn web_error(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> &'static str {
    match status {
        COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT => "Couldn't connect: is the server running?",
        COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => "The server is unreachable",
        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => "The server took too long to answer",
        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => "The host name couldn't be found",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_ABORTED => "The connection was aborted",
        COREWEBVIEW2_WEB_ERROR_STATUS_CONNECTION_RESET => "The connection was reset",
        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED => "The network is disconnected",
        COREWEBVIEW2_WEB_ERROR_STATUS_ERROR_HTTP_INVALID_SERVER_RESPONSE => {
            "The server's answer isn't valid HTTP"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_REDIRECT_FAILED => "A redirect failed",
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
        | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID => {
            "The server's certificate isn't trusted"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_AUTHENTICATION_CREDENTIALS_REQUIRED => {
            "The server asks for credentials"
        }
        COREWEBVIEW2_WEB_ERROR_STATUS_VALID_PROXY_AUTHENTICATION_REQUIRED => {
            "The proxy asks for credentials"
        }
        _ => "The page couldn't be loaded",
    }
}

/// The system browser (only http(s): see [`logic::browsable`]).
fn open_in_browser(url: &str) {
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// The modifiers held now, as GPUI's Windows platform reads them
/// (`current_modifiers`): right Alt with left Ctrl is AltGr, typing, not
/// Ctrl+Alt. Asynchronous state: the keys went to WebView2's window.
fn modifiers() -> Modifiers {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0;
    let altgr = down(VK_RMENU) && down(VK_LCONTROL);
    Modifiers {
        control: down(VK_CONTROL) && !altgr,
        alt: (down(VK_LMENU) || down(VK_RMENU)) && !altgr,
        shift: down(VK_SHIFT),
        platform: down(VK_LWIN) || down(VK_RWIN),
        function: false,
    }
}

/// The current layout's character for a key, as GPUI's `get_key_from_vkey`
/// (unshifted) and `get_shifted_key` name it.
fn layout_key(vk: u16, scan_code: u32, shifted: bool) -> Option<String> {
    if !shifted {
        let ch = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) } & 0xFFFF;
        return char::from_u32(ch).filter(|&c| c != '\0').map(|c| c.to_string());
    }
    let mut state = [0u8; 256];
    state[VK_SHIFT.0 as usize] = 0x80;
    let mut buffer = [0u16; 8];
    // Flag 4: leave the keyboard's dead-key state alone.
    let len = unsafe { ToUnicode(vk as u32, scan_code, Some(&state), &mut buffer, 1 << 2) };
    let text = String::from_utf16(&buffer[..len.unsigned_abs() as usize]).ok()?;
    let first = text.chars().next()?;
    (len < 0 || !first.is_control()).then_some(text)
}

/// The PNG `CapturePreview` wrote, as RGBA8.
fn decode(stream: &IStream) -> Option<Snapshot> {
    let mut png = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    unsafe {
        stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
        loop {
            let mut read = 0u32;
            stream
                .Read(buffer.as_mut_ptr().cast(), buffer.len() as u32, Some(&mut read))
                .ok()
                .ok()?;
            if read == 0 {
                break;
            }
            png.extend_from_slice(&buffer[..read as usize]);
        }
    }
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Some(Snapshot {
        width,
        height,
        rgba: image.into_raw(),
    })
}

// ---- Clicks into GPUI take keyboard focus back from the page ----

thread_local! {
    /// GPUI's window procedure of each window we subclassed.
    static GPUI_PROCS: RefCell<HashMap<isize, WNDPROC>> = RefCell::new(HashMap::new());
}

/// A click into a child window takes keyboard focus, but a click back on its
/// parent doesn't give it back: after a click in the page, typing in a GPUI
/// pane would still go to the page. So the GPUI window takes focus on a
/// mouse button press in its client area (classic subclassing, before
/// GPUI's own window procedure sees the message). Once per window, for its
/// lifetime.
fn focus_follows_clicks(hwnd: HWND) {
    let key = hwnd.0 as isize;
    if GPUI_PROCS.with_borrow(|p| p.contains_key(&key)) {
        return;
    }
    let gpui = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
    if gpui == 0 {
        return;
    }
    // Known before ours is in, so every message finds it.
    let gpui = unsafe { std::mem::transmute::<isize, WNDPROC>(gpui) };
    GPUI_PROCS.with_borrow_mut(|p| p.insert(key, gpui));
    unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, take_focus_on_click as usize as isize) };
}

unsafe extern "system" fn take_focus_on_click(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN)
        && unsafe { GetFocus() } != hwnd
    {
        let _ = unsafe { SetFocus(Some(hwnd)) };
    }
    let key = hwnd.0 as isize;
    let previous = GPUI_PROCS.with_borrow(|p| p.get(&key).copied().flatten());
    let result = unsafe { CallWindowProcW(previous, hwnd, msg, wparam, lparam) };
    if msg == WM_NCDESTROY {
        // The handle can be reused by a new window.
        GPUI_PROCS.with_borrow_mut(|p| p.remove(&key));
    }
    result
}
