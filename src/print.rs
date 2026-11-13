//! File > Print: the document (or selection) as RTF, handed to the system's
//! print dialog. macOS shows Cocoa's print panel as a sheet (with preview
//! and Save as PDF); Windows shows the Print dialog and lays the pages out
//! with a hidden RichEdit control. Long lines wrap; tabs become spaces.

use std::fmt::Write as _;

use egui::Color32;

use crate::lexer::{Span, TokenKind};
use crate::theme::LangStyles;

/// The fixed-width font printed with.
pub const FONT: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(windows) {
    "Consolas"
} else {
    "Courier New"
};

/// [`FONT`]'s character width, in ems.
const ADVANCE: f32 = if cfg!(target_os = "macos") {
    0.602
} else if cfg!(windows) {
    0.55
} else {
    0.6
};

/// Line numbers are printed in this grey.
const LINE_NUMBER_COLOR: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x8A);

/// What to print and how.
pub struct PrintJob {
    /// The document's name: the job name and the page header.
    pub title: String,
    pub rtf: String,
    pub header_footer: bool,
}

/// One printed line: its number (from 1), full text and syntax spans (byte
/// offsets), and the byte range of it to print (all of it except at the
/// ends of a selection).
pub struct PrintLine {
    pub number: usize,
    pub text: String,
    pub spans: Vec<Span>,
    pub from: usize,
    pub to: usize,
}

/// How a run of text looks on paper.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RunStyle {
    /// Index into the colour table (from 1); 0 for the default black.
    color: usize,
    bold: bool,
    italic: bool,
    underline: bool,
}

/// RTF for `lines` in `font_size` points. With `styles`, text is coloured
/// by token kind; `numbers` prefixes each line with its number.
pub fn build_rtf(
    lines: &[PrintLine],
    styles: Option<&LangStyles>,
    numbers: bool,
    font_size: f32,
    tab_width: usize,
) -> String {
    let tab_width = tab_width.max(1);
    let mut colors: Vec<Color32> = Vec::new();
    let mut color_index = |c: Color32| -> usize {
        if c == Color32::BLACK {
            return 0;
        }
        colors.iter().position(|x| *x == c).map_or_else(
            || {
                colors.push(c);
                colors.len()
            },
            |i| i + 1,
        )
    };
    let number_color = color_index(LINE_NUMBER_COLOR);
    let width = lines.last().map_or(1, |l| l.number.to_string().len());

    let mut body = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            body.push_str("\\par\n");
        }
        if numbers {
            let _ = write!(body, "{{\\cf{number_color} {:>width$}  }}", line.number);
        }
        // Runs of one style, with tabs expanded from the line's start.
        let mut col = 0;
        let mut span_i = 0;
        let mut run = String::new();
        let mut run_style: Option<RunStyle> = None;
        for (b, ch) in line.text.char_indices() {
            let n = if ch == '\t' {
                tab_width - col % tab_width
            } else {
                1
            };
            col += n;
            if b < line.from || b >= line.to {
                continue;
            }
            while span_i < line.spans.len() && line.spans[span_i].1 <= b {
                span_i += 1;
            }
            let kind = match line.spans.get(span_i) {
                Some(&(s, _, k)) if s <= b => k,
                _ => TokenKind::Default,
            };
            let style = styles.map_or(
                RunStyle {
                    color: 0,
                    bold: false,
                    italic: false,
                    underline: false,
                },
                |st| {
                    let s = st.get(kind);
                    RunStyle {
                        color: color_index(s.fg),
                        bold: s.bold,
                        italic: s.italic,
                        underline: s.underline,
                    }
                },
            );
            if run_style.is_some_and(|r| r != style) {
                push_run(&mut body, &run, run_style.unwrap());
                run.clear();
            }
            run_style = Some(style);
            if ch == '\t' {
                run.extend(std::iter::repeat_n(' ', n));
            } else {
                run.push(ch);
            }
        }
        if let Some(style) = run_style {
            push_run(&mut body, &run, style);
        }
    }

    let mut out = String::new();
    let _ = write!(
        out,
        "{{\\rtf1\\ansi\\deff0{{\\fonttbl{{\\f0\\fmodern {FONT};}}}}{{\\colortbl;"
    );
    for c in &colors {
        let _ = write!(out, "\\red{}\\green{}\\blue{};", c.r(), c.g(), c.b());
    }
    let font_size = font_size.clamp(4.0, 72.0);
    // Half-points.
    let size = (font_size * 2.0).round() as u32;
    let _ = write!(out, "}}\\pard\\plain\\f0\\fs{size}");
    if numbers {
        // Wrapped lines continue under the text, not the numbers (twips).
        let indent = ((width + 2) as f32 * font_size * 20.0 * ADVANCE).round() as u32;
        let _ = write!(out, "\\li{indent}\\fi-{indent}");
    }
    out.push('\n');
    out.push_str(&body);
    out.push_str("\n}");
    out
}

fn push_run(out: &mut String, text: &str, s: RunStyle) {
    out.push('{');
    if s.color > 0 {
        let _ = write!(out, "\\cf{}", s.color);
    }
    if s.bold {
        out.push_str("\\b");
    }
    if s.italic {
        out.push_str("\\i");
    }
    if s.underline {
        out.push_str("\\ul");
    }
    out.push(' ');
    escape(out, text);
    out.push('}');
}

/// RTF text: ASCII, with control characters escaped and the rest as
/// `\uN?` (UTF-16, signed).
fn escape(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(ch);
            }
            ' '..='~' => out.push(ch),
            c if (c as u32) < 0x20 || c == '\u{7F}' => out.push(' '),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{}?", *unit as i16);
                }
            }
        }
    }
}

/// Shows the print dialog for `job`. `Ok` also when the user cancels.
pub fn print(job: &PrintJob) -> Result<(), String> {
    imp::print(job, None)
}

/// Prints `job` straight to a PDF file, without any dialog (macOS only).
#[allow(dead_code)]
pub fn print_to_pdf(job: &PrintJob, pdf: &std::path::Path) -> Result<(), String> {
    imp::print(job, Some(pdf))
}

#[cfg(target_os = "macos")]
mod imp {
    use std::cell::RefCell;
    use std::ffi::{CString, c_void};
    use std::path::Path;

    use objc2::encode::{Encode, Encoding, RefEncode};
    use objc2::runtime::{AnyObject, Sel};
    use objc2::{class, msg_send};

    use super::PrintJob;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }

    // SAFETY: the layouts match CGPoint, CGSize and CGRect on 64-bit macOS.
    unsafe impl Encode for Point {
        const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl RefEncode for Point {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }
    unsafe impl Encode for Size {
        const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
    }
    unsafe impl RefEncode for Size {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }
    unsafe impl Encode for Rect {
        const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
    }
    unsafe impl RefEncode for Rect {
        const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSPrintHeaderAndFooter: *mut AnyObject;
        static NSPrintJobSavingURL: *mut AnyObject;
        static NSPrintSaveJob: *mut AnyObject;
    }

    thread_local! {
        /// Objects of the last print, released when the next one starts (a
        /// sheet returns before printing is done).
        static LAST: RefCell<Vec<*mut AnyObject>> = const { RefCell::new(Vec::new()) };
    }

    unsafe fn ns_string(s: &str) -> *mut AnyObject {
        let c = CString::new(s.replace('\0', "")).unwrap_or_default();
        // SAFETY: a valid NUL-terminated UTF-8 string.
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
    }

    /// Prints `job`: a print panel sheet on the key window, or, with `pdf`,
    /// straight to that file without any panel (for tests).
    pub fn print(job: &PrintJob, pdf: Option<&Path>) -> Result<(), String> {
        // SAFETY: AppKit on the main thread (where the UI runs); every
        // message is a documented NSData / NSAttributedString / NSTextView
        // / NSPrintInfo / NSPrintOperation method with matching types.
        // Objects we own are released on the next print.
        unsafe {
            LAST.with_borrow_mut(|last| {
                let current: *mut AnyObject = msg_send![class!(NSPrintOperation), currentOperation];
                if current.is_null() {
                    for o in last.drain(..) {
                        let () = msg_send![o, release];
                    }
                }
            });

            let data: *mut AnyObject = msg_send![
                class!(NSData),
                dataWithBytes: job.rtf.as_ptr().cast::<c_void>(),
                length: job.rtf.len()
            ];
            let attr: *mut AnyObject = msg_send![class!(NSAttributedString), alloc];
            let attr: *mut AnyObject = msg_send![
                attr,
                initWithRTF: data,
                documentAttributes: std::ptr::null_mut::<*mut AnyObject>()
            ];
            if attr.is_null() {
                return Err("Couldn't lay out the text to print.".into());
            }

            let shared: *mut AnyObject = msg_send![class!(NSPrintInfo), sharedPrintInfo];
            let info: *mut AnyObject = msg_send![shared, copy];
            let () = msg_send![info, setHorizontallyCentered: false];
            let () = msg_send![info, setVerticallyCentered: false];
            // NSPrintingPaginationModeFit: never split a page sideways.
            let () = msg_send![info, setHorizontalPagination: 1isize];
            let dict: *mut AnyObject = msg_send![info, dictionary];
            let flag: *mut AnyObject =
                msg_send![class!(NSNumber), numberWithBool: job.header_footer];
            let () = msg_send![dict, setObject: flag, forKey: NSPrintHeaderAndFooter];
            if let Some(path) = pdf {
                let () = msg_send![info, setJobDisposition: NSPrintSaveJob];
                let url: *mut AnyObject = msg_send![
                    class!(NSURL),
                    fileURLWithPath: ns_string(&path.to_string_lossy())
                ];
                let () = msg_send![dict, setObject: url, forKey: NSPrintJobSavingURL];
            }

            let paper: Size = msg_send![info, paperSize];
            let left: f64 = msg_send![info, leftMargin];
            let right: f64 = msg_send![info, rightMargin];
            let width = (paper.width - left - right).max(72.0);
            let frame = Rect {
                origin: Point { x: 0.0, y: 0.0 },
                size: Size {
                    width,
                    height: 72.0,
                },
            };
            let view: *mut AnyObject = msg_send![class!(NSTextView), alloc];
            let view: *mut AnyObject = msg_send![view, initWithFrame: frame];
            let storage: *mut AnyObject = msg_send![view, textStorage];
            let () = msg_send![storage, setAttributedString: attr];
            let () = msg_send![view, setHorizontallyResizable: false];
            let () = msg_send![view, setVerticallyResizable: true];
            let layout: *mut AnyObject = msg_send![view, layoutManager];
            let container: *mut AnyObject = msg_send![view, textContainer];
            let () = msg_send![layout, ensureLayoutForTextContainer: container];
            let () = msg_send![view, sizeToFit];

            let op: *mut AnyObject = msg_send![
                class!(NSPrintOperation),
                printOperationWithView: view,
                printInfo: info
            ];
            if op.is_null() {
                return Err("Couldn't start printing.".into());
            }
            let op: *mut AnyObject = msg_send![op, retain];
            let () = msg_send![op, setJobTitle: ns_string(&job.title)];
            LAST.with_borrow_mut(|last| last.extend([attr, info, view, op]));

            if pdf.is_some() {
                let () = msg_send![op, setShowsPrintPanel: false];
                let () = msg_send![op, setShowsProgressPanel: false];
                let ok: bool = msg_send![op, runOperation];
                return if ok {
                    Ok(())
                } else {
                    Err("Printing failed.".into())
                };
            }
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let mut window: *mut AnyObject = msg_send![app, keyWindow];
            if window.is_null() {
                window = msg_send![app, mainWindow];
            }
            if window.is_null() {
                let _: bool = msg_send![op, runOperation];
            } else {
                let none: Option<Sel> = None;
                let () = msg_send![
                    op,
                    runOperationModalForWindow: window,
                    delegate: std::ptr::null_mut::<AnyObject>(),
                    didRunSelector: none,
                    contextInfo: std::ptr::null_mut::<c_void>()
                ];
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
#[allow(non_snake_case, clippy::upper_case_acronyms)]
mod imp {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{GlobalFree, HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateFontW, DeleteDC, DeleteObject, GetDeviceCaps, HDC, HORZRES, LOGPIXELSX, LOGPIXELSY,
        MM_TEXT, RestoreDC, SaveDC, SelectObject, SetMapMode, SetTextAlign, TA_CENTER, TA_LEFT,
        TA_RIGHT, TA_TOP, TextOutW, VERTRES,
    };
    use windows_sys::Win32::Storage::Xps::{
        AbortDoc, DOCINFOW, EndDoc, EndPage, StartDocW, StartPage,
    };
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC, PD_USEDEVMODECOPIESANDCOLLATE, PRINTDLGW,
        PrintDlgW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, ES_MULTILINE, GetForegroundWindow, SendMessageW, WS_POPUP,
    };

    use super::PrintJob;

    // RichEdit (Richedit.h), which windows-sys doesn't cover. The structs
    // need no padding on x64, so `repr(C)` matches its 4-byte packing.
    const WM_USER: u32 = 0x0400;
    const EM_EXLIMITTEXT: u32 = WM_USER + 53;
    const EM_FORMATRANGE: u32 = WM_USER + 57;
    const EM_GETTEXTLENGTHEX: u32 = WM_USER + 95;
    const EM_SETTEXTEX: u32 = WM_USER + 97;
    const GTL_PRECISE: u32 = 2;
    const GTL_NUMCHARS: u32 = 8;

    #[repr(C)]
    struct CHARRANGE {
        cpMin: i32,
        cpMax: i32,
    }

    #[repr(C)]
    struct FORMATRANGE {
        hdc: HDC,
        hdcTarget: HDC,
        rc: RECT,
        rcPage: RECT,
        chrg: CHARRANGE,
    }

    #[repr(C)]
    struct SETTEXTEX {
        flags: u32,
        codepage: u32,
    }

    #[repr(C)]
    struct GETTEXTLENGTHEX {
        flags: u32,
        codepage: u32,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Twips per inch (RichEdit's page units).
    const TWIPS: i32 = 1440;

    pub fn print(job: &PrintJob, _pdf: Option<&Path>) -> Result<(), String> {
        // SAFETY: plain Win32 calls on the UI thread. The print dialog's
        // memory, the printer DC, the fonts and the hidden RichEdit window
        // are all freed before returning.
        unsafe {
            let owner = GetForegroundWindow();
            let mut pd: PRINTDLGW = std::mem::zeroed();
            pd.lStructSize = std::mem::size_of::<PRINTDLGW>() as u32;
            pd.hwndOwner = owner;
            pd.Flags = PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_USEDEVMODECOPIESANDCOLLATE;
            pd.nCopies = 1;
            let chosen = PrintDlgW(&mut pd) != 0;
            if !pd.hDevMode.is_null() {
                GlobalFree(pd.hDevMode);
            }
            if !pd.hDevNames.is_null() {
                GlobalFree(pd.hDevNames);
            }
            if !chosen || pd.hDC.is_null() {
                return Ok(()); // cancelled
            }
            let dc = pd.hDC;
            let result = print_to(dc, job);
            DeleteDC(dc);
            result
        }
    }

    unsafe fn print_to(dc: HDC, job: &PrintJob) -> Result<(), String> {
        // SAFETY: see `print`.
        unsafe {
            if LoadLibraryW(wide("Msftedit.dll").as_ptr()).is_null() {
                return Err("RichEdit (Msftedit.dll) isn't available.".into());
            }
            let edit: HWND = CreateWindowExW(
                0,
                wide("RICHEDIT50W").as_ptr(),
                std::ptr::null(),
                WS_POPUP | ES_MULTILINE as u32,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            if edit.is_null() {
                return Err("Couldn't create the RichEdit control.".into());
            }
            let result = print_pages(dc, edit, job);
            DestroyWindow(edit);
            result
        }
    }

    unsafe fn print_pages(dc: HDC, edit: HWND, job: &PrintJob) -> Result<(), String> {
        // SAFETY: see `print`.
        unsafe {
            SendMessageW(edit, EM_EXLIMITTEXT, 0, isize::MAX);
            let rtf: Vec<u8> = job.rtf.bytes().chain(std::iter::once(0)).collect();
            let st = SETTEXTEX {
                flags: 0,    // ST_DEFAULT
                codepage: 0, // CP_ACP: the RTF is ASCII
            };
            SendMessageW(
                edit,
                EM_SETTEXTEX,
                &raw const st as usize,
                rtf.as_ptr() as isize,
            );
            let gtl = GETTEXTLENGTHEX {
                flags: GTL_NUMCHARS | GTL_PRECISE,
                codepage: 1200,
            };
            let total = SendMessageW(edit, EM_GETTEXTLENGTHEX, &raw const gtl as usize, 0) as i32;

            let dpi_x = GetDeviceCaps(dc, LOGPIXELSX as i32).max(1);
            let dpi_y = GetDeviceCaps(dc, LOGPIXELSY as i32).max(1);
            let px_w = GetDeviceCaps(dc, HORZRES as i32);
            let px_h = GetDeviceCaps(dc, VERTRES as i32);
            let tw_w = px_w * TWIPS / dpi_x;
            let tw_h = px_h * TWIPS / dpi_y;
            // Half an inch inside the printable area; room for the header
            // and footer lines.
            let margin = TWIPS / 2;
            let band = if job.header_footer { TWIPS / 3 } else { 0 };
            let body = RECT {
                left: margin,
                top: margin + band,
                right: (tw_w - margin).max(margin + TWIPS),
                bottom: (tw_h - margin - band).max(margin + band + TWIPS),
            };
            let page = RECT {
                left: 0,
                top: 0,
                right: tw_w,
                bottom: tw_h,
            };
            let to_px_x = |t: i32| t * dpi_x / TWIPS;
            let to_px_y = |t: i32| t * dpi_y / TWIPS;

            let font = CreateFontW(
                -(9 * dpi_y / 72),
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                wide("Segoe UI").as_ptr(),
            );
            let date = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
            let title = wide(&job.title);
            let doc_name = wide(&job.title);
            let info = DOCINFOW {
                cbSize: std::mem::size_of::<DOCINFOW>() as i32,
                lpszDocName: doc_name.as_ptr(),
                lpszOutput: std::ptr::null(),
                lpszDatatype: std::ptr::null(),
                fwType: 0,
            };
            if StartDocW(dc, &info) <= 0 {
                DeleteObject(font);
                // Includes cancelling "Microsoft Print to PDF"'s file prompt.
                return Ok(());
            }
            let mut cp = 0;
            let mut number = 1;
            let mut failed = false;
            loop {
                if StartPage(dc) <= 0 {
                    failed = true;
                    break;
                }
                if job.header_footer {
                    let saved = SaveDC(dc);
                    // Device pixels, whatever RichEdit left set.
                    SetMapMode(dc, MM_TEXT);
                    let old = SelectObject(dc, font);
                    let top = to_px_y(margin);
                    SetTextAlign(dc, TA_LEFT | TA_TOP);
                    TextOutW(
                        dc,
                        to_px_x(margin),
                        top,
                        title.as_ptr(),
                        (title.len() - 1) as i32,
                    );
                    let d = wide(&date);
                    SetTextAlign(dc, TA_RIGHT | TA_TOP);
                    TextOutW(
                        dc,
                        to_px_x(body.right),
                        top,
                        d.as_ptr(),
                        (d.len() - 1) as i32,
                    );
                    let p = wide(&format!("Page {number}"));
                    SetTextAlign(dc, TA_CENTER | TA_TOP);
                    TextOutW(
                        dc,
                        to_px_x((body.left + body.right) / 2),
                        to_px_y(body.bottom + band / 3),
                        p.as_ptr(),
                        (p.len() - 1) as i32,
                    );
                    SelectObject(dc, old);
                    RestoreDC(dc, saved);
                }
                let range = FORMATRANGE {
                    hdc: dc,
                    hdcTarget: dc,
                    rc: body,
                    rcPage: page,
                    chrg: CHARRANGE {
                        cpMin: cp,
                        cpMax: -1,
                    },
                };
                let next = SendMessageW(edit, EM_FORMATRANGE, 1, &raw const range as isize) as i32;
                if EndPage(dc) <= 0 {
                    failed = true;
                    break;
                }
                if next <= cp || next >= total {
                    break;
                }
                cp = next;
                number += 1;
            }
            // Frees RichEdit's cached layout.
            SendMessageW(edit, EM_FORMATRANGE, 0, 0);
            DeleteObject(font);
            if failed {
                AbortDoc(dc);
                return Err("The printer stopped the job.".into());
            }
            EndDoc(dc);
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use std::path::Path;

    use super::PrintJob;

    pub fn print(_job: &PrintJob, _pdf: Option<&Path>) -> Result<(), String> {
        Err("Printing isn't supported on this platform yet.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Style;

    fn line(number: usize, text: &str, spans: Vec<Span>) -> PrintLine {
        PrintLine {
            number,
            text: text.into(),
            spans,
            from: 0,
            to: text.len(),
        }
    }

    #[test]
    fn plain_text_escapes_and_expands_tabs() {
        let rtf = build_rtf(
            &[line(1, "a{b}\\c", vec![]), line(2, "\tx", vec![])],
            None,
            false,
            10.0,
            4,
        );
        assert!(rtf.starts_with("{\\rtf1\\ansi"));
        assert!(rtf.contains("\\fs20"));
        assert!(rtf.contains("{ a\\{b\\}\\\\c}\\par\n{     x}"), "{rtf}");
        assert!(rtf.ends_with('}'));
        assert!(rtf.is_ascii());
    }

    #[test]
    fn unicode_as_utf16_escapes() {
        let rtf = build_rtf(&[line(1, "é😀", vec![])], None, false, 10.0, 4);
        // é = U+00E9; 😀 = D83D DE00 (negative as i16).
        assert!(rtf.contains("\\u233?\\u-10179?\\u-8704?"), "{rtf}");
    }

    #[test]
    fn colors_styles_and_line_numbers() {
        let plain = Style {
            fg: Color32::BLACK,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
        };
        let mut kinds = vec![plain; 64];
        kinds[TokenKind::Comment.index()] = Style {
            fg: Color32::from_rgb(0, 128, 0),
            italic: true,
            ..plain
        };
        let styles = LangStyles {
            kinds,
            keywords: Vec::new(),
        };
        let rtf = build_rtf(
            &[
                line(9, "x // hi", vec![(2, 7, TokenKind::Comment)]),
                line(10, "y", vec![]),
            ],
            Some(&styles),
            true,
            9.0,
            4,
        );
        // Grey numbers first in the colour table, then the comment green.
        assert!(
            rtf.contains("{\\colortbl;\\red138\\green138\\blue138;\\red0\\green128\\blue0;}"),
            "{rtf}"
        );
        assert!(rtf.contains("{\\cf1  9  }{ x }{\\cf2\\i // hi}"), "{rtf}");
        assert!(rtf.contains("{\\cf1 10  }{ y}"), "{rtf}");
    }

    #[test]
    fn part_of_a_line() {
        let mut l = line(1, "\tabc def", vec![]);
        l.from = 2; // "bc def" (after the tab and 'a')
        l.to = 5; // "bc "
        let rtf = build_rtf(&[l], None, false, 10.0, 4);
        assert!(rtf.contains("{ bc }"), "{rtf}");
    }
}
