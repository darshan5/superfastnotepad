//! OS integration: reveal in Finder/Explorer, terminals, URLs, the default
//! browser, new windows, the eyedropper, and single-instance forwarding of
//! command-line arguments.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command as Proc;
use std::sync::mpsc::Sender;

pub fn open_with_default_app(target: &str) {
    #[cfg(target_os = "macos")]
    let _ = Proc::new("open").arg(target).spawn();
    #[cfg(windows)]
    let _ = Proc::new("cmd").args(["/C", "start", "", target]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Proc::new("xdg-open").arg(target).spawn();
}

/// Opens `path` in the OS default web browser (not the file type's default
/// app), falling back to the default app if the browser can't be found.
pub fn open_in_browser(path: &Path) {
    match default_browser() {
        #[cfg(target_os = "macos")]
        Some(app) => {
            let _ = Proc::new("open").arg("-a").arg(app).arg(path).spawn();
        }
        #[cfg(not(target_os = "macos"))]
        Some(exe) => {
            let _ = Proc::new(exe).arg(path).spawn();
        }
        None => open_with_default_app(&path.display().to_string()),
    }
}

/// The app bundle that opens `https:` links.
#[cfg(target_os = "macos")]
fn default_browser() -> Option<String> {
    use std::ffi::{CStr, c_char};

    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    // SAFETY: standard NSWorkspace / NSURL / NSString messages on objects
    // that are checked for nil before use.
    unsafe {
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let s: *mut AnyObject =
            msg_send![class!(NSString), stringWithUTF8String: c"https://example.com".as_ptr()];
        let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: s];
        if ws.is_null() || url.is_null() {
            return None;
        }
        let app: *mut AnyObject = msg_send![ws, URLForApplicationToOpenURL: url];
        if app.is_null() {
            return None;
        }
        let path: *mut AnyObject = msg_send![app, path];
        if path.is_null() {
            return None;
        }
        let c: *const c_char = msg_send![path, UTF8String];
        (!c.is_null()).then(|| CStr::from_ptr(c).to_string_lossy().into_owned())
    }
}

/// The executable registered for `https:` links (the user's choice).
#[cfg(windows)]
fn default_browser() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let query = |key: &str, value: &[&str]| {
        let out = Proc::new("reg")
            .arg("query")
            .arg(key)
            .args(value)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines()
            .find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
    };
    let prog_id = query(
        r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
        &["/v", "ProgId"],
    )?;
    let command = query(&format!(r"HKCR\{prog_id}\shell\open\command"), &["/ve"])?;
    browser_exe(&command)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_browser() -> Option<String> {
    None
}

/// The program in a registry open command such as
/// `"C:\Program Files\App\app.exe" --single-argument %1`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn browser_exe(command: &str) -> Option<String> {
    let command = command.trim();
    let exe = match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => command.split_whitespace().next()?,
    };
    (!exe.is_empty()).then(|| exe.to_string())
}

pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Proc::new("open").arg("-R").arg(path).spawn();
    #[cfg(windows)]
    let _ = Proc::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(dir) = path.parent() {
        let _ = Proc::new("xdg-open").arg(dir).spawn();
    }
}

pub fn open_terminal(dir: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Proc::new("open").args(["-a", "Terminal"]).arg(dir).spawn();
    #[cfg(windows)]
    let _ = Proc::new("cmd")
        .args(["/C", "start", "cmd", "/K", "cd", "/d"])
        .arg(dir)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Proc::new("x-terminal-emulator").current_dir(dir).spawn();
}

/// Starts another editor window as a separate process. Extra windows don't
/// restore or save the session (only the first window owns it), so closing one
/// with unsaved changes asks to save them. `args` are extra command-line
/// arguments, such as files to open.
pub fn open_new_window(args: &[std::ffi::OsString]) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    // Inside a macOS .app, launch through LaunchServices so the new window
    // gets a proper Dock entry and the bundle's icon.
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        Proc::new("open")
            .arg("-n")
            .arg(bundle)
            .arg("--args")
            .args(["-multiInst", "-nosession"])
            .args(args)
            .spawn()?;
        return Ok(());
    }
    Proc::new(exe)
        .args(["-multiInst", "-nosession"])
        .args(args)
        .spawn()?;
    Ok(())
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ------------------------------------------------------------------ single instance

fn lock_path() -> std::path::PathBuf {
    crate::config::session_dir().join("instance.lock")
}

/// Tries to hand `args` to an already running instance. Returns true on success.
pub fn forward_to_running_instance(args: &[String]) -> bool {
    let Ok(lock) = std::fs::read_to_string(lock_path()) else {
        return false;
    };
    let mut parts = lock.split_whitespace();
    let (Some(port), Some(token)) = (parts.next(), parts.next()) else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port.parse().unwrap_or(0))) else {
        return false;
    };
    let cwd = std::env::current_dir().unwrap_or_default();
    let payload = serde_json::json!({ "token": token, "cwd": cwd, "args": args });
    writeln!(stream, "{payload}").is_ok()
}

/// Starts listening for argument hand-offs from future instances.
pub fn start_instance_server(tx: Sender<Vec<String>>, wake: impl Fn() + Send + 'static) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
        return;
    };
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let token = format!(
        "{:x}{:x}",
        crate::textops::Rng::seeded().next(),
        std::process::id()
    );
    let _ = std::fs::create_dir_all(crate::config::config_dir());
    if std::fs::write(lock_path(), format!("{} {token}", addr.port())).is_err() {
        return;
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if v["token"].as_str() != Some(token.as_str()) {
                continue;
            }
            let cwd = v["cwd"]
                .as_str()
                .map(std::path::PathBuf::from)
                .unwrap_or_default();
            let args: Vec<String> = v["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| {
                            if s.starts_with('-') || Path::new(s).is_absolute() {
                                s.to_string()
                            } else {
                                cwd.join(s).display().to_string()
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let _ = tx.send(args);
            wake();
        }
    });
}

pub fn release_instance_lock() {
    let _ = std::fs::remove_file(lock_path());
}

/// Files opened from Finder ("Open With", double-click, drop on the Dock
/// icon). macOS sends these to the app delegate as `application:openURLs:`,
/// which the window library's delegate doesn't implement; adding the method
/// to `NSObject` makes every object, the delegate included, accept them.
#[cfg(target_os = "macos")]
pub mod finder {
    use std::ffi::{CStr, c_char};
    use std::sync::{Mutex, OnceLock};

    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{msg_send, sel};

    static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

    unsafe extern "C-unwind" fn open_urls(
        _this: *mut AnyObject,
        _cmd: Sel,
        _app: *mut AnyObject,
        urls: *mut AnyObject,
    ) {
        if urls.is_null() {
            return;
        }
        // SAFETY: AppKit passes an NSArray<NSURL *>; each message below is
        // a standard NSArray / NSURL / NSString method.
        let paths: Vec<String> = unsafe {
            let n: usize = msg_send![urls, count];
            (0..n)
                .filter_map(|i| {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                    let path: *mut AnyObject = msg_send![url, path];
                    if path.is_null() {
                        return None;
                    }
                    let c: *const c_char = msg_send![path, UTF8String];
                    (!c.is_null()).then(|| CStr::from_ptr(c).to_string_lossy().into_owned())
                })
                .collect()
        };
        if let Ok(mut q) = OPENED.lock() {
            q.extend(paths);
        }
        if let Some(w) = WAKE.get() {
            w();
        }
    }

    /// Call once, before the event loop starts.
    pub fn install() {
        let Some(cls) = AnyClass::get(c"NSObject") else {
            return;
        };
        let f: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject) =
            open_urls;
        // SAFETY: the function matches the `v@:@@` signature of
        // `application:openURLs:`; NSObject doesn't define it already.
        unsafe {
            let imp: Imp = std::mem::transmute(f);
            objc2::ffi::class_addMethod(
                std::ptr::from_ref(cls).cast_mut(),
                sel!(application:openURLs:),
                imp,
                c"v@:@@".as_ptr(),
            );
        }
    }

    /// How to wake the UI when files arrive.
    pub fn set_wake(f: impl Fn() + Send + Sync + 'static) {
        let _ = WAKE.set(Box::new(f));
    }

    /// Files opened from Finder since the last call.
    pub fn take() -> Vec<String> {
        OPENED
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }
}

/// Eyedropper: samples one pixel from anywhere on screen.
///
/// macOS shows the system color sampler (a magnifier; click to pick, Esc to
/// cancel). Windows follows the pointer and picks on Space or Enter, so the
/// pick doesn't click into the window underneath.
pub mod eyedropper {
    /// The result of a sample in progress.
    pub enum Sample {
        /// Still sampling; on Windows, the color under the pointer.
        Pending(Option<[u8; 3]>),
        /// Finished: the picked color, or `None` if cancelled.
        Done(Option<[u8; 3]>),
    }

    pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

    pub const HINT: &str = if cfg!(target_os = "macos") {
        "Click any pixel on screen. Esc cancels."
    } else {
        "Point at any pixel on screen and press Space or Enter. Esc cancels."
    };

    #[cfg(target_os = "macos")]
    mod imp {
        use std::sync::Mutex;

        use block2::RcBlock;
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};

        use super::Sample;

        /// `None` while sampling, then the result.
        static RESULT: Mutex<Option<Option<[u8; 3]>>> = Mutex::new(None);

        pub fn start() -> bool {
            if let Ok(mut r) = RESULT.lock() {
                *r = None;
            }
            let handler = RcBlock::new(|color: *mut AnyObject| {
                // SAFETY: AppKit passes an NSColor (or nil when cancelled);
                // the messages are standard NSColor / NSColorSpace methods.
                let rgb = unsafe {
                    if color.is_null() {
                        None
                    } else {
                        let srgb: *mut AnyObject = msg_send![class!(NSColorSpace), sRGBColorSpace];
                        let c: *mut AnyObject = msg_send![color, colorUsingColorSpace: srgb];
                        if c.is_null() {
                            None
                        } else {
                            let r: f64 = msg_send![c, redComponent];
                            let g: f64 = msg_send![c, greenComponent];
                            let b: f64 = msg_send![c, blueComponent];
                            let to = |f: f64| (f * 255.0).round().clamp(0.0, 255.0) as u8;
                            Some([to(r), to(g), to(b)])
                        }
                    }
                };
                if let Ok(mut r) = RESULT.lock() {
                    *r = Some(rgb);
                }
            });
            // SAFETY: NSColorSampler (macOS 10.15+) is created and shown on
            // the main thread, where the UI runs. The sampler is kept alive
            // (leaked) until it calls the handler.
            unsafe {
                let sampler: *mut AnyObject = msg_send![class!(NSColorSampler), new];
                if sampler.is_null() {
                    return false;
                }
                let () = msg_send![sampler, showSamplerWithSelectionHandler: &*handler];
            }
            true
        }

        pub fn poll() -> Sample {
            match RESULT.lock().ok().and_then(|mut r| r.take()) {
                Some(done) => Sample::Done(done),
                None => Sample::Pending(None),
            }
        }
    }

    #[cfg(windows)]
    mod imp {
        use std::sync::atomic::{AtomicBool, Ordering};

        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{CLR_INVALID, GetDC, GetPixel, ReleaseDC};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_ESCAPE, VK_RETURN, VK_SPACE,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        use super::Sample;

        /// A pick key was down last frame (so holding it picks only once).
        static KEY_DOWN: AtomicBool = AtomicBool::new(true);

        fn down(vk: u16) -> bool {
            // SAFETY: plain Win32 call with a virtual-key code.
            (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000) != 0
        }

        fn under_pointer() -> Option<[u8; 3]> {
            // SAFETY: plain Win32 calls; the screen DC is released.
            unsafe {
                let mut p = POINT { x: 0, y: 0 };
                if GetCursorPos(&mut p) == 0 {
                    return None;
                }
                let dc = GetDC(std::ptr::null_mut());
                let c = GetPixel(dc, p.x, p.y);
                ReleaseDC(std::ptr::null_mut(), dc);
                (c != CLR_INVALID).then(|| [c as u8, (c >> 8) as u8, (c >> 16) as u8])
            }
        }

        pub fn start() -> bool {
            // The key that may have started this must be released first.
            KEY_DOWN.store(true, Ordering::Relaxed);
            true
        }

        pub fn poll() -> Sample {
            if down(VK_ESCAPE) {
                return Sample::Done(None);
            }
            let live = under_pointer();
            let key = down(VK_SPACE) || down(VK_RETURN);
            let was = KEY_DOWN.swap(key, Ordering::Relaxed);
            if key && !was {
                return Sample::Done(live);
            }
            Sample::Pending(live)
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    mod imp {
        use super::Sample;

        pub fn start() -> bool {
            false
        }

        pub fn poll() -> Sample {
            Sample::Done(None)
        }
    }

    /// Starts sampling; false if this platform can't.
    pub fn start() -> bool {
        imp::start()
    }

    /// Call every frame while sampling.
    pub fn poll() -> Sample {
        imp::poll()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn browser_exe_from_open_command() {
        assert_eq!(
            super::browser_exe(
                r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#
            )
            .as_deref(),
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
        );
        assert_eq!(
            super::browser_exe(r"C:\Browsers\firefox.exe -osint -url %1").as_deref(),
            Some(r"C:\Browsers\firefox.exe")
        );
        assert_eq!(super::browser_exe("  "), None);
    }

    /// macOS always has a browser (Safari at least) registered for https.
    #[cfg(target_os = "macos")]
    #[test]
    fn finds_default_browser() {
        let app = super::default_browser().expect("a browser");
        println!("default browser: {app}");
        assert!(app.ends_with(".app"), "{app}");
    }
}
