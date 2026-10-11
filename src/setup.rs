//! Polystation Setup: a small installer that downloads the latest release from GitHub and installs it.
//!
//! It talks to GitHub through WinHTTP (part of Windows), so there is nothing to bundle. The window is drawn
//! with plain GDI. Run with `--silent` for an install without a window, `--dir <folder>` to install
//! elsewhere and `--uninstall` to remove an installed copy. The installed folder keeps a copy of this
//! program as `polystation-setup.exe`: running it again updates to the newest release.
#![windows_subsystem = "windows"]
#![allow(non_snake_case)]

#[path = "json.rs"]
#[allow(dead_code)]
mod json;

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The GitHub repository releases are taken from.
const REPO: &str = "GreenRu/polystation";
const ASSET: &str = "polystation.exe";
const APP: &str = "Polystation";

// ---------------------------------------------------------------------------------------------
// WinHTTP
// ---------------------------------------------------------------------------------------------
#[link(name = "winhttp")]
unsafe extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> isize;
    fn WinHttpConnect(session: isize, host: *const u16, port: u16, reserved: u32) -> isize;
    fn WinHttpOpenRequest(conn: isize, verb: *const u16, path: *const u16, version: *const u16, referrer: *const u16, accept: *const *const u16, flags: u32) -> isize;
    fn WinHttpSendRequest(req: isize, headers: *const u16, headers_len: u32, optional: *const c_void, optional_len: u32, total_len: u32, context: usize) -> i32;
    fn WinHttpReceiveResponse(req: isize, reserved: *mut c_void) -> i32;
    fn WinHttpQueryHeaders(req: isize, level: u32, name: *const u16, buf: *mut c_void, len: *mut u32, index: *mut u32) -> i32;
    fn WinHttpReadData(req: isize, buf: *mut c_void, to_read: u32, read: *mut u32) -> i32;
    fn WinHttpCloseHandle(h: isize) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn split_url(url: &str) -> Result<(String, String), String> {
    let rest = url.strip_prefix("https://").ok_or("only https downloads are supported")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h.to_string(), format!("/{p}"))).unwrap_or((rest.to_string(), "/".into()));
    Ok((host, path))
}

/// Downloads a URL. `progress` is called with (bytes so far, total bytes if known).
fn http_get(url: &str, progress: &mut dyn FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let (host, path) = split_url(url)?;
    unsafe {
        let session = WinHttpOpen(wide("Polystation-Setup").as_ptr(), 0, std::ptr::null(), std::ptr::null(), 0);
        if session == 0 {
            return Err("could not start the network library".into());
        }
        let conn = WinHttpConnect(session, wide(&host).as_ptr(), 443, 0);
        let req = if conn == 0 { 0 } else { WinHttpOpenRequest(conn, wide("GET").as_ptr(), wide(&path).as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0x0080_0000) };
        let close = |h: [isize; 3]| {
            for x in h {
                if x != 0 {
                    WinHttpCloseHandle(x);
                }
            }
        };
        if req == 0 {
            close([session, conn, 0]);
            return Err(format!("could not reach {host}"));
        }
        let headers = wide("Accept: application/vnd.github+json, application/octet-stream\r\n");
        if WinHttpSendRequest(req, headers.as_ptr(), u32::MAX, std::ptr::null(), 0, 0, 0) == 0 || WinHttpReceiveResponse(req, std::ptr::null_mut()) == 0 {
            close([session, conn, req]);
            return Err(format!("could not connect to {host}: check your internet connection"));
        }
        let number = |what: u32| -> u32 {
            let (mut v, mut len) = (0u32, 4u32);
            WinHttpQueryHeaders(req, what | 0x2000_0000, std::ptr::null(), &mut v as *mut u32 as *mut c_void, &mut len, std::ptr::null_mut());
            v
        };
        let status = number(19);
        if status != 200 {
            close([session, conn, req]);
            return Err(match status {
                404 => "no release has been published yet".into(),
                403 => "GitHub refused the request (rate limit?), try again later".into(),
                s => format!("the server answered with error {s}"),
            });
        }
        let total = number(5) as u64;
        let mut data = Vec::with_capacity(total as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            if WinHttpReadData(req, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n) == 0 {
                close([session, conn, req]);
                return Err("the download was interrupted".into());
            }
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n as usize]);
            progress(data.len() as u64, total);
        }
        close([session, conn, req]);
        Ok(data)
    }
}

// ---------------------------------------------------------------------------------------------
// the release
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
struct Release {
    version: String,
    url: String,
    size: u64,
}

fn latest_release() -> Result<Release, String> {
    let body = http_get(&format!("https://api.github.com/repos/{REPO}/releases/latest"), &mut |_, _| {})?;
    parse_release(&String::from_utf8_lossy(&body))
}

fn parse_release(text: &str) -> Result<Release, String> {
    let j = json::parse(text)?;
    let tag = j.str("tag_name").ok_or("the release has no version")?.trim_start_matches('v').to_string();
    for a in j.get("assets").and_then(|a| a.as_arr()).unwrap_or(&[]) {
        if a.str("name") == Some(ASSET) {
            return Ok(Release { version: tag, url: a.str("browser_download_url").ok_or("the release file has no link")?.to_string(), size: a.num("size").unwrap_or(0.0) as u64 });
        }
    }
    Err(format!("the latest release has no {ASSET}"))
}

// ---------------------------------------------------------------------------------------------
// installing
// ---------------------------------------------------------------------------------------------
fn default_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\"));
    base.join("Programs").join(APP)
}

fn run_hidden(cmd: &str, args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(cmd).args(args).creation_flags(0x0800_0000).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

fn powershell(script: &str) -> bool {
    run_hidden("powershell.exe", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
}

fn start_menu() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Microsoft").join("Windows").join("Start Menu").join("Programs").join(format!("{APP}.lnk"))
}

fn shortcut(lnk: &str, target: &Path, dir: &Path) -> bool {
    powershell(&format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut({lnk}); $s.TargetPath='{t}'; $s.WorkingDirectory='{d}'; $s.Description='Polystation: a PS1-style 3D engine'; $s.Save()",
        t = target.display().to_string().replace('\'', "''"),
        d = dir.display().to_string().replace('\'', "''"),
    ))
}

#[derive(Clone)]
struct Options {
    dir: PathBuf,
    desktop: bool,
}

/// Downloads the release and puts everything in place. `status` gets (text, fraction 0..1).
fn install(rel: &Release, opt: &Options, status: &mut dyn FnMut(&str, f32)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&opt.dir).map_err(|e| format!("could not create {}: {e}", opt.dir.display()))?;
    let exe = opt.dir.join("polystation.exe");
    status("Downloading Polystation...", 0.02);
    let data = http_get(&rel.url, &mut |got, total| {
        let total = if total == 0 { rel.size.max(1) } else { total };
        status(&format!("Downloading Polystation...  {:.1} / {:.1} MB", got as f64 / 1.0e6, total as f64 / 1.0e6), 0.02 + 0.8 * (got as f32 / total as f32).min(1.0));
    })?;
    if data.len() < 100_000 || &data[..2] != b"MZ" {
        return Err("the downloaded file is not a program".into());
    }
    status("Installing...", 0.84);
    // a running copy cannot be overwritten, but it can be renamed out of the way
    let old = opt.dir.join("polystation.exe.old");
    let _ = std::fs::remove_file(&old);
    if exe.exists() && std::fs::rename(&exe, &old).is_err() {
        return Err("Polystation is running. Close it and try again.".into());
    }
    std::fs::write(&exe, &data).map_err(|e| format!("could not write {}: {e}", exe.display()))?;
    let _ = std::fs::remove_file(&old);
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let setup = opt.dir.join("polystation-setup.exe");
    if me != setup {
        let _ = std::fs::copy(&me, &setup);
    }
    status("Creating shortcuts...", 0.9);
    let lnk = start_menu();
    if let Some(p) = lnk.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    shortcut(&format!("'{}'", lnk.display()), &exe, &opt.dir);
    if opt.desktop {
        shortcut(&format!("([Environment]::GetFolderPath('Desktop') + '\\{APP}.lnk')"), &exe, &opt.dir);
    }
    status("Registering .polystation files...", 0.95);
    register(&exe, &setup, &opt.dir, &rel.version);
    status("Done", 1.0);
    Ok(exe)
}

fn reg(args: &[&str]) {
    let mut a = vec!["add"];
    a.extend_from_slice(args);
    a.push("/f");
    run_hidden("reg.exe", &a);
}

fn register(exe: &Path, setup: &Path, dir: &Path, version: &str) {
    let (exe, setup, dir) = (exe.display().to_string(), setup.display().to_string(), dir.display().to_string());
    reg(&["HKCU\\Software\\Classes\\.polystation", "/ve", "/d", "Polystation.Project"]);
    reg(&["HKCU\\Software\\Classes\\Polystation.Project", "/ve", "/d", "Polystation Project"]);
    reg(&["HKCU\\Software\\Classes\\Polystation.Project\\DefaultIcon", "/ve", "/d", &format!("{exe},0")]);
    reg(&["HKCU\\Software\\Classes\\Polystation.Project\\shell\\open\\command", "/ve", "/d", &format!("\"{exe}\" \"%1\"")]);
    let key = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Polystation";
    reg(&[key, "/v", "DisplayName", "/d", APP]);
    reg(&[key, "/v", "DisplayVersion", "/d", version]);
    reg(&[key, "/v", "Publisher", "/d", "Polystation"]);
    reg(&[key, "/v", "InstallLocation", "/d", &dir]);
    reg(&[key, "/v", "DisplayIcon", "/d", &exe]);
    reg(&[key, "/v", "UninstallString", "/d", &format!("\"{setup}\" --uninstall")]);
    reg(&[key, "/v", "NoModify", "/t", "REG_DWORD", "/d", "1"]);
    reg(&[key, "/v", "NoRepair", "/t", "REG_DWORD", "/d", "1"]);
}

fn uninstall(dir: &Path) {
    let _ = std::fs::remove_file(start_menu());
    powershell(&format!("Remove-Item -ErrorAction SilentlyContinue ([Environment]::GetFolderPath('Desktop') + '\\{APP}.lnk')"));
    for key in ["HKCU\\Software\\Classes\\.polystation", "HKCU\\Software\\Classes\\Polystation.Project", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Polystation"] {
        run_hidden("reg.exe", &["delete", key, "/f"]);
    }
    let _ = std::fs::remove_file(dir.join("polystation.exe"));
    let _ = std::fs::remove_file(dir.join("polystation.exe.old"));
    // this program is the one still running: let a helper remove the folder once it has exited
    run_hidden("cmd.exe", &["/c", &format!("ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{}\"", dir.display())]);
}

// ---------------------------------------------------------------------------------------------
// the window
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Checking,
    Ready,
    Installing,
    Done,
    Failed(String),
}

struct State {
    stage: Stage,
    release: Option<Release>,
    status: String,
    fraction: f32,
    opt: Options,
    exe: Option<PathBuf>,
    updating: bool,
}

type Shared = Arc<Mutex<State>>;

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}
#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct MSG {
    hwnd: isize,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: POINT,
    lPrivate: u32,
}
#[repr(C)]
struct PAINTSTRUCT {
    hdc: isize,
    erase: i32,
    rc: RECT,
    restore: i32,
    incupdate: i32,
    reserved: [u8; 32],
}
#[repr(C)]
struct WNDCLASSW {
    style: u32,
    proc_: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    inst: isize,
    icon: isize,
    cursor: isize,
    background: isize,
    menu: *const u16,
    class: *const u16,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(wc: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: isize, menu: isize, inst: isize, param: *mut c_void) -> isize;
    fn DefWindowProcW(h: isize, m: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(msg: *mut MSG, h: isize, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn PostQuitMessage(code: i32);
    fn LoadCursorW(inst: isize, name: usize) -> isize;
    fn BeginPaint(h: isize, ps: *mut PAINTSTRUCT) -> isize;
    fn EndPaint(h: isize, ps: *const PAINTSTRUCT) -> i32;
    fn GetClientRect(h: isize, r: *mut RECT) -> i32;
    fn InvalidateRect(h: isize, r: *const RECT, erase: i32) -> i32;
    fn SetTimer(h: isize, id: usize, ms: u32, f: *const c_void) -> usize;
    fn FillRect(dc: isize, r: *const RECT, brush: isize) -> i32;
    fn DrawTextW(dc: isize, text: *const u16, n: i32, r: *mut RECT, flags: u32) -> i32;
    fn SetProcessDPIAware() -> i32;
    fn GetDpiForSystem() -> u32;
    fn AdjustWindowRect(r: *mut RECT, style: u32, menu: i32) -> i32;
    fn MessageBoxW(h: isize, text: *const u16, caption: *const u16, flags: u32) -> i32;
    fn GetSystemMetrics(i: i32) -> i32;
    fn SetCapture(h: isize) -> isize;
    fn ReleaseCapture() -> i32;
    fn DestroyWindow(h: isize) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: isize) -> isize;
    fn CreateCompatibleBitmap(dc: isize, w: i32, h: i32) -> isize;
    fn SelectObject(dc: isize, obj: isize) -> isize;
    fn DeleteObject(obj: isize) -> i32;
    fn DeleteDC(dc: isize) -> i32;
    fn BitBlt(dst: isize, x: i32, y: i32, w: i32, h: i32, src: isize, sx: i32, sy: i32, rop: u32) -> i32;
    fn CreateSolidBrush(color: u32) -> isize;
    fn SetBkMode(dc: isize, mode: i32) -> i32;
    fn SetTextColor(dc: isize, color: u32) -> u32;
    fn CreateFontW(h: i32, w: i32, esc: i32, orient: i32, weight: i32, italic: u32, underline: u32, strike: u32, charset: u32, out: u32, clip: u32, quality: u32, pitch: u32, face: *const u16) -> isize;
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(h: isize, attr: u32, data: *const c_void, size: u32) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
}

thread_local! {
    static SHARED: std::cell::RefCell<Option<Shared>> = const { std::cell::RefCell::new(None) };
    static UI: std::cell::RefCell<Ui> = std::cell::RefCell::new(Ui::default());
}

#[derive(Default)]
struct Ui {
    scale: f32,
    hover: Option<usize>,
    pressed: Option<usize>,
    buttons: Vec<(usize, [i32; 4])>,
    check: [i32; 4],
}

fn rgb(hex: u32) -> u32 {
    ((hex & 0xFF) << 16) | (hex & 0xFF00) | ((hex >> 16) & 0xFF)
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

unsafe fn fill(dc: isize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let b = unsafe { CreateSolidBrush(rgb(color)) };
    let r = rect(x, y, w, h);
    unsafe {
        FillRect(dc, &r, b);
        DeleteObject(b);
    }
}

unsafe fn text(dc: isize, font: isize, s: &str, x: i32, y: i32, w: i32, h: i32, color: u32, flags: u32) {
    unsafe {
        SelectObject(dc, font);
        SetTextColor(dc, rgb(color));
        let mut r = rect(x, y, w, h);
        let t = wide(s);
        DrawTextW(dc, t.as_ptr(), -1, &mut r, flags | 0x0000_0800 | 0x0000_8000); // DT_NOPREFIX | DT_END_ELLIPSIS
    }
}

const DT_LEFT: u32 = 0;
const DT_CENTER: u32 = 1;
const DT_VCENTER: u32 = 4;
const DT_SINGLE: u32 = 0x20;
const DT_WRAP: u32 = 0;

unsafe fn paint(hwnd: isize) {
    let Some(shared) = SHARED.with(|s| s.borrow().clone()) else { return };
    let st = shared.lock().map(|s| (s.stage.clone(), s.release.clone(), s.status.clone(), s.fraction, s.opt.clone(), s.updating)).ok();
    let Some((stage, release, status, fraction, opt, updating)) = st else { return };
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    let mut cr = rect(0, 0, 0, 0);
    unsafe { GetClientRect(hwnd, &mut cr) };
    let (w, h) = (cr.right, cr.bottom);
    let dc = unsafe { CreateCompatibleDC(hdc) };
    let bmp = unsafe { CreateCompatibleBitmap(hdc, w, h) };
    let old = unsafe { SelectObject(dc, bmp) };
    unsafe { SetBkMode(dc, 1) };
    let s = UI.with(|u| u.borrow().scale);
    let px = |v: f32| (v * s).round() as i32;
    let face = wide("Segoe UI");
    let mk = |size: f32, bold: bool| unsafe { CreateFontW(-px(size), 0, 0, 0, if bold { 700 } else { 400 }, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) };
    let (f_body, f_bold, f_small) = (mk(14.0, false), mk(14.0, true), mk(12.0, false));
    let f_logo = unsafe { CreateFontW(-px(34.0), 0, 0, 0, 700, 0, 0, 0, 1, 0, 0, 3, 0, wide("Terminal").as_ptr()) };

    unsafe {
        fill(dc, 0, 0, w, h, 0x1e1f24);
        fill(dc, 0, 0, w, px(118.0), 0x17181c);
        fill(dc, 0, px(118.0), w, 1, 0x34363e);
        text(dc, f_logo, "POLY", px(36.0), px(18.0), px(200.0), px(44.0), 0xf2f2ff, DT_LEFT | DT_SINGLE);
        text(dc, f_logo, "STATION", px(36.0) + px(104.0), px(18.0), px(300.0), px(44.0), 0x66a0ff, DT_LEFT | DT_SINGLE);
        text(dc, f_body, if updating { "Update" } else { "Setup" }, px(38.0), px(66.0), px(300.0), px(24.0), 0x9a9ea8, DT_LEFT | DT_SINGLE);
        text(dc, f_small, "a PS1-style 3D engine", px(38.0), px(88.0), px(300.0), px(20.0), 0x6c707a, DT_LEFT | DT_SINGLE);
    }

    let mut buttons: Vec<(usize, [i32; 4])> = Vec::new();
    let mut check = [0; 4];
    let (cx, cw) = (px(36.0), w - px(72.0));
    unsafe {
        match &stage {
            Stage::Checking => {
                text(dc, f_bold, "Looking for the latest version...", cx, px(150.0), cw, px(26.0), 0xe4e6eb, DT_LEFT | DT_SINGLE);
                text(dc, f_small, "Contacting github.com", cx, px(178.0), cw, px(22.0), 0x6c707a, DT_LEFT | DT_SINGLE);
                bar(dc, cx, px(220.0), cw, px(14.0), ((std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) % 1200) as f32) / 1200.0, true);
            }
            Stage::Ready => {
                let v = release.as_ref().map_or(String::new(), |r| r.version.clone());
                text(dc, f_bold, &format!("Polystation {v}"), cx, px(142.0), cw, px(26.0), 0xe4e6eb, DT_LEFT | DT_SINGLE);
                text(dc, f_body, "is ready to install.", cx, px(170.0), cw, px(24.0), 0x9a9ea8, DT_LEFT | DT_SINGLE);
                text(dc, f_small, "Install location", cx, px(212.0), cw, px(20.0), 0x6c707a, DT_LEFT | DT_SINGLE);
                fill(dc, cx, px(234.0), cw, px(32.0), 0x2a2b32);
                text(dc, f_body, &opt.dir.display().to_string(), cx + px(10.0), px(234.0), cw - px(20.0), px(32.0), 0xe4e6eb, DT_LEFT | DT_VCENTER | DT_SINGLE);
                check = [cx, px(282.0), cw, px(24.0)];
                fill(dc, cx, px(285.0), px(18.0), px(18.0), if opt.desktop { 0x4d8eff } else { 0x2a2b32 });
                if opt.desktop {
                    text(dc, f_bold, "x", cx, px(285.0), px(18.0), px(18.0), 0xffffff, DT_CENTER | DT_VCENTER | DT_SINGLE);
                }
                text(dc, f_body, "Create a desktop shortcut", cx + px(28.0), px(282.0), cw, px(24.0), 0xe4e6eb, DT_LEFT | DT_VCENTER | DT_SINGLE);
                buttons.push((1, [w - px(36.0) - px(150.0), h - px(64.0), px(150.0), px(40.0)]));
                buttons.push((2, [w - px(36.0) - px(150.0) - px(110.0), h - px(64.0), px(100.0), px(40.0)]));
            }
            Stage::Installing => {
                text(dc, f_bold, if updating { "Updating Polystation" } else { "Installing Polystation" }, cx, px(150.0), cw, px(26.0), 0xe4e6eb, DT_LEFT | DT_SINGLE);
                text(dc, f_body, &status, cx, px(184.0), cw, px(24.0), 0x9a9ea8, DT_LEFT | DT_SINGLE);
                bar(dc, cx, px(226.0), cw, px(18.0), fraction, false);
                text(dc, f_small, &format!("{:.0}%", fraction * 100.0), cx, px(252.0), cw, px(20.0), 0x6c707a, DT_LEFT | DT_SINGLE);
            }
            Stage::Done => {
                text(dc, f_bold, "Polystation is installed", cx, px(150.0), cw, px(26.0), 0x7ec88a, DT_LEFT | DT_SINGLE);
                text(dc, f_body, "Start it from the Start menu, or open a .polystation project file.", cx, px(184.0), cw, px(24.0), 0x9a9ea8, DT_LEFT | DT_SINGLE);
                text(dc, f_small, &opt.dir.display().to_string(), cx, px(214.0), cw, px(20.0), 0x6c707a, DT_LEFT | DT_SINGLE);
                buttons.push((3, [w - px(36.0) - px(170.0), h - px(64.0), px(170.0), px(40.0)]));
                buttons.push((4, [w - px(36.0) - px(170.0) - px(110.0), h - px(64.0), px(100.0), px(40.0)]));
            }
            Stage::Failed(msg) => {
                text(dc, f_bold, "Something went wrong", cx, px(150.0), cw, px(26.0), 0xff7b72, DT_LEFT | DT_SINGLE);
                let mut r = rect(cx, px(184.0), cw, px(90.0));
                let t = wide(msg);
                SelectObject(dc, f_body);
                SetTextColor(dc, rgb(0xe4e6eb));
                DrawTextW(dc, t.as_ptr(), -1, &mut r, DT_WRAP | 0x10);
                buttons.push((5, [w - px(36.0) - px(120.0), h - px(64.0), px(120.0), px(40.0)]));
                buttons.push((4, [w - px(36.0) - px(120.0) - px(110.0), h - px(64.0), px(100.0), px(40.0)]));
            }
        }
        // the buttons
        let (hover, pressed) = UI.with(|u| (u.borrow().hover, u.borrow().pressed));
        for (id, b) in &buttons {
            let primary = matches!(id, 1 | 3 | 5);
            let lit = hover == Some(*id);
            let down = pressed == Some(*id) && lit;
            let base = if primary { 0x4d8eff } else { 0x32343c };
            let face_c = if down { if primary { 0x3972d8 } else { 0x27282f } } else if lit { if primary { 0x6ca3ff } else { 0x42454f } } else { base };
            fill(dc, b[0], b[1], b[2], b[3], face_c);
            // a one pixel bevel, like the engine's blocks
            fill(dc, b[0], b[1], b[2], 1, if primary { 0x8fbaff } else { 0x5a5d68 });
            fill(dc, b[0], b[1] + b[3] - 1, b[2], 1, if primary { 0x2a5bb8 } else { 0x1d1e23 });
            let label = match id {
                1 => "Install",
                2 | 4 => if matches!(stage, Stage::Done) { "Close" } else { "Cancel" },
                3 => "Launch Polystation",
                _ => "Try again",
            };
            text(dc, f_bold, label, b[0], b[1], b[2], b[3], 0xffffff, DT_CENTER | DT_VCENTER | DT_SINGLE);
        }
        BitBlt(hdc, 0, 0, w, h, dc, 0, 0, 0x00CC0020);
        SelectObject(dc, old);
        for f in [f_body, f_bold, f_small, f_logo] {
            DeleteObject(f);
        }
        DeleteObject(bmp);
        DeleteDC(dc);
        EndPaint(hwnd, &ps);
    }
    UI.with(|u| {
        let mut u = u.borrow_mut();
        u.buttons = buttons;
        u.check = check;
    });
}

/// A flat progress bar made of blocks (or a sliding block while the length is not known).
unsafe fn bar(dc: isize, x: i32, y: i32, w: i32, h: i32, f: f32, indeterminate: bool) {
    unsafe {
        fill(dc, x, y, w, h, 0x0e0f12);
        fill(dc, x, y, w, 1, 0x34363e);
        let (bx, bw) = if indeterminate { (x + ((w as f32 - w as f32 * 0.25) * f) as i32, (w as f32 * 0.25) as i32) } else { (x, (w as f32 * f.clamp(0.0, 1.0)) as i32) };
        let block = (h - 4).max(4);
        let mut p = bx + 2;
        while p + block - 2 <= bx + bw {
            fill(dc, p, y + 2, block - 2, h - 4, 0x4d8eff);
            fill(dc, p, y + 2, block - 2, ((h - 4) as f32 * 0.35) as i32, 0x8fbaff);
            p += block;
        }
    }
}

unsafe extern "system" fn proc_(hwnd: isize, m: u32, w: usize, l: isize) -> isize {
    let hit = |x: i32, y: i32| UI.with(|u| u.borrow().buttons.iter().find(|(_, b)| x >= b[0] && x < b[0] + b[2] && y >= b[1] && y < b[1] + b[3]).map(|(i, _)| *i));
    let pos = |l: isize| ((l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
    match m {
        0x000F => unsafe { paint(hwnd) },
        0x0014 => return 1,
        0x0113 => {
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
        }
        0x0200 => {
            let (x, y) = pos(l);
            let h = hit(x, y);
            let changed = UI.with(|u| std::mem::replace(&mut u.borrow_mut().hover, h) != h);
            if changed {
                unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
            }
            let over = h.is_some() || UI.with(|u| {
                let c = u.borrow().check;
                x >= c[0] && x < c[0] + c[2] && y >= c[1] && y < c[1] + c[3] && c[2] > 0
            });
            unsafe { LoadCursorW(0, if over { 32649 } else { 32512 }) };
        }
        0x0084 => return unsafe { DefWindowProcW(hwnd, m, w, l) },
        0x0020 => {
            let over = UI.with(|u| u.borrow().hover.is_some());
            unsafe { windows_set_cursor(if over { 32649 } else { 32512 }) };
            return 1;
        }
        0x0201 => {
            let (x, y) = pos(l);
            unsafe { SetCapture(hwnd) };
            UI.with(|u| u.borrow_mut().pressed = hit(x, y));
            let c = UI.with(|u| u.borrow().check);
            if c[2] > 0 && x >= c[0] && x < c[0] + c[2] && y >= c[1] && y < c[1] + c[3] {
                if let Some(s) = SHARED.with(|s| s.borrow().clone()) {
                    if let Ok(mut st) = s.lock() {
                        if st.stage == Stage::Ready {
                            st.opt.desktop = !st.opt.desktop;
                        }
                    }
                }
            }
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
        }
        0x0202 => {
            let (x, y) = pos(l);
            unsafe { ReleaseCapture() };
            let down = UI.with(|u| u.borrow_mut().pressed.take());
            if let (Some(d), Some(u)) = (down, hit(x, y)) {
                if d == u {
                    click(hwnd, d);
                }
            }
            unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
        }
        0x0010 => unsafe { DestroyWindow(hwnd); },
        0x0002 => unsafe { PostQuitMessage(0) },
        _ => return unsafe { DefWindowProcW(hwnd, m, w, l) },
    }
    0
}

unsafe fn windows_set_cursor(id: usize) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetCursor(c: isize) -> isize;
    }
    unsafe { SetCursor(LoadCursorW(0, id)) };
}

fn click(hwnd: isize, id: usize) {
    let Some(shared) = SHARED.with(|s| s.borrow().clone()) else { return };
    match id {
        1 => start_install(shared),
        2 | 4 => unsafe {
            DestroyWindow(hwnd);
        },
        3 => {
            if let Some(exe) = shared.lock().ok().and_then(|s| s.exe.clone()) {
                let _ = std::process::Command::new(exe).spawn();
            }
            unsafe {
                DestroyWindow(hwnd);
            }
        }
        5 => {
            if let Ok(mut s) = shared.lock() {
                s.stage = Stage::Checking;
            }
            start_check(shared);
        }
        _ => {}
    }
}

fn start_check(shared: Shared) {
    std::thread::spawn(move || {
        let r = latest_release();
        if let Ok(mut s) = shared.lock() {
            match r {
                Ok(rel) => {
                    s.release = Some(rel);
                    s.stage = Stage::Ready;
                }
                Err(e) => s.stage = Stage::Failed(e),
            }
        }
    });
}

fn start_install(shared: Shared) {
    let (rel, opt) = {
        let Ok(mut s) = shared.lock() else { return };
        let Some(rel) = s.release.clone() else { return };
        s.stage = Stage::Installing;
        s.fraction = 0.0;
        (rel, s.opt.clone())
    };
    std::thread::spawn(move || {
        let sh = shared.clone();
        let result = install(&rel, &opt, &mut |text, f| {
            if let Ok(mut s) = sh.lock() {
                s.status = text.to_string();
                s.fraction = f;
            }
        });
        if let Ok(mut s) = shared.lock() {
            match result {
                Ok(exe) => {
                    s.exe = Some(exe);
                    s.stage = Stage::Done;
                }
                Err(e) => s.stage = Stage::Failed(e),
            }
        }
    });
}

fn run_window(opt: Options) {
    unsafe {
        SetProcessDPIAware();
        let scale = GetDpiForSystem() as f32 / 96.0;
        UI.with(|u| u.borrow_mut().scale = scale);
        let updating = opt.dir.join("polystation.exe").exists();
        let shared: Shared = Arc::new(Mutex::new(State { stage: Stage::Checking, release: None, status: String::new(), fraction: 0.0, opt, exe: None, updating }));
        SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
        start_check(shared);
        let inst = GetModuleHandleW(std::ptr::null());
        let class = wide("PolystationSetup");
        let wc = WNDCLASSW { style: 0, proc_, cls_extra: 0, wnd_extra: 0, inst, icon: 0, cursor: LoadCursorW(0, 32512), background: 0, menu: std::ptr::null(), class: class.as_ptr() };
        RegisterClassW(&wc);
        let style = 0x00C0_0000 | 0x0008_0000 | 0x0002_0000; // CAPTION | SYSMENU | MINIMIZEBOX
        let (cw, ch) = ((560.0 * scale) as i32, (400.0 * scale) as i32);
        let mut r = rect(0, 0, cw, ch);
        AdjustWindowRect(&mut r, style, 0);
        let (ww, wh) = (r.right - r.left, r.bottom - r.top);
        let (sx, sy) = (GetSystemMetrics(0), GetSystemMetrics(1));
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("Polystation Setup").as_ptr(), style | 0x1000_0000, (sx - ww) / 2, (sy - wh) / 2 - 20, ww, wh, 0, 0, inst, std::ptr::null_mut());
        let dark: u32 = 1;
        DwmSetWindowAttribute(hwnd, 20, &dark as *const u32 as *const c_void, 4);
        for (attr, color) in [(35u32, 0x0017_1817u32), (36, 0x00EB_E6E4)] {
            DwmSetWindowAttribute(hwnd, attr, &color as *const u32 as *const c_void, 4);
        }
        SetTimer(hwnd, 1, 33, std::ptr::null());
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn message(text: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(0, wide(text).as_ptr(), wide("Polystation Setup").as_ptr(), flags) }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opt = Options { dir: default_dir(), desktop: false };
    if let Some(i) = args.iter().position(|a| a == "--dir") {
        if let Some(d) = args.get(i + 1) {
            opt.dir = PathBuf::from(d);
        }
    }
    // an installed copy of this program (polystation-setup.exe next to polystation.exe) updates in place
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        // (only the installed copy: a freshly downloaded setup next to some other polystation.exe must not touch it)
        if dir.join("polystation.exe").exists() && dir == default_dir() && !args.iter().any(|a| a == "--dir") {
            opt.dir = dir;
        }
    }
    if args.iter().any(|a| a == "--uninstall") {
        if message("Remove Polystation from this computer?\n\nYour projects are not touched.", 0x24) == 6 {
            uninstall(&opt.dir);
            message("Polystation was removed.", 0x40);
        }
        return;
    }
    if args.iter().any(|a| a == "--silent") {
        let code = match latest_release().and_then(|r| install(&r, &Options { desktop: false, ..opt.clone() }, &mut |_, _| {})) {
            Ok(_) => 0,
            Err(_) => 1,
        };
        std::process::exit(code);
    }
    run_window(opt);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_split_and_only_https_is_allowed() {
        assert_eq!(split_url("https://github.com/a/b/releases/download/v1/x.exe").unwrap(), ("github.com".into(), "/a/b/releases/download/v1/x.exe".into()));
        assert!(split_url("http://example.com/x").is_err());
    }

    #[test]
    fn the_release_json_is_read() {
        let j = r#"{"tag_name":"v0.2.1","assets":[{"name":"notes.txt","browser_download_url":"https://x/n","size":3},{"name":"polystation.exe","browser_download_url":"https://github.com/o/r/releases/download/v0.2.1/polystation.exe","size":2500000}]}"#;
        let r = parse_release(j).unwrap();
        assert_eq!((r.version.as_str(), r.size), ("0.2.1", 2_500_000));
        assert!(r.url.ends_with("/polystation.exe"));
        assert!(parse_release(r#"{"tag_name":"v1","assets":[]}"#).is_err());
        assert!(parse_release("not json").is_err());
    }
}
