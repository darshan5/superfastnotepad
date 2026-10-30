//! OS integration: reveal in Finder/Explorer, terminals, URLs, the Run
//! menu, and single-instance forwarding of command-line arguments.

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

/// Runs a user command line through the system shell (Run menu).
pub fn run_shell(cmdline: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        Proc::new("cmd").args(["/C", cmdline]).spawn()?;
    }
    #[cfg(not(windows))]
    {
        Proc::new("sh").args(["-c", cmdline]).spawn()?;
    }
    Ok(())
}

/// Starts another editor window as a separate process. Extra windows don't
/// restore or save the session (only the first window owns it), so closing one
/// with unsaved changes asks to save them.
pub fn open_new_window(files: &[std::path::PathBuf]) -> std::io::Result<()> {
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
            .args(files)
            .spawn()?;
        return Ok(());
    }
    Proc::new(exe)
        .args(["-multiInst", "-nosession"])
        .args(files)
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
