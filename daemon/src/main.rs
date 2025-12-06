#![allow(unexpected_cfgs)] // objc macros reference cfg(cargo-clippy) internally

mod accessibility;
mod api;
mod interval;
mod logging;
mod mouse_tracker;
mod workspace_tracker;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::env;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use cacao::appkit::menu::{Menu, MenuItem};
use cacao::appkit::{App, AppDelegate};
use cacao::foundation::{NO, NSString, YES, id};
use cacao::image::{Image, SFSymbol};
use cacao::notification_center::Dispatcher;
use chrono::{Local, Utc};
use log::{error, info, warn};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use objc_id::ShareId;
use png::{BitDepth, ColorType, Encoder, EncodingError};
use screencapturekit::error::SCError;
use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use screencapturekit::screenshot_manager::SCScreenshotManager;
use serde::Deserialize;

use crate::accessibility::ActiveWindowInfo;
use crate::api::{ActivityEntry, ActivityEvent, ApiClient, ApiError, ImageFormat, VideoFormat};
use crate::interval::current_interval_id;
use crate::mouse_tracker::MouseTracker;
use crate::workspace_tracker::WorkspaceTracker;

const NS_VARIABLE_STATUS_ITEM_LENGTH: f64 = -1.0;
const API_BASE_ENV: &str = "CLEO_CAPTURE_API_URL";
const DEFAULT_API_BASE: &str = "http://localhost:3000";
const SCREENSHOT_INTERVAL_SECS: u64 = 5;
const BURST_WINDOW_SECS: u64 = 5;
const BURST_THRESHOLD: usize = 5;
const AUTO_RECORDING_TAIL_SECS: u64 = 5;
const TASK_SLEEP_CHUNK_MS: u64 = 100;

#[derive(Debug, Deserialize)]
struct CleoConfig {
    api_token: String,
}

#[derive(Default)]
struct MenuBarApp {
    status_item: RefCell<Option<StatusItem>>,
    menu_handles: RefCell<Option<MenuHandles>>,
    recorder: RefCell<Option<ScreenRecorder>>,
    daemon: RefCell<Option<LoggingDaemon>>,
    api: RefCell<Option<ApiClient>>,
    tracker: RefCell<Option<WorkspaceTracker>>,
    mouse_tracker: RefCell<Option<MouseTracker>>,
    screenshot_task: RefCell<Option<RepeatingTask>>,
    auto_stop_task: RefCell<Option<DelayedTask>>,
    activity_window: RefCell<VecDeque<Instant>>,
    manual_recording: Cell<bool>,
    activity_events: RefCell<Vec<ActivityEntry>>,
}

#[derive(Copy, Clone)]
enum AppMessage {
    ToggleRecording,
    TakeScreenshot,
    MouseClick,
    AutoStopRecording,
}

impl Dispatcher for MenuBarApp {
    type Message = AppMessage;

    fn on_ui_message(&self, message: Self::Message) {
        match message {
            AppMessage::ToggleRecording => {
                if self.recorder.borrow().is_some() {
                    self.manual_recording.set(false);
                    self.stop_recording();
                } else {
                    self.start_recording();
                    if self.recorder.borrow().is_some() {
                        self.manual_recording.set(true);
                        self.cancel_auto_stop();
                    }
                }
            }
            AppMessage::TakeScreenshot => self.take_screenshot(),
            AppMessage::MouseClick => self.record_mouse_click(),
            AppMessage::AutoStopRecording => self.stop_recording_if_auto(),
        }
    }
}

impl AppDelegate for MenuBarApp {
    fn did_finish_launching(&self) {
        info!("Launching Cleo Screen Capture demo");
        set_accessory_activation_policy();

        let (menu, handles) = build_status_menu();
        handles.set_recording(false);
        self.menu_handles.replace(Some(handles));

        let icon = Image::symbol(SFSymbol::MessageFill, "Screen capture menu bar icon");
        let status_item = StatusItem::new(icon, menu);
        self.status_item.replace(Some(status_item));

        self.daemon.replace(Some(LoggingDaemon::start()));
        self.ensure_api_client();
        self.start_activity_tracking();
        self.start_mouse_tracking();
        self.start_screenshot_timer();
    }

    fn will_terminate(&self) {
        self.stop_recording();
        self.stop_daemon();
        self.stop_tracker();
        self.stop_mouse_tracking();
        self.stop_screenshot_timer();
    }
}

impl MenuBarApp {
    fn start_recording(&self) {
        if self.recorder.borrow().is_some() {
            warn!("Recording already in progress");
            return;
        }

        match self.api_client().and_then(ScreenRecorder::start) {
            Ok(recorder) => {
                info!(
                    "Recording started, spooling to {}",
                    recorder.file_path().display()
                );
                self.update_menu_state(true);
                self.recorder.replace(Some(recorder));
            }
            Err(err) => error!("Failed to start recording: {err}"),
        }
    }

    fn stop_recording(&self) {
        self.manual_recording.set(false);
        if let Some(recorder) = self.recorder.borrow_mut().take() {
            match recorder.stop() {
                Ok(()) => info!("Recording uploaded successfully"),
                Err(err) => error!("Failed to stop recording: {err}"),
            }
        }
        self.update_menu_state(false);
        self.cancel_auto_stop();
    }

    fn take_screenshot(&self) {
        if self.recorder.borrow().is_some() {
            info!("Skipping screenshot while recording");
            return;
        }
        let result = self.api_client().and_then(|api| capture_screenshot(&api));
        if let Err(err) = result {
            error!("Failed to capture screenshot: {err}");
        } else {
            info!("Screenshot uploaded successfully");
        }
    }

    fn update_menu_state(&self, recording: bool) {
        if let Some(handles) = self.menu_handles.borrow().as_ref() {
            handles.set_recording(recording);
        }
    }

    fn stop_daemon(&self) {
        if let Some(daemon) = self.daemon.borrow_mut().take() {
            daemon.shutdown();
        }
    }

    fn start_activity_tracking(&self) {
        let this = self as *const MenuBarApp;
        let handler = move |info: ActiveWindowInfo| unsafe {
            if let Some(app) = this.as_ref() {
                app.record_focus_event(info);
            }
        };

        match WorkspaceTracker::start(handler) {
            Ok(tracker) => {
                info!("Workspace tracker started");
                self.tracker.replace(Some(tracker));
            }
            Err(err) => error!("Workspace tracker unavailable: {err}"),
        }
    }

    fn stop_tracker(&self) {
        self.tracker.borrow_mut().take();
    }

    fn record_focus_event(&self, info: ActiveWindowInfo) {
        let event = ActivityEvent::foreground_switch(info.app_name, info.window_title);
        let interval_id = current_interval_id();
        let entry = ActivityEntry::new(Utc::now(), interval_id, event);
        self.activity_events.borrow_mut().push(entry);
        self.handle_activity_event();
    }

    fn start_mouse_tracking(&self) {
        if self.mouse_tracker.borrow().is_some() {
            return;
        }
        let handler = || App::<MenuBarApp, AppMessage>::dispatch_main(AppMessage::MouseClick);
        match MouseTracker::start(handler) {
            Ok(tracker) => {
                info!("Mouse tracker started");
                self.mouse_tracker.replace(Some(tracker));
            }
            Err(err) => error!("Mouse tracker unavailable: {err}"),
        }
    }

    fn stop_mouse_tracking(&self) {
        self.mouse_tracker.borrow_mut().take();
    }

    fn record_mouse_click(&self) {
        info!("Mouse click recorded");
        let event = ActivityEvent::mouse_click();
        let interval_id = current_interval_id();
        let entry = ActivityEntry::new(Utc::now(), interval_id, event);
        self.activity_events.borrow_mut().push(entry);
        self.handle_activity_event();
    }

    fn ensure_api_client(&self) {
        if self.api.borrow().is_some() {
            return;
        }
        match build_api_client() {
            Ok(client) => {
                info!("Using capture API at {}", client.base_url());
                self.api.replace(Some(client));
            }
            Err(err) => error!("Capture API unavailable: {err}"),
        }
    }

    fn api_client(&self) -> Result<ApiClient, CaptureError> {
        if self.api.borrow().is_none() {
            self.ensure_api_client();
        }
        self.api
            .borrow()
            .clone()
            .ok_or(CaptureError::ApiUnavailable)
    }

    fn start_screenshot_timer(&self) {
        if self.screenshot_task.borrow().is_some() {
            return;
        }
        let task = RepeatingTask::start(Duration::from_secs(SCREENSHOT_INTERVAL_SECS), || {
            App::<MenuBarApp, AppMessage>::dispatch_main(AppMessage::TakeScreenshot);
        });
        self.screenshot_task.replace(Some(task));
    }

    fn stop_screenshot_timer(&self) {
        self.screenshot_task.borrow_mut().take();
    }

    fn handle_activity_event(&self) {
        self.track_activity_burst();
        self.schedule_auto_stop();
    }

    fn track_activity_burst(&self) {
        let mut window = self.activity_window.borrow_mut();
        let now = Instant::now();
        window.push_back(now);
        let threshold = Duration::from_secs(BURST_WINDOW_SECS);
        while let Some(front) = window.front() {
            if now.duration_since(*front) > threshold {
                window.pop_front();
            } else {
                break;
            }
        }
        if window.len() >= BURST_THRESHOLD && self.recorder.borrow().is_none() {
            info!("Automatic recording triggered by activity burst");
            self.start_recording();
        }
    }

    fn schedule_auto_stop(&self) {
        if self.manual_recording.get() || self.recorder.borrow().is_none() {
            self.cancel_auto_stop();
            return;
        }
        let mut slot = self.auto_stop_task.borrow_mut();
        let had_task = slot.is_some();
        if let Some(task) = slot.take() {
            task.cancel();
        }
        if !had_task {
            info!(
                "Automatic recording will stop in {} seconds without more activity",
                AUTO_RECORDING_TAIL_SECS
            );
        }
        let task = DelayedTask::schedule(Duration::from_secs(AUTO_RECORDING_TAIL_SECS), || {
            App::<MenuBarApp, AppMessage>::dispatch_main(AppMessage::AutoStopRecording);
        });
        slot.replace(task);
    }

    fn cancel_auto_stop(&self) {
        if let Some(task) = self.auto_stop_task.borrow_mut().take() {
            task.cancel();
        }
    }

    fn stop_recording_if_auto(&self) {
        if self.manual_recording.get() {
            return;
        }
        if self.recorder.borrow().is_some() {
            info!("Stopping automatic recording after inactivity");
            self.stop_recording();
        }
    }
}

fn main() {
    logging::init();
    App::new("com.cleo.cleo", MenuBarApp::default()).run();
}

fn build_status_menu() -> (Menu, MenuHandles) {
    let hello = disabled_label("Hello, world!");
    let (record_handle, record_item) = action_item("Start Recording", AppMessage::ToggleRecording);
    let screenshot_item = action_item("Take Screenshot", AppMessage::TakeScreenshot).1;

    let menu = Menu::new(
        "",
        vec![
            hello,
            record_item,
            screenshot_item,
            MenuItem::Separator,
            quit_item(),
        ],
    );

    (menu, MenuHandles::new(record_handle))
}

fn build_api_client() -> Result<ApiClient, CaptureError> {
    let base = env::var(API_BASE_ENV).unwrap_or_else(|_| DEFAULT_API_BASE.to_string());
    let auth_token = load_api_token()?;
    ApiClient::new(base, Some(auth_token)).map_err(CaptureError::from)
}

fn load_api_token() -> Result<String, CaptureError> {
    let path = cleo_config_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Err(CaptureError::Config(format!(
                "Missing Cleo config at {}. Create the file with an `api_token` field.",
                path.display()
            )));
        }
        Err(err) => return Err(CaptureError::from(err)),
    };

    let config: CleoConfig = serde_json::from_str(&contents).map_err(|err| {
        CaptureError::Config(format!(
            "Failed to parse Cleo config {}: {err}",
            path.display()
        ))
    })?;

    let token = config.api_token.trim().to_owned();
    if token.is_empty() {
        return Err(CaptureError::Config(format!(
            "Config {} must define a non-empty `api_token` value",
            path.display()
        )));
    }

    Ok(token)
}

fn cleo_config_path() -> Result<PathBuf, CaptureError> {
    let home = env::var("HOME").map_err(|_| {
        CaptureError::Config(
            "HOME environment variable must be set to locate ~/.config/cleo.json".into(),
        )
    })?;
    let mut path = PathBuf::from(home);
    path.push(".config");
    path.push("cleo.json");
    Ok(path)
}

fn action_item(title: &str, message: AppMessage) -> (MenuItemHandle, MenuItem) {
    let item = MenuItem::new(title).action(move || {
        App::<MenuBarApp, AppMessage>::dispatch_main(message);
    });

    match item {
        MenuItem::Custom(objc) => {
            let shared = unsafe {
                let ptr = (&*objc) as *const Object as *mut Object;
                ShareId::from_ptr(ptr)
            };
            (MenuItemHandle { item: shared }, MenuItem::Custom(objc))
        }
        _ => unreachable!("Custom menu item expected"),
    }
}

fn disabled_label<S: AsRef<str>>(text: S) -> MenuItem {
    match MenuItem::new(text.as_ref()) {
        MenuItem::Custom(objc) => {
            unsafe {
                let _: () = msg_send![&*objc, setEnabled: NO];
            }
            MenuItem::Custom(objc)
        }
        other => other,
    }
}

fn quit_item() -> MenuItem {
    MenuItem::new("Quit Cleo Recorder").action(|| App::terminate())
}

fn set_accessory_activation_policy() {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        // 1 == NSApplicationActivationPolicyAccessory
        let _: () = msg_send![app, setActivationPolicy: 1isize];
    }
}

struct MenuHandles {
    recording: MenuItemHandle,
}

impl MenuHandles {
    fn new(recording: MenuItemHandle) -> Self {
        Self { recording }
    }

    fn set_recording(&self, recording: bool) {
        let title = if recording {
            "Stop Recording"
        } else {
            "Start Recording"
        };
        self.recording.set_title(title);
        self.recording.set_enabled(true);
    }
}

struct MenuItemHandle {
    item: ShareId<Object>,
}

impl MenuItemHandle {
    fn set_enabled(&self, enabled: bool) {
        unsafe {
            let flag = if enabled { YES } else { NO };
            let _: () = msg_send![&*self.item, setEnabled: flag];
        }
    }

    fn set_title(&self, title: &str) {
        unsafe {
            let text = NSString::new(title);
            let _: () = msg_send![&*self.item, setTitle:&*text];
        }
    }
}

struct StatusItem {
    item: ShareId<Object>,
    _menu: Menu,
    _icon: Image,
}

impl StatusItem {
    fn new(icon: Image, menu: Menu) -> Self {
        unsafe {
            let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
            let item: id =
                msg_send![status_bar, statusItemWithLength: NS_VARIABLE_STATUS_ITEM_LENGTH];
            let button: id = msg_send![item, button];

            let tooltip = NSString::new("Cleo Screen Recorder");
            let _: () = msg_send![&*icon.0, setTemplate: YES];
            let _: () = msg_send![button, setToolTip:&*tooltip];
            let _: () = msg_send![button, setImage:&*icon.0];
            let _: () = msg_send![item, setMenu:&*menu.0];

            Self {
                item: ShareId::from_ptr(item),
                _menu: menu,
                _icon: icon,
            }
        }
    }
}

impl Drop for StatusItem {
    fn drop(&mut self) {
        unsafe {
            let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
            let _: () = msg_send![status_bar, removeStatusItem:&*self.item];
        }
    }
}

struct LoggingDaemon {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LoggingDaemon {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !thread_flag.load(Ordering::Relaxed) {
                info!(
                    target: "daemon",
                    "heartbeat {}",
                    Local::now().format("%H:%M:%S")
                );
                thread::sleep(Duration::from_secs(1));
            }
            info!(target: "daemon", "stopping");
        });

        LoggingDaemon {
            stop,
            handle: Some(handle),
        }
    }

    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LoggingDaemon {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct RepeatingTask {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl RepeatingTask {
    fn start<F>(interval: Duration, mut action: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            loop {
                if sleep_with_cancellation(&flag, interval) {
                    break;
                }
                if flag.load(Ordering::Relaxed) {
                    break;
                }
                action();
                if flag.load(Ordering::Relaxed) {
                    break;
                }
            }
        });

        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for RepeatingTask {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct DelayedTask {
    cancel_flag: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DelayedTask {
    fn schedule<F>(delay: Duration, action: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let flag = Arc::new(AtomicBool::new(false));
        let wait_flag = Arc::clone(&flag);
        let handle = thread::spawn(move || {
            if sleep_with_cancellation(&wait_flag, delay) {
                return;
            }
            action();
        });

        Self {
            cancel_flag: flag,
            handle: Some(handle),
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for DelayedTask {
    fn drop(&mut self) {
        self.cancel();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn sleep_with_cancellation(flag: &AtomicBool, duration: Duration) -> bool {
    let mut elapsed = Duration::ZERO;
    while elapsed < duration {
        if flag.load(Ordering::Relaxed) {
            return true;
        }
        let remaining = duration - elapsed;
        let step = if remaining > Duration::from_millis(TASK_SLEEP_CHUNK_MS) {
            Duration::from_millis(TASK_SLEEP_CHUNK_MS)
        } else {
            remaining
        };
        thread::sleep(step);
        elapsed += step;
    }
    flag.load(Ordering::Relaxed)
}

struct ScreenRecorder {
    stream: SCStream,
    recording_output: SCRecordingOutput,
    file_path: PathBuf,
    running: bool,
    api: ApiClient,
}

impl ScreenRecorder {
    fn start(api: ApiClient) -> Result<Self, CaptureError> {
        let content = SCShareableContent::get().map_err(CaptureError::from)?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or(CaptureError::NoDisplay)?;

        let filter = SCContentFilter::builder()
            .display(&display)
            .exclude_windows(&[])
            .build();

        let config = SCStreamConfiguration::new()
            .with_width(display.width())
            .with_height(display.height())
            .with_shows_cursor(true)
            .with_fps(30);

        let stream = SCStream::new(&filter, &config);
        let file_path = recording_file_path();

        let recording_config = SCRecordingOutputConfiguration::new()
            .with_output_url(&file_path)
            .with_video_codec(SCRecordingOutputCodec::H264)
            .with_output_file_type(SCRecordingOutputFileType::MOV);

        let recording_output =
            SCRecordingOutput::new(&recording_config).ok_or(CaptureError::RecordingUnavailable)?;

        stream
            .add_recording_output(&recording_output)
            .map_err(CaptureError::from)?;
        stream.start_capture().map_err(CaptureError::from)?;

        Ok(Self {
            stream,
            recording_output,
            file_path,
            running: true,
            api,
        })
    }

    fn stop(mut self) -> Result<(), CaptureError> {
        self.stop_stream()?;
        let bytes = fs::read(&self.file_path)?;
        self.api
            .upload_video(bytes, VideoFormat::QuickTime)
            .map_err(CaptureError::from)?;
        let _ = fs::remove_file(&self.file_path);
        Ok(())
    }

    fn file_path(&self) -> &Path {
        &self.file_path
    }

    fn stop_stream(&mut self) -> Result<(), CaptureError> {
        if self.running {
            self.stream.stop_capture().map_err(CaptureError::from)?;
            self.stream
                .remove_recording_output(&self.recording_output)
                .map_err(CaptureError::from)?;
            self.running = false;
        }
        Ok(())
    }
}

impl Drop for ScreenRecorder {
    fn drop(&mut self) {
        if let Err(err) = self.stop_stream() {
            error!("Failed to stop stream during drop: {err}");
        }
        let _ = fs::remove_file(&self.file_path);
    }
}

#[derive(Debug)]
enum CaptureError {
    NoDisplay,
    RecordingUnavailable,
    Io(std::io::Error),
    ScreenKit(SCError),
    Api(ApiError),
    ApiUnavailable,
    ImageEncoding(String),
    Config(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::NoDisplay => write!(f, "No displays available for capture"),
            CaptureError::RecordingUnavailable => {
                write!(f, "Recording output is unavailable on this system")
            }
            CaptureError::Io(err) => write!(f, "Filesystem error: {err}"),
            CaptureError::ScreenKit(err) => write!(f, "ScreenCaptureKit error: {err}"),
            CaptureError::Api(err) => write!(f, "Capture API error: {err}"),
            CaptureError::ApiUnavailable => {
                write!(
                    f,
                    "Capture API client is unavailable. Set {} to point to the server.",
                    API_BASE_ENV
                )
            }
            CaptureError::ImageEncoding(err) => write!(f, "Failed to encode image: {err}"),
            CaptureError::Config(err) => write!(f, "{err}"),
        }
    }
}

impl From<SCError> for CaptureError {
    fn from(value: SCError) -> Self {
        CaptureError::ScreenKit(value)
    }
}

impl From<std::io::Error> for CaptureError {
    fn from(value: std::io::Error) -> Self {
        CaptureError::Io(value)
    }
}

impl From<ApiError> for CaptureError {
    fn from(value: ApiError) -> Self {
        CaptureError::Api(value)
    }
}

impl From<EncodingError> for CaptureError {
    fn from(value: EncodingError) -> Self {
        CaptureError::ImageEncoding(value.to_string())
    }
}

fn recording_file_path() -> PathBuf {
    let mut path = env::temp_dir();
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    path.push(format!("cleo-recording-{stamp}.mov"));
    path
}

fn capture_screenshot(api: &ApiClient) -> Result<(), CaptureError> {
    let content = SCShareableContent::get().map_err(CaptureError::from)?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or(CaptureError::NoDisplay)?;

    let filter = SCContentFilter::builder()
        .display(&display)
        .exclude_windows(&[])
        .build();

    let config = SCStreamConfiguration::new()
        .with_width(display.width())
        .with_height(display.height())
        .with_shows_cursor(true);

    let image = SCScreenshotManager::capture_image(&filter, &config).map_err(CaptureError::from)?;
    let rgba = image.rgba_data().map_err(CaptureError::from)?;
    let png = encode_png(image.width() as u32, image.height() as u32, &rgba)?;
    api.upload_image(png, ImageFormat::Png)
        .map_err(CaptureError::from)
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, CaptureError> {
    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(buffer)
}
