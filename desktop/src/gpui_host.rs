use crate::{host::UiRequest, ipc};
use gpui_kit::{prelude::*, *};
use gpui_webview::WebView;
use std::sync::{mpsc, Arc};
use std::time::Duration;

struct GameWindow {
    // Drop the native child before its WebContext and GPUI parent.
    webview: Entity<WebView>,
    _web_context: wry::WebContext,
    _runtime: Arc<crate::DesktopRuntime>,
}

impl Render for GameWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x101014))
            .child(self.webview.clone())
    }
}

pub fn run() -> anyhow::Result<()> {
    // A persistent Wry WebContext creates GTK objects before WebView::build
    // can initialize GTK itself. Initialize on the UI thread first.
    #[cfg(target_os = "linux")]
    {
        gtk::gdk::set_allowed_backends("x11");
        gtk::init().map_err(|error| anyhow::anyhow!("無法初始化 X11/GTK：{error}"))?;
    }
    // The native WebView2 child requires this renderer mode (upstream example).
    #[cfg(target_os = "windows")]
    std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "true");
    let (ui_tx, ui_rx) = mpsc::channel();
    let runtime = Arc::new(crate::prepare(ui_tx)?);
    let tokio = runtime._runtime.handle().clone();
    let capability = uuid::Uuid::new_v4().to_string();
    let script = ipc::initialization_script(&runtime.server_url, &capability);
    let (reply_tx, reply_rx) = mpsc::channel();
    #[cfg(target_os = "linux")]
    let application = gpui_kit::platform::linux(WindowingModes::X11);
    #[cfg(not(target_os = "linux"))]
    let application = gpui_kit::application();
    application.run(move |cx| {
        gpui_kit::init(cx);
        cx.on_window_closed(|cx, _| { if cx.windows().is_empty() { cx.quit(); } }).detach();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1280.), px(720.)), cx))),
            window_min_size: Some(size(px(800.), px(600.))),
            titlebar: Some(TitlebarOptions { title: Some("ReversedFront".into()), ..Default::default() }),
            ..Default::default()
        };
        let result = gpui_kit::open_window(options, cx, move |window, cx| {
            let mut context = wry::WebContext::new(Some(runtime.session_dir.clone()));
            let origin = runtime.server_url.clone();
            let nav_origin = origin.clone();
            let host = runtime.host.clone();
            let builder = wry::WebViewBuilder::new_with_web_context(&mut context)
                .with_url(format!("{origin}/?rf_account_slot={}", host.process_slot))
                .with_initialization_script_for_main_only(script, true)
                .with_navigation_handler(move |url| ipc::same_origin(&url, &nav_origin)
                    || reqwest::Url::parse(&url).is_ok_and(|url| url.scheme() == "https"))
                .with_clipboard(true)
                .with_ipc_handler(move |request| {
                    match ipc::parse(&request.uri().to_string(), request.body(), &origin, &capability) {
                        Ok(request) => {
                            let host = host.clone();
                            let reply_tx = reply_tx.clone();
                            let reply_origin = serde_json::json!(origin).to_string();
                            tokio.spawn(async move {
                                let reply = ipc::dispatch(&host, request).await;
                                // Navigation can finish before an asynchronous command does.
                                let _ = reply_tx.send(format!("if (window === window.top && location.origin === {reply_origin}) {{ {reply} }}"));
                            });
                        }
                        Err(error) => eprintln!("[Native IPC] {error}"),
                    }
                });
            #[cfg(target_os = "macos")]
            let builder = {
                use wry::WebViewBuilderExtDarwin;
                builder.with_data_store_identifier(crate::commands::process_data_store_identifier(
                    &format!("main-slot:{}", runtime.host.process_slot)))
            };
            let webview = cx.new(|cx| WebView::build(builder, window, cx)
                .expect("Unable to create ReversedFront game WebView"));
            let handle = window.window_handle();
            let weak_webview = webview.downgrade();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    if handle.update(cx, |_, window, cx| {
                        for request in ui_rx.try_iter() {
                            match request {
                                UiRequest::Exit => cx.quit(),
                                UiRequest::ToggleFullscreen(reply) => {
                                    let target = !window.is_fullscreen();
                                    window.toggle_fullscreen();
                                    let _ = reply.send(Ok(target));
                                }
                            }
                        }
                        for script in reply_rx.try_iter() {
                            let _ = weak_webview.update(cx, |view, _| {
                                if let Err(error) = view.raw().evaluate_script(&script) {
                                    eprintln!("[Native IPC] Reply failed: {error}");
                                }
                            });
                        }
                    }).is_err() { break; }
                }
            }).detach();
            cx.new(|_| GameWindow { webview, _web_context: context, _runtime: runtime })
        });
        if let Err(error) = result {
            eprintln!("ReversedFront 視窗建立失敗：{error:#}");
            cx.quit();
        }
        cx.activate(true);
    });
    Ok(())
}
