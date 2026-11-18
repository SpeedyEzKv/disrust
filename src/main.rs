#![windows_subsystem = "windows"]

use std::{
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::run_return::EventLoopExtRunReturn,
    platform::windows::{IconExtWindows, WindowBuilderExtWindows},
    window::{Fullscreen, Icon, Theme, WindowBuilder},
};
use url::Url;
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, ContainsFullScreenElementChangedEventHandler,
    DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2NavigationCompletedEventArgs2},
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, take_pwstr,
};
use windows::{
    Win32::UI::{
        Shell::ShellExecuteW,
        WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL},
    },
    core::{BOOL, Interface, PCWSTR, PWSTR, w},
};
use wry::{
    PermissionKind, PermissionResponse, WebContext, WebViewBuilder, WebViewBuilderExtWindows,
    WebViewExtWindows,
};

const APP_NAME: &str = "Disrust";
const DISCORD: &str = "https://discord.com/login";
const OFFLINE_PROBE: &str = "<!doctype html><meta charset=utf-8><title>Disrust offline check</title><style>html{color-scheme:dark;font:16px system-ui}</style><p>Disrust offline check</p>";

enum AppEvent {
    Fullscreen(bool),
    Open(String),
    CheckStartup(u8),
    RetryStartup(u8),
    CheckBrowser(bool),
    Probe(Instant),
    Exit,
}

const PROBE_SAMPLES: usize = 80;

struct Diagnostic {
    output: PathBuf,
    offline: bool,
}

impl Diagnostic {
    fn parse() -> Result<Option<Self>, Box<dyn Error>> {
        let mut args = std::env::args_os().skip(1);
        let Some(mode) = args.next() else {
            return Ok(None);
        };
        let output = if mode == "--diagnose" {
            args.next().map(PathBuf::from).filter(|path| path.is_dir())
        } else {
            None
        };
        let offline = match args.next().as_deref() {
            None => false,
            Some(flag) if flag == "--offline" => true,
            _ => return Err("Использование: disrust --diagnose <папка> [--offline]".into()),
        };
        if let Some(output) = output.filter(|_| args.next().is_none()) {
            Ok(Some(Self { output, offline }))
        } else {
            Err("Для диагностики укажи существующую папку: --diagnose <папка> [--offline]".into())
        }
    }
}

fn main() {
    let started = Instant::now();
    if let Err(error) = Diagnostic::parse().and_then(|diagnostic| {
        let result = run(started, diagnostic.as_ref());
        if let (Err(error), Some(diagnostic)) = (&result, diagnostic) {
            let _ = fs::write(
                diagnostic.output.join("startup-error.txt"),
                error.to_string(),
            );
            std::process::exit(1);
        }
        result
    }) {
        let message = format!(
            "Не удалось открыть Disrust. Проверь, что установлен Microsoft Edge WebView2 Runtime.\n\n{error}"
        );
        let wide: Vec<u16> = message.encode_utf16().chain([0]).collect();
        // The UTF-16 buffer stays alive for the duration of the dialog call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(wide.as_ptr()),
                w!("Disrust"),
                MB_OK | MB_ICONERROR,
            )
        };
        std::process::exit(1);
    }
}

fn run(started: Instant, diagnostic: Option<&Diagnostic>) -> Result<(), Box<dyn Error>> {
    let profile = match diagnostic {
        Some(diagnostic) => diagnostic.output.join("profile"),
        None => {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("Не найдена папка LocalAppData")?)
                .join("Disrust")
                .join("Profile")
        }
    };
    fs::create_dir_all(&profile)?;

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let icon = Icon::from_resource(1, None).ok();
    let window = WindowBuilder::new()
        .with_title(APP_NAME)
        .with_window_icon(icon.clone())
        .with_taskbar_icon(icon)
        .with_theme(Some(Theme::Dark))
        .with_inner_size(LogicalSize::new(1180, 760))
        .with_min_inner_size(LogicalSize::new(640, 460))
        .with_visible(diagnostic.is_none())
        .build(&event_loop)?;

    let mut context = WebContext::new(Some(profile));
    let builder = WebViewBuilder::new_with_web_context(&mut context)
        .with_background_color((28, 29, 34, 255))
        .with_theme(wry::Theme::Dark)
        .with_devtools(false)
        // Keep WebView2's default SmartScreen, security and autoplay settings.
        .with_additional_browser_args("")
        .with_permission_handler(|kind| match kind {
            PermissionKind::Microphone
            | PermissionKind::Camera
            | PermissionKind::Notifications
            | PermissionKind::ClipboardRead
            | PermissionKind::Autoplay => PermissionResponse::Default,
            _ => PermissionResponse::Deny,
        });
    let webview = builder.build(&window)?;
    let core = webview.webview();
    let full_screen_proxy = event_loop.create_proxy();
    let popup_proxy = event_loop.create_proxy();
    let startup_proxy = event_loop.create_proxy();
    let offline = diagnostic.is_some_and(|diagnostic| diagnostic.offline);

    // Runs on the thread that owns WebView2. No objects or messages bridge the site to native code.
    unsafe {
        let settings = core.Settings()?;
        settings.SetAreHostObjectsAllowed(false)?;
        settings.SetIsWebMessageEnabled(false)?;
        core.add_ContainsFullScreenElementChanged(
            &ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _| {
                if let Some(sender) = sender {
                    let mut fullscreen = BOOL::default();
                    sender.ContainsFullScreenElement(&mut fullscreen)?;
                    let _ =
                        full_screen_proxy.send_event(AppEvent::Fullscreen(fullscreen.as_bool()));
                }
                Ok(())
            })),
            &mut 0,
        )?;
        core.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = take_pwstr(uri);
                    let allowed = uri == "about:blank"
                        || (offline && uri.starts_with("data:text/html"))
                        || parse_http(&uri).is_some_and(|url| is_discord(&url));
                    if !allowed {
                        args.SetCancel(true)?;
                        let mut initiated = BOOL::default();
                        args.IsUserInitiated(&mut initiated)?;
                        if initiated.as_bool() {
                            open_external(&uri);
                        }
                    }
                }
                Ok(())
            })),
            &mut 0,
        )?;
        core.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                    let mut initiated = BOOL::default();
                    args.IsUserInitiated(&mut initiated)?;
                    if initiated.as_bool() {
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        let uri = take_pwstr(uri);
                        if parse_http(&uri).is_some_and(|url| is_discord(&url)) {
                            let _ = popup_proxy.send_event(AppEvent::Open(uri));
                        } else {
                            open_external(&uri);
                        }
                    }
                }
                Ok(())
            })),
            &mut 0,
        )?;
    }

    if let Some(diagnostic) = diagnostic {
        monitor_navigation(&core, started, &diagnostic.output)?;
        monitor_network(&core, &diagnostic.output)?;
        fs::write(
            diagnostic.output.join("host-ready.txt"),
            started.elapsed().as_millis().to_string(),
        )?;
    }
    if offline {
        webview.load_html(OFFLINE_PROBE)?;
    } else {
        schedule_startup_check(&core, event_loop.create_proxy())?;
        webview.load_url(DISCORD)?;
    }

    let diagnostic_output = diagnostic.map(|diagnostic| diagnostic.output.clone());
    if let Some(output) = &diagnostic_output {
        let proxy = event_loop.create_proxy();
        let output = output.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(60);
            let browser_report = output.join("browser-check.json");
            for attempt in 1..=38 {
                if output.join("navigation.txt").is_file() {
                    if browser_report.is_file() {
                        break;
                    }
                    if proxy
                        .send_event(AppEvent::CheckBrowser(attempt == 38))
                        .is_err()
                    {
                        return;
                    }
                }
                thread::sleep(Duration::from_secs(1));
            }
            for _ in 0..PROBE_SAMPLES {
                if proxy.send_event(AppEvent::Probe(Instant::now())).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            while !output.join("stop").exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(100));
            }
            let _ = proxy.send_event(AppEvent::Exit);
        });
    }
    let mut probe_samples = Vec::new();
    let exit = event_loop.run_return(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *flow = ControlFlow::Exit,
            Event::UserEvent(AppEvent::Fullscreen(fullscreen)) => {
                window.set_fullscreen(
                    fullscreen.then(|| Fullscreen::Borderless(window.current_monitor())),
                );
            }
            Event::UserEvent(AppEvent::Open(uri)) => {
                if let Err(error) = webview.load_url(&uri) {
                    let wide: Vec<u16> = error.to_string().encode_utf16().chain([0]).collect();
                    // The parent window is available, but no native object is exposed to the page.
                    unsafe {
                        MessageBoxW(
                            None,
                            PCWSTR(wide.as_ptr()),
                            w!("Disrust"),
                            MB_OK | MB_ICONERROR,
                        )
                    };
                }
            }
            Event::UserEvent(AppEvent::CheckStartup(attempt)) => {
                let proxy = startup_proxy.clone();
                let _ = webview.evaluate_script_with_callback(
                    r#"(() => {
                        const app = document.getElementById("app-mount");
                        return location.origin === "https://discord.com" && location.pathname === "/login"
                            && document.readyState === "complete" && !!app
                            && !app.innerText.trim() && !app.querySelector("input, button, iframe, [contenteditable]");
                    })()"#,
                    move |empty| {
                        if empty == "true" {
                            let _ = proxy.send_event(AppEvent::RetryStartup(attempt));
                        }
                    },
                );
            }
            Event::UserEvent(AppEvent::RetryStartup(attempt)) => {
                // Reissue a normal HTTPS request. WebView2 still validates the server certificate.
                let _ = unsafe { core.Reload() };
                if let Some(output) = &diagnostic_output {
                    let _ = fs::write(output.join("startup-retry.txt"), attempt.to_string());
                }
                if attempt < 2 {
                    let proxy = startup_proxy.clone();
                    thread::spawn(move || {
                        thread::sleep(Duration::from_secs(11));
                        let _ = proxy.send_event(AppEvent::CheckStartup(attempt + 1));
                    });
                }
            }
            Event::UserEvent(AppEvent::CheckBrowser(last_attempt)) => {
                if let Some(output) = &diagnostic_output {
                    check_browser(&webview, started, output, last_attempt);
                }
            }
            Event::UserEvent(AppEvent::Probe(sent)) => {
                probe_samples.push(sent.elapsed().as_micros());
                if probe_samples.len() == PROBE_SAMPLES
                    && let Some(output) = &diagnostic_output
                {
                    probe_samples.sort_unstable();
                    let p95 = probe_samples[PROBE_SAMPLES * 95 / 100 - 1];
                    let max = probe_samples[PROBE_SAMPLES - 1];
                    let _ = fs::write(
                        output.join("ui-latency.txt"),
                        format!("count={PROBE_SAMPLES}\np95_us={p95}\nmax_us={max}\n"),
                    );
                }
            }
            Event::UserEvent(AppEvent::Exit) => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
    if exit == 0 {
        Ok(())
    } else {
        Err(format!("Окно завершилось с кодом {exit}").into())
    }
}

fn schedule_startup_check(
    core: &ICoreWebView2,
    proxy: EventLoopProxy<AppEvent>,
) -> windows::core::Result<()> {
    let scheduled = std::cell::Cell::new(false);
    // NavigationCompleted may succeed while individual resources fail during runtime startup.
    unsafe {
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut ok = BOOL::default();
                    args.IsSuccess(&mut ok)?;
                    if ok.as_bool() && !scheduled.replace(true) {
                        let proxy = proxy.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(1250));
                            let _ = proxy.send_event(AppEvent::CheckStartup(1));
                        });
                    }
                }
                Ok(())
            })),
            &mut 0,
        )
    }
}

fn check_browser(webview: &wry::WebView, started: Instant, output: &Path, last_attempt: bool) {
    let report = output.join("browser-check.json");
    let ready_time = output.join("page-ready.txt");
    // Diagnostics report only capability flags from the isolated WebView2 profile.
    let _ = webview.evaluate_script_with_callback(
        r#"({
            discordOrigin: location.origin === "https://discord.com",
            loginForm: location.pathname === "/login" && !!document.querySelector("input[type=password]"),
            notificationsApi: typeof Notification === "function",
            microphoneApi: typeof navigator.mediaDevices?.getUserMedia === "function",
            screenCaptureApi: typeof navigator.mediaDevices?.getDisplayMedia === "function",
            offlinePage: document.title === "Disrust offline check"
        })"#,
        move |result| {
            let ready = result.contains("\"loginForm\":true") || result.contains("\"offlinePage\":true");
            if (ready || last_attempt) && !report.is_file() {
                if ready {
                    let _ = fs::write(&ready_time, started.elapsed().as_millis().to_string());
                }
                let _ = fs::write(&report, result);
            }
        },
    );
}

fn monitor_network(core: &ICoreWebView2, output: &Path) -> windows::core::Result<()> {
    let log = output.join("network-errors.txt");
    // Collect Chromium error identifiers only. URLs and request fields stay in WebView2.
    unsafe {
        core.GetDevToolsProtocolEventReceiver(w!("Network.loadingFailed"))?
            .add_DevToolsProtocolEventReceived(
                &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let mut value = PWSTR::null();
                        args.ParameterObjectAsJson(&mut value)?;
                        let json = take_pwstr(value);
                        if let Some((_, tail)) = json.split_once("net::ERR_") {
                            let code = tail
                                .split(|c: char| {
                                    !(c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                                })
                                .next()
                                .unwrap_or("");
                            if let Ok(mut file) =
                                fs::OpenOptions::new().create(true).append(true).open(&log)
                            {
                                let _ = writeln!(file, "net::ERR_{code}");
                            }
                        }
                    }
                    Ok(())
                })),
                &mut 0,
            )?;
        core.CallDevToolsProtocolMethod(
            w!("Network.enable"),
            w!("{}"),
            &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(()))),
        )
    }
}

fn parse_http(uri: &str) -> Option<Url> {
    let url = Url::parse(uri).ok()?;
    (matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

fn is_discord(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("discord.com") && url.port().is_none()
}

fn open_external(uri: &str) {
    if let Some(url) = parse_http(uri) {
        let wide: Vec<u16> = url.as_str().encode_utf16().chain([0]).collect();
        // This invokes the registered HTTP(S) handler with a validated URL, without a command shell.
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
    }
}

fn monitor_navigation(
    core: &ICoreWebView2,
    started: Instant,
    output: &Path,
) -> windows::core::Result<()> {
    let output = output.join("navigation.txt");
    // This event is registered only with an isolated profile in diagnostic runs.
    unsafe {
        core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |sender, args| {
                if let (Some(sender), Some(args)) = (sender, args) {
                    let mut ok = BOOL::default();
                    args.IsSuccess(&mut ok)?;
                    let mut pid = 0;
                    sender.BrowserProcessId(&mut pid)?;
                    let mut title = PWSTR::null();
                    sender.DocumentTitle(&mut title)?;
                    let title = take_pwstr(title);
                    let mut http_status = 0;
                    if let Ok(args) = args.cast::<ICoreWebView2NavigationCompletedEventArgs2>() {
                        args.HttpStatusCode(&mut http_status)?;
                    }
                    let log = format!(
                        "pid={pid}\nelapsed_ms={}\nsuccess={}\nhttp_status={http_status}\ntitle={}\n",
                        started.elapsed().as_millis(),
                        ok.as_bool(),
                        title.replace(['\r', '\n'], " "),
                    );
                    let _ = fs::write(&output, log);
                }
                Ok(())
            })),
            &mut 0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_https_origin_stays_in_the_app() {
        assert!(parse_http(DISCORD).is_some_and(|url| is_discord(&url)));
        for url in [
            "http://discord.com/login",
            "https://discord.com:8443/login",
            "https://discord.com.example.org/login",
            "https://discord.com@other.example/login",
            "https://other.example@discord.com/login",
            "file:///C:/Windows/explorer.exe",
            "javascript:alert(1)",
            "discord://-/channels",
        ] {
            assert!(
                !parse_http(url).is_some_and(|url| is_discord(&url)),
                "{url}"
            );
        }
    }
}
