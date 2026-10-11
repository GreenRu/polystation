//! Platform layer: the window, the OpenGL context, input, the clipboard, sound and the DPI scale.
//!
//! The user interface itself (every panel, button, text box and menu) is drawn by `ui`; from Windows
//! only the window with its events, a font (see `ui::font`), the clipboard and the sound player are
//! used. The window is one OpenGL surface: the 3D view is drawn into a rectangle of it and the UI
//! around and over it.
#![allow(non_snake_case, non_camel_case_types)]

use crate::gl::Gl;
use crate::ui::settings_dialog::{ItemSpec, SettingsDialog};
use crate::ui::dialogs::{ColorDialog, Dialog, DlgEvent, FileDialog, MessageDialog};
use crate::ui::font::Font;
use crate::ui::render::Renderer;
use crate::ui::style::{Ctx, Fonts};
pub use crate::ui::tree::DropAt;
pub use crate::ui::MenuItem;
use crate::ui::widgets::{Field, Highlight, Kind, Widget};
use crate::ui::{Cursor, Mods, Route, Ui};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;

pub type HWND = isize;
pub type HDC = isize;
pub type HGLRC = isize;

#[repr(C)]
struct WNDCLASSW {
    style: u32,
    lpfnWndProc: unsafe extern "system" fn(HWND, u32, usize, isize) -> isize,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: isize,
    hIcon: isize,
    hCursor: isize,
    hbrBackground: isize,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wParam: usize,
    lParam: isize,
    time: u32,
    pt: POINT,
    lPrivate: u32,
}

#[repr(C)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MINMAXINFO {
    reserved: POINT,
    max_size: POINT,
    max_position: POINT,
    min_track: POINT,
    max_track: POINT,
}

#[repr(C)]
struct TRACKMOUSEEVENT {
    cb_size: u32,
    flags: u32,
    hwnd: HWND,
    hover_time: u32,
}

#[repr(C)]
struct PIXELFORMATDESCRIPTOR {
    nSize: u16,
    nVersion: u16,
    dwFlags: u32,
    iPixelType: u8,
    cColorBits: u8,
    cRedBits: u8,
    cRedShift: u8,
    cGreenBits: u8,
    cGreenShift: u8,
    cBlueBits: u8,
    cBlueShift: u8,
    cAlphaBits: u8,
    cAlphaShift: u8,
    cAccumBits: u8,
    cAccumRedBits: u8,
    cAccumGreenBits: u8,
    cAccumBlueBits: u8,
    cAccumAlphaBits: u8,
    cDepthBits: u8,
    cStencilBits: u8,
    cAuxBuffers: u8,
    iLayerType: u8,
    bReserved: u8,
    dwLayerMask: u32,
    dwVisibleMask: u32,
    dwDamageMask: u32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn LoadLibraryA(name: *const u8) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> *const c_void;
    fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
    fn GlobalLock(mem: isize) -> *mut c_void;
    fn GlobalUnlock(mem: isize) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(wc: *const WNDCLASSW) -> u16;
    fn CreateWindowExW(ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: isize, menu: isize, inst: isize, param: *mut c_void) -> HWND;
    fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> isize;
    fn PeekMessageW(msg: *mut MSG, h: HWND, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(msg: *const MSG) -> i32;
    fn DispatchMessageW(msg: *const MSG) -> isize;
    fn PostQuitMessage(code: i32);
    fn LoadCursorW(inst: isize, name: usize) -> isize;
    fn GetDC(h: HWND) -> HDC;
    fn GetClientRect(h: HWND, r: *mut RECT) -> i32;
    fn SetCapture(h: HWND) -> HWND;
    fn ReleaseCapture() -> i32;
    fn SetCursor(c: isize) -> isize;
    fn SetProcessDPIAware() -> i32;
    fn GetDpiForSystem() -> u32;
    fn SystemParametersInfoW(action: u32, param: u32, data: *mut c_void, winini: u32) -> i32;
    fn SetWindowTextW(h: HWND, text: *const u16) -> i32;
    fn MessageBoxW(h: HWND, text: *const u16, caption: *const u16, flags: u32) -> i32;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetCursorPos(p: *mut POINT) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn ShowCursor(show: i32) -> i32;
    fn ScreenToClient(h: HWND, p: *mut POINT) -> i32;
    fn ClientToScreen(h: HWND, p: *mut POINT) -> i32;
    fn TrackMouseEvent(t: *mut TRACKMOUSEEVENT) -> i32;
    fn ValidateRect(h: HWND, rc: *const RECT) -> i32;
    fn OpenClipboard(h: HWND) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn GetClipboardData(format: u32) -> isize;
    fn SetClipboardData(format: u32, mem: isize) -> isize;
    fn ShowWindow(h: HWND, cmd: i32) -> i32;
    fn GetWindowLongPtrW(h: HWND, index: i32) -> isize;
    fn SetWindowLongPtrW(h: HWND, index: i32, value: isize) -> isize;
    fn SetWindowPos(h: HWND, after: isize, x: i32, y: i32, w: i32, h2: i32, flags: u32) -> i32;
    fn GetWindowRect(h: HWND, r: *mut RECT) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(ms: u32) -> u32;
    fn PlaySoundW(sound: *const u8, module: isize, flags: u32) -> i32;
}

#[link(name = "dwmapi")]
unsafe extern "system" {
    fn DwmSetWindowAttribute(h: HWND, attr: u32, data: *const c_void, size: u32) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn ChoosePixelFormat(dc: HDC, pfd: *const PIXELFORMATDESCRIPTOR) -> i32;
    fn SetPixelFormat(dc: HDC, fmt: i32, pfd: *const PIXELFORMATDESCRIPTOR) -> i32;
    fn SwapBuffers(dc: HDC) -> i32;
}

#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglCreateContext(dc: HDC) -> HGLRC;
    fn wglMakeCurrent(dc: HDC, rc: HGLRC) -> i32;
    fn wglDeleteContext(rc: HGLRC) -> i32;
    fn wglGetProcAddress(name: *const u8) -> *const c_void;
}

/// Input gathered each frame. Mouse coordinates are relative to the 3D view.
#[derive(Clone, Copy, Default)]
pub struct Input {
    pub mouse: (i32, i32),
    pub left_down: bool,
    pub right_down: bool,
    pub wheel: f32,
    pub quit: bool,
    /// One-shot (reset after each poll): where the left button went down / up on the 3D view.
    pub left_pressed: Option<(i32, i32)>,
    pub left_released: Option<(i32, i32)>,
    /// Double-click inside the hierarchy (one-shot).
    pub tree_dbl_click: bool,
    /// Double-click on an item of the file explorer (one-shot).
    pub explorer_dbl_click: bool,
    /// Right click on the hierarchy / the file list (one-shot): window position; the item under it is selected.
    pub tree_rclick: Option<(i32, i32)>,
    pub explorer_rclick: Option<(i32, i32)>,
}

/// What the window procedure saw; `Window::poll` turns these into UI and application input.
enum Raw {
    Move(f32, f32),
    Down(u8, f32, f32, bool),
    Up(u8, f32, f32),
    Wheel(f32, f32, f32),
    Key(u32, bool),
    Char(char),
    Resize(f32, f32),
    Leave,
    Close,
}

thread_local! {
    static INPUT: Cell<Input> = Cell::new(Input::default());
    static RAW: RefCell<Vec<Raw>> = const { RefCell::new(Vec::new()) };
    /// Commands posted to the window by other programs / the test scripts (WM_COMMAND).
    static POSTED: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    static MAIN: Cell<HWND> = const { Cell::new(0) };
    static LOOK_ANCHOR: Cell<(i32, i32)> = Cell::new((0, 0));
    /// True while play mode runs: editor shortcuts are off and Esc stops playing.
    static PLAYING: Cell<bool> = const { Cell::new(false) };
    /// System DPI / 96: layout metrics are in 96-dpi units and scaled by this.
    static SCALE: Cell<f32> = Cell::new(1.0);
    static CURSOR: Cell<Cursor> = const { Cell::new(Cursor::Arrow) };
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static HIGH_SURROGATE: Cell<u16> = const { Cell::new(0) };
    static LAST_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static LOOKING: Cell<bool> = const { Cell::new(false) };
    /// Window style and place saved while in full screen.
    static SAVED_FRAME: Cell<Option<(isize, (i32, i32, i32, i32))>> = const { Cell::new(None) };
    /// Interface size in percent chosen in the settings (0 = follow Windows).
    static SCALE_OVERRIDE: Cell<u32> = const { Cell::new(0) };
}

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_CLOSE: u32 = 0x0010;
const WM_PAINT: u32 = 0x000F;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SETCURSOR: u32 = 0x0020;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_COMMAND: u32 = 0x0111;
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_RBUTTONDBLCLK: u32 = 0x0206;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_KILLFOCUS: u32 = 0x0008;

// ---- commands (menu items, accelerators, panel buttons) -------------------------------------
pub const CMD_OPEN_PROJECT: u16 = 1001;
pub const CMD_SAVE: u16 = 1002;
pub const CMD_SAVE_AS: u16 = 1003;
pub const CMD_IMPORT: u16 = 1004;
pub const CMD_EXIT: u16 = 1005;
pub const CMD_SETTINGS: u16 = 1007;
pub const CMD_COMPILE: u16 = 1008;
pub const CMD_GUI_EDITOR: u16 = 1009;
pub const CMD_NEW_PROJECT: u16 = 1011;
pub const CMD_START_PAGE: u16 = 1012;
pub const CMD_UPDATE: u16 = 1013;
pub const CMD_UNINSTALL: u16 = 1014;
pub const CMD_SCREENSHOT: u16 = 1006;
pub const CMD_RESET_CAMERA: u16 = 1010;
pub const CMD_UNDO: u16 = 1020;
pub const CMD_REDO: u16 = 1021;
pub const CMD_DELETE: u16 = 1022;
pub const CMD_DUPLICATE: u16 = 1023;
pub const CMD_HIDE_SEL: u16 = 1024;
pub const CMD_SHOW_ALL: u16 = 1025;
/// `CMD_ADD_BASE + n` adds primitive number n.
pub const CMD_ADD_BASE: u16 = 1300;
/// Gameplay object n (see the prefabs module): 1330 + n.
pub const CMD_PREFAB_BASE: u16 = 1330;
/// `CMD_MODE_BASE + n` switches to display mode n.
pub const CMD_MODE_BASE: u16 = 1040;
pub const MODE_COUNT: u16 = 6;
pub const CMD_AFFINE: u16 = 1050;
pub const CMD_GRID: u16 = 1051;
pub const CMD_CULL: u16 = 1052;
pub const CMD_FRAME_SEL: u16 = 1053;
pub const CMD_FRAME_ALL: u16 = 1054;
/// `CMD_FOG_BASE + n` selects fog mode n (0 off, 1 smooth, 2 dithered).
pub const CMD_FOG_BASE: u16 = 1070;
pub const FOG_MODES: u16 = 3;
pub const CMD_FOG_NEARER: u16 = 1073;
pub const CMD_FOG_FARTHER: u16 = 1074;
pub const CMD_DITHER_FINER: u16 = 1075;
pub const CMD_DITHER_CHUNKIER: u16 = 1076;
/// `CMD_GIZMO_BASE + n` selects the gizmo: 0 none, 1 move, 2 rotate, 3 scale.
pub const CMD_GIZMO_BASE: u16 = 1080;
pub const CMD_COLOR: u16 = 1060;
pub const CMD_CLEAR_COLOR: u16 = 1061;
pub const CMD_RESET_XFORM: u16 = 1062;
/// Placeholder: the custom language and player will hook in here.
pub const CMD_PLAY: u16 = 1100;
pub const CMD_EXPLORER_UP: u16 = 1120;
pub const CMD_EXPLORER_REFRESH: u16 = 1121;
pub const ST_EXPLORER_PATH: u16 = 3103;
pub const LV_EXPLORER: u16 = 3104;
pub const ST_TEXTURE: u16 = 3105;
pub const CMD_TEXTURE_SET: u16 = 1140;
pub const CMD_TEXTURE_CLEAR: u16 = 1141;

// ---- particle panel ids -----------------------------------------------------------------------
/// Velocity x,y,z = 3080..=3082.
pub const ED_PVEL0: u16 = 3080;
pub const ED_PGRAV: u16 = 3083;
pub const ED_PSIZE: u16 = 3084;
pub const ED_PAMOUNT: u16 = 3085;
pub const ED_PBOX: u16 = 3086;
pub const ED_PLIFE: u16 = 3087;
pub const ST_PTEXTURE: u16 = 3106;

// ---- scatter panel ids ------------------------------------------------------------------------
pub const ED_SCOUNT: u16 = 3090;
pub const ED_SMIN: u16 = 3091;
pub const ED_SMAX: u16 = 3092;
pub const ED_SSEED: u16 = 3093;
pub const ED_SSLOPE: u16 = 3094;
pub const ED_SSINK: u16 = 3095;
pub const CB_SSHAPE: u16 = 3075;
pub const ST_STARGET: u16 = 3107;
pub const CMD_PLAYER_SOUND: u16 = 1160;
pub const CMD_PLAYER_SOUND_CLEAR: u16 = 1161;
/// Player panel edit boxes: 15 consecutive ids.
pub const ED_PL0: u16 = 3110;
pub const ED_PL_LAST: u16 = 3125;
pub const ST_PLSOUND: u16 = 3108;
pub const ED_PL_FOV: u16 = 3118;
pub const CMD_SCATTER_ADD: u16 = 1150;
pub const CMD_TAB_PROJECT: u16 = 1172;
pub const CMD_TAB_SCRIPT: u16 = 1173;
pub const CMD_ADD_MENU: u16 = 1174;
pub const CMD_TAB_GUIDE: u16 = 1178;
pub const CMD_HELP_GUIDE: u16 = 1179;
/// Example script n of the Help menu: 1180 + n (7 of them).
pub const CMD_EXAMPLE_BASE: u16 = 1180;
pub const CMD_EXAMPLE_COUNT: u16 = crate::guide::EXAMPLES.len() as u16;
/// Collision mode n (solid, passable, trigger) of the selected objects: 1190 + n.
pub const CMD_COLL_BASE: u16 = 1210;
pub const CB_COLLISION: u16 = 3076;
pub const CB_LKIND: u16 = 3077;
pub const CB_TKIND: u16 = 3078;
pub const CB_SSOURCE: u16 = 3079;
pub const ED_TRUNK: u16 = 3150;
pub const ED_TRWIDTH: u16 = 3151;
pub const ED_CROWN: u16 = 3152;
pub const ED_DETAIL: u16 = 3153;
pub const ED_TRSEED: u16 = 3154;
pub const ST_TRINFO: u16 = 3155;
pub const CMD_TREE_RANDOMIZE: u16 = 1152;
pub const ED_LINT: u16 = 3140;
pub const ED_LRANGE: u16 = 3141;
pub const ED_LANGLE: u16 = 3142;
pub const ST_LINFO: u16 = 3143;
pub const CMD_ENV_AMBIENT: u16 = 1096;
pub const ED_GUIDE: u16 = 3132;
pub const ST_COLLISION_HINT: u16 = 3133;
pub const CMD_SCRIPT_ADD: u16 = 1175;
pub const CMD_EXPLORER_OPEN: u16 = 1176;
pub const CMD_SCRIPT_REMOVE: u16 = 1177;
pub const ED_SCRIPT: u16 = 3130;
pub const ST_SCRIPT_INFO: u16 = 3131;
/// Every section heading in the inspector uses this id (they are painted in the heading colour).
pub const ST_SECTION: u16 = 3200;
pub const CMD_SCATTER_RANDOMIZE: u16 = 1151;
pub const CMD_SELECT_ALL: u16 = 1101;
pub const CMD_SELECT_NONE: u16 = 1102;

// ---- details panel control ids ---------------------------------------------------------------
pub const ED_NAME: u16 = 3000;
/// Position x,y,z = 3001..=3003, rotation = 3004..=3006, scale = 3007..=3009.
pub const ED_XFORM: u16 = 3001;
pub const ED_ALPHA: u16 = 3010;
pub const ED_LAST: u16 = 3010;
pub const ST_INFO: u16 = 3100;

// ---- environment panel ids ------------------------------------------------------------------
pub const ED_FOG_START: u16 = 3020;
pub const ED_FOG_END: u16 = 3021;
pub const ED_DITHER: u16 = 3022;
pub const ED_STEPS: u16 = 3023;
pub const ED_RENDER_DIST: u16 = 3024;
/// Strength of the ambient light.
pub const ED_AMBIENT: u16 = 3025;
/// Quick light colours: warm, cool, red, green (1200 + n).
pub const CMD_LIGHT_COLOR_BASE: u16 = 1200;
pub const CB_FOG: u16 = 3030;
pub const CB_SKY: u16 = 3031;
pub const ST_ENV_INFO: u16 = 3101;

// ---- terrain panel ids ------------------------------------------------------------------------
pub const ED_TERRAIN_DIST: u16 = 3038;
pub const ED_TERRAIN_RES: u16 = 3039;
pub const ST_TERRAIN_RES: u16 = 3102;
pub const CMD_TERRAIN_CHUNKED: u16 = 1113;
/// Layer r: seed 3040 + r, scale 3050 + r, contribution 3060 + r, type combo 3070 + r.
pub const ED_TSEED0: u16 = 3040;
pub const ED_TSCALE0: u16 = 3050;
pub const ED_TAMP0: u16 = 3060;
pub const CB_TTYPE0: u16 = 3070;
pub const CMD_TERRAIN_ADD_LAYER: u16 = 1110;
pub const CMD_TERRAIN_REMOVE_LAYER: u16 = 1111;
pub const CMD_TERRAIN_RANDOMIZE: u16 = 1112;
pub const CMD_ENV_FOG_COLOR: u16 = 1090;
pub const CMD_ENV_FOG_FROM_SKY: u16 = 1091;
pub const CMD_ENV_SKY_COLOR: u16 = 1092;
pub const CMD_ENV_SKY_BOTTOM: u16 = 1093;
pub const CMD_ENV_SKY_TEXTURE: u16 = 1094;
pub const CMD_SELECT_ENV: u16 = 1095;
const CMD_LAST: u16 = 1399;


/// How a number box behaves when dragged: (change per pixel, min, max, whole numbers only, scales with the scene).
pub(crate) fn drag_spec(id: u16) -> Option<(f32, f32, f32, bool, bool)> {
    Some(match id {
        ED_XFORM..=3003 => (0.004, f32::MIN, f32::MAX, false, true), // position
        3004..=3006 => (0.5, -3600.0, 3600.0, false, false),         // rotation (degrees)
        3007..=3009 => (0.01, -1000.0, 1000.0, false, false),        // scale
        ED_ALPHA => (0.004, 0.0, 1.0, false, false),
        ED_FOG_START | ED_FOG_END => (0.02, 0.0, 1000.0, false, false),
        ED_DITHER => (0.05, 1.0, 12.0, false, false),
        ED_STEPS => (0.05, 2.0, 32.0, true, false),
        ED_RENDER_DIST => (0.1, 0.0, 1.0e7, false, true),
        ED_AMBIENT => (0.01, 0.0, 8.0, false, false),
        ED_TERRAIN_RES => (0.5, 2.0, 256.0, true, false),
        ED_TERRAIN_DIST => (0.05, 1.0, 12.0, true, false),
        x if (ED_PVEL0..ED_PVEL0 + 3).contains(&x) => (0.01, -10_000.0, 10_000.0, false, true),
        ED_PGRAV => (0.01, -10_000.0, 10_000.0, false, true),
        ED_PL_FOV => (0.2, 10.0, 170.0, false, false),
        ED_LINT => (0.01, 0.0, 100.0, false, false),
        ED_TRUNK => (0.002, 0.1, 0.7, false, false),
        ED_TRWIDTH => (0.001, 0.02, 0.2, false, false),
        ED_CROWN => (0.002, 0.2, 1.0, false, false),
        ED_DETAIL => (0.1, 2.0, 8.0, true, false),
        ED_TRSEED => (0.25, 0.0, 999_999.0, true, false),
        ED_LRANGE => (0.05, 0.1, 100_000.0, false, true),
        ED_LANGLE => (0.5, 5.0, 170.0, false, false),
        x if (ED_PL0..=ED_PL_LAST).contains(&x) => (0.01, 0.0, 10_000.0, false, false),
        ED_SCOUNT => (2.0, 0.0, 5000.0, true, false),
        ED_SMIN | ED_SMAX => (0.005, 0.001, 100_000.0, false, true),
        ED_SSEED => (0.25, 0.0, 999_999.0, true, false),
        ED_SSLOPE => (0.5, 0.0, 90.0, false, false),
        ED_SSINK => (0.005, -100_000.0, 100_000.0, false, true),
        ED_PSIZE => (0.001, 0.001, 1000.0, false, true),
        ED_PAMOUNT => (5.0, 0.0, 50_000.0, true, false),
        ED_PBOX => (0.05, 0.1, 100_000.0, false, true),
        ED_PLIFE => (0.02, 0.1, 600.0, false, false),
        x if (ED_TSEED0..ED_TSEED0 + 5).contains(&x) => (0.25, 0.0, 999_999.0, true, false),
        x if (ED_TSCALE0..ED_TSCALE0 + 5).contains(&x) => (0.002, 0.01, 10.0, false, false),
        x if (ED_TAMP0..ED_TAMP0 + 5).contains(&x) => (0.002, -10.0, 10.0, false, false),
        _ => return None,
    })
}

pub(crate) fn format_number(v: f32, whole: bool) -> String {
    if whole {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" { "0".into() } else { s.to_string() }
}

/// The Add menu: grouped shapes, nature, effects, the player and ready-made gameplay objects.
pub fn add_menu_items() -> Vec<MenuItem> {
    use crate::prims::Prim;
    let item = |p: Prim| MenuItem::new(CMD_ADD_BASE + crate::prims::ALL.iter().position(|&q| q == p).unwrap_or(0) as u16, p.name());
    let group = |title: &str, list: &[Prim]| MenuItem::submenu(title, list.iter().map(|&p| item(p)).collect());
    vec![
        group("Shapes", &[Prim::Cube, Prim::Plane, Prim::Sphere, Prim::Cylinder, Prim::Cone, Prim::Torus, Prim::Capsule, Prim::Pyramid, Prim::Disc]),
        group("Building", &[Prim::Ramp, Prim::Stairs, Prim::Arch]),
        group("Nature", &[Prim::Terrain, Prim::Tree, Prim::Rock]),
        item(Prim::Particles),
        item(Prim::Player),
        item(Prim::Light),
        MenuItem::sep(),
        MenuItem::submenu("Gameplay", crate::prefabs::PREFABS.iter().enumerate().map(|(i, p)| MenuItem::new(CMD_PREFAB_BASE + i as u16, p.name)).collect()),
    ]
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn pos_of(l: isize) -> (f32, f32) {
    ((l & 0xFFFF) as i16 as f32, ((l >> 16) & 0xFFFF) as i16 as f32)
}

fn push(ev: Raw) {
    RAW.with_borrow_mut(|r| r.push(ev));
}

unsafe extern "system" fn main_proc(h: HWND, m: u32, w: usize, l: isize) -> isize {
    match m {
        WM_CLOSE => push(Raw::Close),
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        WM_SIZE => push(Raw::Resize((l & 0xFFFF) as f32, ((l >> 16) & 0xFFFF) as f32)),
        WM_PAINT => {
            unsafe { ValidateRect(h, std::ptr::null()) }; // the frame loop paints continuously
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_GETMINMAXINFO => {
            let mmi = unsafe { &mut *(l as *mut MINMAXINFO) };
            mmi.min_track = POINT { x: 960, y: 640 };
            return 0;
        }
        WM_COMMAND => {
            let id = (w & 0xFFFF) as u16;
            if (CMD_OPEN_PROJECT..=CMD_LAST).contains(&id) {
                POSTED.with_borrow_mut(|c| c.push(id));
            }
        }
        WM_MOUSEMOVE => {
            if !TRACKING.get() {
                let mut t = TRACKMOUSEEVENT { cb_size: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, flags: 2, hwnd: h, hover_time: 0 };
                unsafe { TrackMouseEvent(&mut t) };
                TRACKING.set(true);
            }
            let (x, y) = pos_of(l);
            push(Raw::Move(x, y));
        }
        WM_MOUSELEAVE => {
            TRACKING.set(false);
            push(Raw::Leave);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            unsafe { SetCapture(h) };
            let (x, y) = pos_of(l);
            push(Raw::Down(if m == WM_LBUTTONDOWN || m == WM_LBUTTONDBLCLK { 0 } else { 1 }, x, y, m == WM_LBUTTONDBLCLK || m == WM_RBUTTONDBLCLK));
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let (x, y) = pos_of(l);
            push(Raw::Up(if m == WM_LBUTTONUP { 0 } else { 1 }, x, y));
            // keep the capture while the other button is still down
            let other = if m == WM_LBUTTONUP { 0x02 } else { 0x01 };
            if unsafe { GetAsyncKeyState(other) } as u16 & 0x8000 == 0 {
                unsafe { ReleaseCapture() };
            }
        }
        WM_MOUSEWHEEL => {
            let mut p = POINT { x: (l & 0xFFFF) as i16 as i32, y: ((l >> 16) & 0xFFFF) as i16 as i32 };
            unsafe { ScreenToClient(h, &mut p) };
            let delta = ((w >> 16) & 0xFFFF) as u16 as i16 as f32 / 120.0;
            push(Raw::Wheel(delta, p.x as f32, p.y as f32));
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = w as u32;
            push(Raw::Key(vk, m == WM_SYSKEYDOWN));
            // Alt+F4 (close) and Alt+Space (system menu) stay with Windows
            if m == WM_SYSKEYDOWN && (vk == 0x73 || vk == 0x20) {
                return unsafe { DefWindowProcW(h, m, w, l) };
            }
            return 0;
        }
        WM_CHAR => {
            let u = w as u16;
            if (0xD800..0xDC00).contains(&u) {
                HIGH_SURROGATE.set(u);
            } else {
                let c = if (0xDC00..0xE000).contains(&u) {
                    let hi = HIGH_SURROGATE.replace(0) as u32;
                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (u as u32 - 0xDC00))
                } else {
                    char::from_u32(u as u32)
                };
                if let Some(c) = c {
                    push(Raw::Char(c));
                }
            }
            return 0;
        }
        WM_KILLFOCUS => {
            push(Raw::Leave);
            unsafe { ReleaseCapture() };
        }
        WM_SETCURSOR if (l & 0xFFFF) == 1 && !LOOKING.get() => {
            let id = match CURSOR.get() {
                Cursor::Arrow => 32512,
                Cursor::IBeam => 32513,
                Cursor::SizeWE => 32644,
                Cursor::Hand => 32649,
            };
            unsafe { SetCursor(LoadCursorW(0, id)) };
            return 1;
        }
        _ => return unsafe { DefWindowProcW(h, m, w, l) },
    }
    0
}

/// Sets the interface size (percent, 0 = follow Windows) used by the next window that opens.
pub fn set_scale_override(percent: u32) {
    SCALE_OVERRIDE.set(percent);
}

/// Pixel size of a 96-dpi length.
pub fn px(v: i32) -> i32 {
    (v as f32 * SCALE.get()).round() as i32
}

pub struct Window {
    main: HWND,
    dc: HDC,
    ui: RefCell<Ui>,
    rend: RefCell<Option<Renderer>>,
    /// Which mouse buttons went down in the 3D view (their drags belong to the application).
    view_capture: Cell<[bool; 2]>,
}

impl Window {
    /// Opens the window and makes an OpenGL 3.3 core context current on this thread.
    pub fn new(title: &str, w: i32, h: i32) -> Result<Self, String> {
        unsafe {
            // Work in real pixels: otherwise Windows bitmap-scales the whole window on high-DPI displays.
            SetProcessDPIAware();
            timeBeginPeriod(1); // sleeps of a millisecond really last about a millisecond (frame rate cap)
            SCALE.set(match SCALE_OVERRIDE.get() {
                0 => GetDpiForSystem() as f32 / 96.0,
                pct => pct as f32 / 100.0,
            });
            let (w, h) = (px(w), px(h));
            let inst = GetModuleHandleW(std::ptr::null());
            let class = wide("polystation_main");
            let wc = WNDCLASSW {
                style: 0x0003 | 0x0020 | 0x0008, // HREDRAW | VREDRAW | OWNDC | DBLCLKS
                lpfnWndProc: main_proc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: inst,
                hIcon: 0,
                hCursor: LoadCursorW(0, 32512),
                hbrBackground: 0,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                return Err("RegisterClassW failed".into());
            }
            // Fit the window into the primary monitor's work area (screen minus taskbar), centred.
            let mut work = RECT { left: 0, top: 0, right: w, bottom: h };
            SystemParametersInfoW(0x0030, 0, &mut work as *mut RECT as *mut c_void, 0); // SPI_GETWORKAREA
            let (aw, ah) = (work.right - work.left, work.bottom - work.top);
            let (w, h) = (w.min(aw), h.min(ah));
            let (x, y) = (work.left + (aw - w) / 2, work.top + (ah - h) / 2);
            let main = CreateWindowExW(0, class.as_ptr(), wide(title).as_ptr(), 0x00CF0000 | 0x1000_0000, x, y, w, h, 0, 0, inst, std::ptr::null_mut()); // OVERLAPPEDWINDOW | VISIBLE
            if main == 0 {
                return Err("CreateWindowExW failed".into());
            }
            MAIN.set(main);
            // dark title bar
            let one: u32 = 1;
            DwmSetWindowAttribute(main, 20, &one as *const u32 as *const c_void, 4);
            for (attr, color) in [(35u32, 0x001C1817u32), (36, 0x00EBE6E4), (34, 0x003E3634)] {
                DwmSetWindowAttribute(main, attr, &color as *const u32 as *const c_void, 4);
            }

            let mut ui = Ui::new(SCALE.get());
                        let popup = |items: &[(u16, &str)]| -> Vec<MenuItem> {
                items.iter().map(|&(id, text)| if id == 0 { MenuItem::sep() } else { MenuItem::new(id, &text.replace('&', "")) }).collect()
            };
            let file = popup(&[
                (CMD_NEW_PROJECT, "&New Project...\tCtrl+N"),
                (CMD_OPEN_PROJECT, "&Open Project...\tCtrl+O"),
                (CMD_START_PAGE, "Start &Page..."),
                (CMD_SAVE, "&Save Project\tCtrl+S"),
                (CMD_SAVE_AS, "Save Project &As...\tCtrl+Shift+S"),
                (0, ""),
                (CMD_IMPORT, "&Import Model...\tCtrl+I"),
                (CMD_SCREENSHOT, "Save S&creenshot...\tF12"),
                (0, ""),
                (CMD_COMPILE, "Co&mpile Game..."),
                (CMD_SETTINGS, "Se&ttings..."),
                (0, ""),
                (CMD_EXIT, "E&xit"),
            ]);
            let edit = popup(&[
                (CMD_UNDO, "&Undo\tCtrl+Z"),
                (CMD_REDO, "&Redo\tCtrl+Y"),
                (0, ""),
                (CMD_DELETE, "&Delete\tDel"),
                (CMD_DUPLICATE, "D&uplicate Primitive\tCtrl+D"),
                (0, ""),
                (CMD_HIDE_SEL, "&Hide Selected\tH"),
                (CMD_SHOW_ALL, "Show &All\tAlt+H"),
                (CMD_SELECT_ALL, "Select A&ll\tCtrl+A"),
                (CMD_SELECT_NONE, "Select &None\tEsc"),
                (0, ""),
                (CMD_GIZMO_BASE, "Gizmo: N&one\tQ"),
                (CMD_GIZMO_BASE + 1, "Gizmo: &Move\tR"),
                (CMD_GIZMO_BASE + 2, "Gizmo: &Rotate\tT"),
                (CMD_GIZMO_BASE + 3, "Gizmo: &Scale\tY"),
            ]);
            let add = add_menu_items();
            let view = popup(&[
                (CMD_MODE_BASE, "&Textured\t1"),
                (CMD_MODE_BASE + 1, "&Solid\t2"),
                (CMD_MODE_BASE + 2, "&Wireframe\t3"),
                (CMD_MODE_BASE + 3, "Textured + Wire&frame\t4"),
                (CMD_MODE_BASE + 4, "&Unlit\t5"),
                (CMD_MODE_BASE + 5, "&Object Colors\t6"),
                (0, ""),
                (CMD_AFFINE, "&Affine Texture Mapping"),
                (CMD_GRID, "&Grid"),
                (CMD_CULL, "Backface &Culling"),
                (0, ""),
                (CMD_FOG_BASE, "Fog: O&ff"),
                (CMD_FOG_BASE + 1, "Fog: S&mooth"),
                (CMD_FOG_BASE + 2, "Fog: &Dithered"),
                (CMD_FOG_NEARER, "Fog Den&ser\t["),
                (CMD_FOG_FARTHER, "Fog Th&inner\t]"),
                (CMD_DITHER_FINER, "Dither Fi&ner\t,"),
                (CMD_DITHER_CHUNKIER, "Dither Chun&kier\t."),
                (0, ""),
                (CMD_FRAME_SEL, "Frame &Selected\tF"),
                (CMD_FRAME_ALL, "Frame A&ll\tShift+F"),
                (CMD_SELECT_ENV, "&Environment Settings..."),
                (CMD_GUI_EDITOR, "Game &GUI Editor..."),
                (CMD_RESET_CAMERA, "&Reset Camera"),
            ]);
            let run = popup(&[(CMD_PLAY, "&Play\tF5")]);
            let mut help = vec![MenuItem::new(CMD_HELP_GUIDE, "Scripting Guide"), MenuItem::new(CMD_UPDATE, "Check for Updates..."), MenuItem::new(CMD_UNINSTALL, "Uninstall Polystation..."), MenuItem::sep()];
            help.push(MenuItem::submenu(
                "Example Scripts",
                crate::guide::EXAMPLES.iter().enumerate().map(|(i, (title, _))| MenuItem::new(CMD_EXAMPLE_BASE + i as u16, title)).collect(),
            ));
            ui.set_bar(vec![
                ("File".into(), file), ("Edit".into(), edit), ("Add".into(), add), ("View".into(), view), ("Run".into(), run), ("Help".into(), help)]);
            let group = Cell::new(1u8);
            let dy = Cell::new(0);
            const EDIT: u32 = 0;
            const BTN: u32 = 0;
            const COMBO: u32 = 0;
            const CHECK: u32 = 0x3; // BS_AUTOCHECKBOX: a button that stays pressed
            let mut ctl = |class: &str, text: &str, style: u32, id: u16, x: i32, y: i32, w: i32, h: i32| {
                let y = y + dy.get();
                let g = group.get();
                let kind = match class {
                    "BUTTON" => Kind::Button,
                    "EDIT" => Kind::Edit,
                    "COMBOBOX" => Kind::Combo,
                    _ if id == ST_SECTION => Kind::Section,
                    _ => Kind::Static,
                };
                let mut wd = Widget::new(id, kind, g, x, y, w, h, text);
                if kind == Kind::Button {
                    wd.toggle = style & 0x3 == 0x3 && style & 0x8 == 0 && style != 0;
                    wd.accent = id == CMD_PLAY;
                }
                if kind == Kind::Edit {
                    let multi = id == ED_SCRIPT || id == ED_GUIDE;
                    let mut f = Field::new(multi, id == ED_GUIDE);
                    if id == ED_SCRIPT {
                        f.highlight = Highlight::Script;
                        f.gutter = true;
                    }
                    if id == ED_GUIDE {
                        f.highlight = Highlight::Guide;
                    }
                    wd.field = Some(Box::new(f));
                }
                ui.widgets.push(wd);
            };
            group.set(9);
            ctl("BUTTON", "Play", BTN, CMD_PLAY, 8, 5, 84, 26);
            ctl("BUTTON", "+ Add", BTN, CMD_ADD_MENU, 100, 5, 74, 26);
            ctl("BUTTON", "Compile", BTN, CMD_COMPILE, 182, 5, 84, 26);
            ctl("BUTTON", "GUI", BTN, CMD_GUI_EDITOR, 274, 5, 56, 26);
            // file explorer (bottom): Up / Refresh buttons, the current folder, and a list of its files
            group.set(8);
            ctl("BUTTON", "Up", BTN, CMD_EXPLORER_UP, 4, 2, 50, 22);
            ctl("BUTTON", "Refresh", BTN, CMD_EXPLORER_REFRESH, 58, 2, 70, 22);
            ctl("STATIC", "", 0, ST_EXPLORER_PATH, 136, 6, 0, 18);
            // LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS, with a border and sibling clipping
            ctl("SysListView32", "", 0x1 | 0x4 | 0x8 | 0x0400_0000, LV_EXPLORER, 0, 28, 0, 0);
            // script editor (second tab of the bottom area): a status line and the code box
            group.set(7);
            ctl("STATIC", "", 0, ST_SCRIPT_INFO, 8, 3, 0, 20);
            // ES_MULTILINE | AUTOVSCROLL | AUTOHSCROLL | WANTRETURN, with both scroll bars and a tab stop
            ctl("EDIT", "", 0x4 | 0x40 | 0x80 | 0x1000 | 0x0020_0000 | 0x0010_0000 | 0x0001_0000, ED_SCRIPT, 0, 26, 0, 0);
            // the written guide (third tab of the bottom area): read-only text
            group.set(10);
            ctl("EDIT", "", 0x4 | 0x40 | 0x800 | 0x0020_0000 | 0x0001_0000, ED_GUIDE, 0, 4, 0, 0);
            group.set(1);
            ctl("STATIC", "Name", 0, 0, 8, 10, 44, 18);
            ctl("EDIT", "", EDIT, ED_NAME, 58, 6, 236, 22);
            ctl("STATIC", "", 0, ST_INFO, 8, 38, 288, 112);
            ctl("STATIC", "TRANSFORM", 0, ST_SECTION, 8, 156, 288, 16);
            for (row, label) in ["Position", "Rotation", "Scale"].iter().enumerate() {
                let y = 178 + row as i32 * 28;
                ctl("STATIC", label, 0, 0, 8, y + 4, 62, 18);
                for axis in 0..3 {
                    ctl("EDIT", "", EDIT, ED_XFORM + (row * 3 + axis) as u16, 72 + axis as i32 * 74, y, 68, 22);
                }
            }
            ctl("BUTTON", "Reset Transform", BTN, CMD_RESET_XFORM, 8, 266, 140, 24);
            ctl("STATIC", "APPEARANCE", 0, ST_SECTION, 8, 306, 288, 16);
            ctl("STATIC", "Alpha", 0, 0, 8, 330, 62, 18);
            ctl("EDIT", "", EDIT, ED_ALPHA, 72, 326, 68, 22);
            ctl("STATIC", "", 0, ST_TEXTURE, 8, 356, 288, 18);
            for (i, (id, text)) in [
                (CMD_COLOR, "Color..."), (CMD_CLEAR_COLOR, "Clear Color"), (CMD_TEXTURE_SET, "Texture..."),
                (CMD_TEXTURE_CLEAR, "Clear Texture"), (CMD_FRAME_SEL, "Frame"), (CMD_DUPLICATE, "Duplicate"),
            ].into_iter().enumerate() {
                ctl("BUTTON", text, BTN, id, 8 + (i as i32 % 3) * 98, 380 + (i as i32 / 3) * 30, 94, 24);
            }
            ctl("BUTTON", "Delete", BTN, CMD_DELETE, 8, 440, 94, 24);
            group.set(0);

            // how the player meets the object (part of the object details)
            group.set(1);
            ctl("STATIC", "COLLISION", 0, ST_SECTION, 8, 480, 288, 16);
            ctl("COMBOBOX", "", COMBO, CB_COLLISION, 8, 502, 200, 150);
            ctl("STATIC", "Solid blocks the player. Passable can be walked through. Trigger is passable and runs on touch / on leave. Water is a pool: the player wades and swims, and the surface ripples.", 0, ST_COLLISION_HINT, 8, 532, 288, 70);
            // Terrain settings (shown below the object details while a terrain is selected)
            group.set(3);
            dy.set(-504 + 596); // starts right below the object details
            ctl("STATIC", "TERRAIN", 0, ST_SECTION, 8, 504, 150, 16);
            ctl("BUTTON", "Add scatter", BTN, CMD_SCATTER_ADD, 160, 500, 136, 22);
            ctl("STATIC", "Resolution", 0, 0, 8, 532, 76, 18);
            ctl("EDIT", "", EDIT, ED_TERRAIN_RES, 84, 528, 68, 22);
            ctl("STATIC", "cells per side", 0, ST_TERRAIN_RES, 158, 532, 130, 18);
            ctl("BUTTON", "Load in chunks", BTN | CHECK, CMD_TERRAIN_CHUNKED, 8, 556, 150, 22); // auto check box
            ctl("STATIC", "Render dist.", 0, 0, 162, 560, 80, 18);
            ctl("EDIT", "", EDIT, ED_TERRAIN_DIST, 246, 556, 50, 22);
            for (x, label) in [(8, "Type"), (112, "Seed"), (168, "Scale"), (228, "Amount")] {
                ctl("STATIC", label, 0, 0, x, 586, 60, 16);
            }
            for r in 0..crate::terrain::MAX_LAYERS as u16 {
                let y = 606 + r as i32 * 28;
                ctl("COMBOBOX", "", COMBO, CB_TTYPE0 + r, 8, y, 100, 150);
                ctl("EDIT", "", EDIT, ED_TSEED0 + r, 112, y, 52, 22);
                ctl("EDIT", "", EDIT, ED_TSCALE0 + r, 168, y, 56, 22);
                ctl("EDIT", "", EDIT, ED_TAMP0 + r, 228, y, 66, 22);
            }
            ctl("BUTTON", "Add layer", BTN, CMD_TERRAIN_ADD_LAYER, 8, 756, 94, 24);
            ctl("BUTTON", "Remove layer", BTN, CMD_TERRAIN_REMOVE_LAYER, 106, 756, 94, 24);
            ctl("BUTTON", "Randomize", BTN, CMD_TERRAIN_RANDOMIZE, 204, 756, 92, 24);

            // Scatter layer settings (shown instead of the object details for a scatter layer)
            group.set(5);
            dy.set(0);
            ctl("STATIC", "SCATTER", 0, ST_SECTION, 8, 8, 288, 16);
            ctl("STATIC", "", 0, ST_STARGET, 8, 32, 288, 36);
            ctl("BUTTON", "Texture...", BTN, CMD_TEXTURE_SET, 8, 74, 94, 24);
            ctl("BUTTON", "Clear Texture", BTN, CMD_TEXTURE_CLEAR, 106, 74, 94, 24);
            ctl("BUTTON", "Color...", BTN, CMD_COLOR, 204, 74, 92, 24);
            ctl("STATIC", "Shape", 0, 0, 8, 112, 62, 18);
            ctl("COMBOBOX", "", COMBO, CB_SSHAPE, 72, 108, 222, 150);
            ctl("STATIC", "Source", 0, 0, 8, 146, 62, 18);
            ctl("COMBOBOX", "", COMBO, CB_SSOURCE, 72, 142, 222, 150);
            for (i, (label, id, unit)) in [
                ("Count", ED_SCOUNT, "per chunk (or total)"),
                ("Min size", ED_SMIN, "world units"),
                ("Max size", ED_SMAX, "world units"),
                ("Seed", ED_SSEED, ""),
                ("Max slope", ED_SSLOPE, "degrees (90 = any)"),
                ("Sink", ED_SSINK, "units into the ground"),
            ]
            .into_iter()
            .enumerate()
            {
                let y = 178 + i as i32 * 28;
                ctl("STATIC", label, 0, 0, 8, y + 4, 62, 18);
                ctl("EDIT", "", EDIT, id, 72, y, 68, 22);
                if unit.is_empty() {
                    ctl("BUTTON", "New random layout", BTN, CMD_SCATTER_RANDOMIZE, 146, y - 1, 150, 24);
                } else {
                    ctl("STATIC", unit, 0, 0, 146, y + 4, 150, 18);
                }
            }
            ctl("STATIC", "Every scatter layer is an object of its own: add more layers for trees, rocks, grass. Source = plant copies of any object or collection of the scene (a tree you modeled): hide the original with its checkbox.", 0, 0, 8, 356, 288, 80);

            // Player settings (shown instead of the object details for a player)
            group.set(6);
            dy.set(0);
            ctl("STATIC", "PLAYER", 0, ST_SECTION, 8, 8, 288, 16);
            ctl("STATIC", "", 0, ST_PLSOUND, 8, 30, 288, 18);
            ctl("BUTTON", "Step sound...", BTN, CMD_PLAYER_SOUND, 8, 52, 94, 24);
            ctl("BUTTON", "No sound", BTN, CMD_PLAYER_SOUND_CLEAR, 106, 52, 94, 24);
            ctl("BUTTON", "Color...", BTN, CMD_COLOR, 204, 52, 92, 24);
            for (i, (label, unit)) in [
                ("Size", "x bigger, scene unchanged"),
                ("Speed", "units/s"),
                ("Sprint x", "speed multiplier"),
                ("Gravity", "units/s2 (9.81 = Earth)"),
                ("Jump", "jump height, units"),
                ("Height", "player height"),
                ("Radius", "body radius"),
                ("Eye height", "camera above feet"),
                ("FOV", "degrees"),
                ("Head bob", "sway amount, units"),
                ("Bob speed", "cycles per second"),
                ("Step vol.", "0 - 1"),
                ("Step dist.", "units between steps"),
                ("Mouse", "degrees per pixel"),
                ("Max step", "ledge climbed, units"),
                ("Air ctrl", "0 - 1"),
            ]
            .into_iter()
            .enumerate()
            {
                let y = 88 + i as i32 * 25;
                ctl("STATIC", label, 0, 0, 8, y + 4, 62, 18);
                ctl("EDIT", "", EDIT, ED_PL0 + i as u16, 72, y, 68, 22);
                ctl("STATIC", unit, 0, 0, 146, y + 4, 150, 18);
            }

            // Light settings (shown instead of the object details for a light)
            group.set(11);
            dy.set(0);
            ctl("STATIC", "LIGHT", 0, ST_SECTION, 8, 8, 288, 16);
            ctl("STATIC", "", 0, ST_LINFO, 8, 30, 288, 18);
            ctl("BUTTON", "Color...", BTN, CMD_COLOR, 8, 54, 120, 24);
            ctl("STATIC", "Kind", 0, 0, 8, 96, 62, 18);
            ctl("COMBOBOX", "", COMBO, CB_LKIND, 72, 92, 222, 150);
            for (i, (label, id, unit)) in [("Intensity", ED_LINT, "brightness"), ("Range", ED_LRANGE, "world units (point, spot)"), ("Angle", ED_LANGLE, "degrees (spot)")].into_iter().enumerate() {
                let y = 128 + i as i32 * 28;
                ctl("STATIC", label, 0, 0, 8, y + 4, 62, 18);
                ctl("EDIT", "", EDIT, id, 72, y, 68, 22);
                ctl("STATIC", unit, 0, 0, 146, y + 4, 150, 18);
            }
            ctl("STATIC", "Colors", 0, 0, 8, 232, 52, 18);
            for (i, name) in ["Warm", "Cool", "Red", "Green"].into_iter().enumerate() {
                ctl("BUTTON", name, BTN, CMD_LIGHT_COLOR_BASE + i as u16, 62 + i as i32 * 58, 228, 54, 24);
            }
            ctl("STATIC", "A light shines down the object's -Y axis: rotate it to aim a spot light or the sun. Lighting is worked out at the corners (vertices) of every face, so use Terrain, or small faces, for floors that should show a pool of light.", 0, 0, 8, 262, 288, 96);

            // Tree settings (shown instead of the object details for a tree)
            group.set(12);
            dy.set(0);
            ctl("STATIC", "TREE", 0, ST_SECTION, 8, 8, 288, 16);
            ctl("STATIC", "", 0, ST_TRINFO, 8, 30, 288, 18);
            ctl("BUTTON", "Texture...", BTN, CMD_TEXTURE_SET, 8, 54, 94, 24);
            ctl("BUTTON", "Clear Texture", BTN, CMD_TEXTURE_CLEAR, 106, 54, 94, 24);
            ctl("BUTTON", "Color...", BTN, CMD_COLOR, 204, 54, 92, 24);
            ctl("STATIC", "Kind", 0, 0, 8, 96, 62, 18);
            ctl("COMBOBOX", "", COMBO, CB_TKIND, 72, 92, 222, 150);
            for (i, (label, id, unit)) in [
                ("Trunk", ED_TRUNK, "bare trunk, 0.1 - 0.7"),
                ("Trunk width", ED_TRWIDTH, "of the height"),
                ("Crown", ED_CROWN, "width of the leaves"),
                ("Detail", ED_DETAIL, "layers / clumps / fronds"),
                ("Seed", ED_TRSEED, ""),
            ]
            .into_iter()
            .enumerate()
            {
                let y = 128 + i as i32 * 28;
                ctl("STATIC", label, 0, 0, 8, y + 4, 62, 18);
                ctl("EDIT", "", EDIT, id, 72, y, 68, 22);
                if unit.is_empty() {
                    ctl("BUTTON", "New random tree", BTN, CMD_TREE_RANDOMIZE, 146, y - 1, 150, 24);
                } else {
                    ctl("STATIC", unit, 0, 0, 146, y + 4, 150, 18);
                }
            }
            ctl("STATIC", "Colors come from the built-in tree palette. Scale the object for a bigger or smaller tree. To plant many, add a Scatter layer on a terrain.", 0, 0, 8, 280, 288, 80);

            // Particle system settings (shown instead of the object details for a particle system)
            group.set(4);
            dy.set(0);
            ctl("STATIC", "PARTICLE SYSTEM", 0, ST_SECTION, 8, 8, 288, 16);
            ctl("STATIC", "", 0, ST_PTEXTURE, 8, 34, 288, 18);
            ctl("BUTTON", "Texture...", BTN, CMD_TEXTURE_SET, 8, 58, 94, 24);
            ctl("BUTTON", "Clear Texture", BTN, CMD_TEXTURE_CLEAR, 106, 58, 94, 24);
            ctl("BUTTON", "Color...", BTN, CMD_COLOR, 204, 58, 92, 24);
            ctl("STATIC", "Velocity", 0, 0, 8, 100, 62, 18);
            for axis in 0..3 {
                ctl("EDIT", "", EDIT, ED_PVEL0 + axis, 72 + axis as i32 * 74, 96, 68, 22);
            }
            for (i, (label, id, unit)) in [
                ("Gravity", ED_PGRAV, "units/s2, down"),
                ("Size", ED_PSIZE, "world units"),
                ("Amount", ED_PAMOUNT, "particles (max 50000)"),
                ("Box size", ED_PBOX, "cube around the camera"),
                ("Lifetime", ED_PLIFE, "seconds"),
            ]
            .into_iter()
            .enumerate()
            {
                let y = 128 + i as i32 * 28;
                ctl("STATIC", label, 0, 0, 8, y + 4, 62, 18);
                ctl("EDIT", "", EDIT, id, 72, y, 68, 22);
                ctl("STATIC", unit, 0, 0, 146, y + 4, 150, 18);
            }
            ctl("STATIC", "Particles fill a cube around the camera and wrap around it, so the camera always sees an endless field of them. Hide the object with its checkbox.", 0, 0, 8, 276, 288, 64);

            // Environment settings (shown instead of the object details when the Environment node is selected)
            group.set(2);
            dy.set(0);
            ctl("STATIC", "ENVIRONMENT", 0, ST_SECTION, 8, 8, 288, 16);
            ctl("STATIC", "Fog", 0, 0, 8, 38, 62, 18);
            ctl("COMBOBOX", "", COMBO, CB_FOG, 72, 34, 222, 150);
            ctl("STATIC", "Range", 0, 0, 8, 70, 62, 18);
            ctl("EDIT", "", EDIT, ED_FOG_START, 72, 66, 68, 22);
            ctl("EDIT", "", EDIT, ED_FOG_END, 146, 66, 68, 22);
            ctl("STATIC", "x radius", 0, 0, 220, 70, 76, 18);
            ctl("STATIC", "Dither", 0, 0, 8, 98, 62, 18);
            ctl("EDIT", "", EDIT, ED_DITHER, 72, 94, 68, 22);
            ctl("EDIT", "", EDIT, ED_STEPS, 146, 94, 68, 22);
            ctl("STATIC", "px / steps", 0, 0, 220, 98, 76, 18);
            ctl("STATIC", "Draw dist.", 0, 0, 8, 126, 62, 18);
            ctl("EDIT", "", EDIT, ED_RENDER_DIST, 72, 122, 68, 22);
            ctl("STATIC", "units (0 = no limit)", 0, 0, 146, 126, 150, 18);
            ctl("BUTTON", "Fog color...", BTN, CMD_ENV_FOG_COLOR, 8, 154, 140, 24);
            ctl("BUTTON", "Fog = sky", BTN, CMD_ENV_FOG_FROM_SKY, 152, 154, 140, 24);
            ctl("STATIC", "Sky", 0, 0, 8, 190, 62, 18);
            ctl("COMBOBOX", "", COMBO, CB_SKY, 72, 186, 222, 150);
            ctl("BUTTON", "Sky color...", BTN, CMD_ENV_SKY_COLOR, 8, 218, 94, 24);
            ctl("BUTTON", "Bottom...", BTN, CMD_ENV_SKY_BOTTOM, 106, 218, 94, 24);
            ctl("BUTTON", "Texture...", BTN, CMD_ENV_SKY_TEXTURE, 204, 218, 92, 24);
            ctl("BUTTON", "Ambient light...", BTN, CMD_ENV_AMBIENT, 8, 252, 140, 24);
            ctl("STATIC", "Strength", 0, 0, 156, 256, 56, 18);
            ctl("EDIT", "", EDIT, ED_AMBIENT, 216, 252, 68, 22);
            ctl("STATIC", "", 0, ST_ENV_INFO, 8, 288, 288, 130);

            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            GetClientRect(main, &mut r);
            ui.relayout(r.right as f32, r.bottom as f32);

            let dc = GetDC(main);
            let pfd = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: 0x04 | 0x20 | 0x01, // DRAW_TO_WINDOW | SUPPORT_OPENGL | DOUBLEBUFFER
                iPixelType: 0, // RGBA
                cColorBits: 32,
                cDepthBits: 24,
                cRedBits: 0, cRedShift: 0, cGreenBits: 0, cGreenShift: 0, cBlueBits: 0, cBlueShift: 0,
                cAlphaBits: 8, cAlphaShift: 0, cAccumBits: 0, cAccumRedBits: 0, cAccumGreenBits: 0,
                cAccumBlueBits: 0, cAccumAlphaBits: 0, cStencilBits: 0, cAuxBuffers: 0, iLayerType: 0,
                bReserved: 0, dwLayerMask: 0, dwVisibleMask: 0, dwDamageMask: 0,
            };
            let fmt = ChoosePixelFormat(dc, &pfd);
            if fmt == 0 || SetPixelFormat(dc, fmt, &pfd) == 0 {
                return Err("could not set a pixel format".into());
            }
            // A legacy context is needed first to reach wglCreateContextAttribsARB.
            let legacy = wglCreateContext(dc);
            if legacy == 0 || wglMakeCurrent(dc, legacy) == 0 {
                return Err("could not create a legacy GL context".into());
            }
            let p = wglGetProcAddress(b"wglCreateContextAttribsARB\0".as_ptr());
            if p.is_null() {
                return Err("wglCreateContextAttribsARB unavailable".into());
            }
            let create: unsafe extern "system" fn(HDC, HGLRC, *const i32) -> HGLRC = std::mem::transmute(p);
            let attribs = [0x2091, 3, 0x2092, 3, 0x9126, 0x1, 0]; // 3.3, core profile
            let rc = create(dc, 0, attribs.as_ptr());
            if rc == 0 {
                return Err("OpenGL 3.3 core context not supported".into());
            }
            wglMakeCurrent(dc, rc);
            wglDeleteContext(legacy);
            // vsync off: the frame rate is not tied to the monitor refresh rate
            let swap = wglGetProcAddress(b"wglSwapIntervalEXT\0".as_ptr());
            if !swap.is_null() {
                let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(swap);
                f(0);
            }
            Ok(Self { main, dc, ui: RefCell::new(ui), rend: RefCell::new(None), view_capture: Cell::new([false; 2]) })
        }
    }

    /// Creates the UI renderer and its fonts once the GL functions are loaded.
    pub fn init_ui(&self, g: Gl) {
        let s = SCALE.get();
        let px = |v: f32| (v * s).round() as i32;
        let fonts = Fonts {
            ui: Font::new(&g, "Segoe UI", px(12.0), false),
            bold: Font::new(&g, "Segoe UI", px(11.0), true),
            mono: Font::new(&g, "Consolas", px(13.0), false),
            pixel: Font::new_pixel(&g),
        };
        self.ui.borrow_mut().fonts = Some(fonts);
        *self.rend.borrow_mut() = Some(Renderer::new(g));
    }

    fn client(&self) -> (f32, f32) {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetClientRect(self.main, &mut r) };
        (r.right as f32, r.bottom as f32)
    }

    /// The 3D view in window pixels: (x, y from the top, width, height).
    pub fn viewport_top(&self) -> (i32, i32, i32, i32) {
        let v = self.ui.borrow().layout().viewport();
        (v[0] as i32, v[1] as i32, v[2] as i32, v[3] as i32)
    }

    /// The 3D view as OpenGL wants it: (x, y from the bottom, width, height).
    pub fn viewport(&self) -> (i32, i32, i32, i32) {
        let (x, y, w, h) = self.viewport_top();
        let (_, ch) = self.client();
        (x, ch as i32 - y - h, w, h)
    }

    /// Size of the 3D view in pixels.
    pub fn size(&self) -> (i32, i32) {
        let (_, _, w, h) = self.viewport_top();
        (w, h)
    }

    /// Paints the UI over the frame. `overlay` draws extra things inside the 3D view (the orientation widget).
    pub fn draw_ui(&self, overlay: &mut dyn FnMut(&mut Ctx, [f32; 4])) {
        let mut rend = self.rend.borrow_mut();
        if let Some(r) = rend.as_mut() {
            self.ui.borrow_mut().draw(r, overlay);
        }
    }

    pub fn swap(&self) {
        unsafe { SwapBuffers(self.dc) };
    }

    // ---------------------------------------------------------------- events
    fn pump(&self) -> bool {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, 0, 0, 0, 1) != 0 {
                if msg.message == 0x0012 {
                    return false; // WM_QUIT
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        true
    }

    fn mods() -> Mods {
        Mods { ctrl: key_down(0x11), shift: key_down(0x10), alt: key_down(0x12) }
    }

    /// Pumps the message queue; returns the input state. One-shot fields are reset after each call.
    pub fn poll(&self) -> Input {
        if !self.pump() {
            update(|i| i.quit = true);
        }
        let events: Vec<Raw> = RAW.with_borrow_mut(std::mem::take);
        {
            let mut ui = self.ui.borrow_mut();
            for ev in events {
                self.handle(&mut ui, ev);
            }
            CURSOR.set(ui.cursor());
            update(|i| {
                i.tree_dbl_click |= std::mem::take(&mut ui.tree_dbl_click);
                i.explorer_dbl_click |= std::mem::take(&mut ui.explorer_dbl_click);
                if let Some(p) = ui.tree_rclick.take() {
                    i.tree_rclick = Some((p.0 as i32, p.1 as i32));
                }
                if let Some(p) = ui.explorer_rclick.take() {
                    i.explorer_rclick = Some((p.0 as i32, p.1 as i32));
                }
            });
        }
        let i = INPUT.get();
        INPUT.set(Input { wheel: 0.0, tree_dbl_click: false, explorer_dbl_click: false, left_pressed: None, left_released: None, tree_rclick: None, explorer_rclick: None, ..i });
        i
    }

    fn handle(&self, ui: &mut Ui, ev: Raw) {
        let vp = ui.layout().viewport();
        let rel = |x: f32, y: f32| ((x - vp[0]) as i32, (y - vp[1]) as i32);
        match ev {
            Raw::Move(x, y) => {
                ui.mouse_move((x, y));
                update(|i| i.mouse = rel(x, y));
            }
            Raw::Down(btn, x, y, dbl) => {
                if ui.mouse_down(btn, (x, y), dbl, Self::mods()) == Route::Viewport {
                    let mut cap = self.view_capture.get();
                    cap[btn as usize] = true;
                    self.view_capture.set(cap);
                    update(|i| {
                        i.mouse = rel(x, y);
                        if btn == 0 {
                            i.left_down = true;
                            i.left_pressed = Some(rel(x, y));
                        } else {
                            i.right_down = true;
                        }
                    });
                }
            }
            Raw::Up(btn, x, y) => {
                ui.mouse_up(btn, (x, y));
                let mut cap = self.view_capture.get();
                if cap[btn as usize] {
                    cap[btn as usize] = false;
                    self.view_capture.set(cap);
                    update(|i| {
                        i.mouse = rel(x, y);
                        if btn == 0 {
                            i.left_down = false;
                            i.left_released = Some(rel(x, y));
                        } else {
                            i.right_down = false;
                        }
                    });
                }
            }
            Raw::Wheel(n, x, y) => {
                if !ui.wheel(n, (x, y)) {
                    update(|i| i.wheel += n);
                }
            }
            Raw::Key(vk, sys) => self.handle_key(ui, vk, sys),
            Raw::Char(c) => {
                if !PLAYING.get() && ui.text_focus() {
                    ui.char_input(c);
                }
            }
            Raw::Resize(w, h) => ui.relayout(w, h),
            Raw::Leave => {
                ui.mouse_leave();
                // losing the mouse capture ends any drag that was going on in the 3D view
                if self.view_capture.get() != [false; 2] && unsafe { GetAsyncKeyState(0x01) } as u16 & 0x8000 == 0 && unsafe { GetAsyncKeyState(0x02) } as u16 & 0x8000 == 0 {
                    self.view_capture.set([false; 2]);
                    update(|i| {
                        i.left_down = false;
                        i.right_down = false;
                    });
                }
            }
            Raw::Close => update(|i| i.quit = true),
        }
    }

    fn handle_key(&self, ui: &mut Ui, vk: u32, sys: bool) {
        if PLAYING.get() {
            // play mode: Esc and F5 stop; every other key belongs to the game
            if vk == 0x1B || vk == 0x74 {
                ui.commands.push(CMD_PLAY);
            } else if vk == 0x7A && ui.game {
                self.set_fullscreen(!self.is_fullscreen()); // F11 in a compiled game
            }
            return;
        }
        let mods = Self::mods();
        // Alt + letter opens a menu of the menu bar
        if sys && mods.alt && !mods.ctrl {
            if let Some(i) = ['F', 'E', 'A', 'V', 'R'].iter().position(|&c| c as u32 == vk) {
                ui.open_bar(i);
                return;
            }
        }
        // popups, drop-downs, text boxes, the hierarchy and the file list get the key first
        if ui.key_down(vk, mods) {
            return;
        }
        let text = ui.text_focus();
        // a script tab is showing: the single-key shortcuts of the 3D view must not fire (Delete would delete an object)
        if ui.script_active() && !mods.ctrl && vk != 0x74 && vk != 0x7B {
            return;
        }
        let flying = key_down(0x02);
        let (ctrl, shift, alt) = (mods.ctrl, mods.shift, mods.alt);
        let cmd = match (vk, ctrl, shift, alt) {
            (0x4F, true, _, _) => Some(CMD_OPEN_PROJECT),
            (0x4E, true, _, _) => Some(CMD_NEW_PROJECT),
            (0x53, true, false, _) => Some(CMD_SAVE),
            (0x53, true, true, _) => Some(CMD_SAVE_AS),
            (0x49, true, _, _) => Some(CMD_IMPORT),
            (0x7B, _, _, _) => Some(CMD_SCREENSHOT),
            (0x74, _, _, _) => Some(CMD_PLAY),
            (0x5A, true, false, _) => Some(CMD_UNDO),
            (0x5A, true, true, _) | (0x59, true, _, _) => Some(CMD_REDO),
            (0x44, true, _, _) => Some(CMD_DUPLICATE),
            (0x41, true, _, _) => Some(CMD_SELECT_ALL),
            _ if text => None,
            (0x2E, false, _, _) => Some(CMD_DELETE),
            (0x46, false, false, false) => Some(CMD_FRAME_SEL),
            (0x46, false, true, false) => Some(CMD_FRAME_ALL),
            (0x48, false, _, false) => Some(CMD_HIDE_SEL),
            (0x48, false, _, true) => Some(CMD_SHOW_ALL),
            (k @ 0x31..=0x36, false, _, false) => Some(CMD_MODE_BASE + (k - 0x31) as u16),
            (0x1B, false, false, false) => Some(CMD_SELECT_NONE),
            (0x51, false, _, false) if !flying => Some(CMD_GIZMO_BASE),
            (0x52, false, _, false) => Some(CMD_GIZMO_BASE + 1),
            (0x54, false, _, false) => Some(CMD_GIZMO_BASE + 2),
            (0x59, false, _, false) => Some(CMD_GIZMO_BASE + 3),
            (0xDB, false, _, false) => Some(CMD_FOG_NEARER),
            (0xDD, false, _, false) => Some(CMD_FOG_FARTHER),
            (0xBC, false, _, false) => Some(CMD_DITHER_FINER),
            (0xBE, false, _, false) => Some(CMD_DITHER_CHUNKIER),
            _ => None,
        };
        if let Some(c) = cmd {
            ui.commands.push(c);
        }
    }

    // ---------------------------------------------------------------- what the application asks of the UI
    pub fn message(&self, text: &str) {
        if self.rend.borrow().is_none() {
            unsafe { MessageBoxW(self.main, wide(text).as_ptr(), wide("Polystation").as_ptr(), 0x10) };
            return;
        }
        self.run_dialog(&mut MessageDialog::new("Polystation", text, &["OK"]));
    }

    /// Asks a yes / no question; true for the first button.
    pub fn confirm(&self, title: &str, text: &str, yes: &str, no: &str) -> bool {
        let mut d = MessageDialog::new(title, text, &[yes, no]);
        self.run_dialog(&mut d);
        d.result == Some(0)
    }

    pub fn set_title(&self, title: &str) {
        unsafe { SetWindowTextW(self.main, wide(title).as_ptr()) };
    }

    /// Tells the number boxes how big the scene is (position-like fields drag faster in big scenes).
    pub fn set_drag_scale(&self, scene_radius: f32) {
        self.ui.borrow_mut().drag_scale = scene_radius.max(0.01);
    }

    pub fn set_status(&self, text: &str) {
        self.ui.borrow_mut().status = text.to_string();
    }

    /// Menu / button / shortcut commands received since the last call.
    pub fn take_commands(&self) -> Vec<u16> {
        let mut v = POSTED.with_borrow_mut(std::mem::take);
        v.extend(std::mem::take(&mut self.ui.borrow_mut().commands));
        v
    }

    /// Ids of edit boxes the user typed into (or dragged) since the last call.
    pub fn take_edit_events(&self) -> Vec<u16> {
        std::mem::take(&mut self.ui.borrow_mut().edit_events)
    }

    pub fn take_combo_events(&self) -> Vec<u16> {
        std::mem::take(&mut self.ui.borrow_mut().combo_events)
    }

    pub fn set_check(&self, id: u16, on: bool) {
        self.ui.borrow_mut().set_check(id, on);
    }

    pub fn set_radio(&self, first: u16, last: u16, selected: u16) {
        for id in first..=last {
            self.set_check(id, id == selected);
        }
    }

    pub fn set_text(&self, id: u16, text: &str) {
        self.ui.borrow_mut().set_text(id, text);
    }

    pub fn set_checked(&self, id: u16, on: bool) {
        self.ui.borrow_mut().set_checked(id, on);
    }

    pub fn is_checked(&self, id: u16) -> bool {
        self.ui.borrow().is_checked(id)
    }

    pub fn get_text(&self, id: u16) -> String {
        self.ui.borrow().get_text(id)
    }

    /// Full text of a (multi-line) box.
    pub fn get_text_long(&self, id: u16) -> String {
        self.ui.borrow().get_text(id)
    }

    pub fn set_enabled(&self, id: u16, on: bool) {
        self.ui.borrow_mut().set_enabled(id, on);
    }

    pub fn set_group_visible(&self, group: u8, visible: bool) {
        self.ui.borrow_mut().set_group_visible(group, visible);
    }

    pub fn combo_set(&self, id: u16, items: &[&str], selected: usize) {
        self.ui.borrow_mut().combo_set(id, items, selected);
    }

    pub fn combo_selected(&self, id: u16) -> usize {
        self.ui.borrow().combo_selected(id)
    }

    /// True while a text box has the keyboard (the inspector must not overwrite what is being typed).
    pub fn panel_has_focus(&self) -> bool {
        self.ui.borrow().text_focus()
    }

    /// Status line of the script editor: green when `ok`, red otherwise.
    pub fn set_script_status(&self, text: &str, ok: bool) {
        self.ui.borrow_mut().set_script_status(text, ok);
    }

    pub fn show_script_tab(&self, on: bool) {
        self.ui.borrow_mut().show_bottom_tab(on as u8);
    }

    /// Shows tab 0 (project), 1 (script) or 2 (guide) of the bottom area.
    pub fn show_bottom_tab(&self, t: u8) {
        self.ui.borrow_mut().show_bottom_tab(t);
    }

    pub fn bottom_tab(&self) -> u8 {
        self.ui.borrow().bottom_tab()
    }

    pub fn focus_script(&self) {
        self.ui.borrow_mut().focus_field(ED_SCRIPT);
    }

    /// Where a control is in the window: (left, top, right, bottom).
    pub fn ctl_screen_rect(&self, id: u16) -> Option<(i32, i32, i32, i32)> {
        self.ui.borrow().ctl_rect(id).map(|r| (r.0 as i32, r.1 as i32, r.2 as i32, r.3 as i32))
    }

    /// Shows a context menu at a window position; the chosen command arrives through `take_commands`.
    pub fn open_popup(&self, items: &[MenuItem], x: i32, y: i32) {
        self.ui.borrow_mut().open_popup(items, x as f32, y as f32);
    }

    /// A compiled game: the 3D view takes the whole window and the editor interface is gone.
    pub fn set_game_mode(&self, on: bool) {
        let (w, h) = self.client();
        let mut ui = self.ui.borrow_mut();
        ui.game = on;
        ui.relayout(w, h);
    }

    pub fn set_playing(&self, on: bool) {
        PLAYING.set(on);
        let mut ui = self.ui.borrow_mut();
        ui.playing = on;
        if on {
            ui.blur();
        }
    }

    pub fn tree_clear(&self) {
        self.ui.borrow_mut().tree_clear();
    }

    /// Adds a checkbox item and returns its handle.
    pub fn tree_add(&self, parent: Option<isize>, text: &str, checked: bool) -> isize {
        self.ui.borrow_mut().tree_add(parent, text, checked)
    }

    pub fn tree_expand(&self, item: isize) {
        self.ui.borrow_mut().tree_expand(item);
    }

    pub fn tree_checked(&self, item: isize) -> bool {
        self.ui.borrow().tree_checked(item)
    }

    pub fn tree_set_checked(&self, item: isize, checked: bool) {
        self.ui.borrow_mut().tree_set_checked(item, checked);
    }

    /// Handle of the selected item, 0 if none.
    pub fn tree_selected(&self) -> isize {
        self.ui.borrow().tree_selected()
    }

    /// Bold text marks the items of a multi-selection.
    pub fn tree_set_bold(&self, item: isize, bold: bool) {
        self.ui.borrow_mut().tree_set_bold(item, bold);
    }

    pub fn tree_select(&self, item: isize) {
        self.ui.borrow_mut().tree_select(item);
    }

    /// A row of the hierarchy was dropped somewhere: (dragged item, target item, where). Items are handles.
    pub fn take_tree_drop(&self) -> Option<(isize, isize, DropAt)> {
        self.ui.borrow_mut().tree_drop.take().map(|(n, t, at)| (n as isize + 1, t as isize + 1, at))
    }

    /// Opens (or shows) the script tab of an object; `graph` is its visual script, if it has one.
    pub fn open_script_tab(&self, object: &str, code: &str, graph: Option<crate::graph::Graph>) {
        self.ui.borrow_mut().open_script_tab(object, code, graph);
    }

    pub fn update_script_tab(&self, object: &str, code: &str, graph: Option<crate::graph::Graph>) {
        self.ui.borrow_mut().update_script_tab(object, code, graph);
    }

    pub fn close_script_tab(&self, object: &str) {
        self.ui.borrow_mut().close_script_tab(object);
    }

    pub fn open_script_tabs(&self) -> Vec<String> {
        self.ui.borrow().open_script_tabs()
    }

    pub fn take_script_events(&self) -> Vec<crate::ui::ScriptEvent> {
        std::mem::take(&mut self.ui.borrow_mut().script_events)
    }

    /// The script button of a hierarchy row was clicked: the object's name.
    pub fn take_script_open(&self) -> Option<String> {
        self.ui.borrow_mut().script_open.take()
    }

    pub fn tree_set_badge(&self, item: isize, on: bool) {
        self.ui.borrow_mut().tree_set_badge(item, on);
    }

    pub fn explorer_clear(&self) {
        self.ui.borrow_mut().explorer_clear();
    }

    pub fn explorer_add(&self, name: &str, kind: &str, size: &str) {
        self.ui.borrow_mut().explorer_add(name, kind, size);
    }

    pub fn explorer_selected(&self) -> Option<usize> {
        self.ui.borrow().explorer_selected()
    }

    // ---------------------------------------------------------------- mouse look, sound
    pub fn look_begin(&self) {
        let mut p = POINT { x: 0, y: 0 };
        unsafe {
            GetCursorPos(&mut p);
            ShowCursor(0);
        }
        LOOKING.set(true);
        LOOK_ANCHOR.set((p.x, p.y));
    }

    /// Mouse movement since the last call; the cursor is recentred so it never hits a screen edge.
    pub fn look_delta(&self) -> (i32, i32) {
        let (ax, ay) = LOOK_ANCHOR.get();
        let mut p = POINT { x: ax, y: ay };
        unsafe {
            GetCursorPos(&mut p);
            SetCursorPos(ax, ay);
        }
        (p.x - ax, p.y - ay)
    }

    pub fn look_end(&self) {
        unsafe { ShowCursor(1) };
        LOOKING.set(false);
    }

    /// Where the cursor was when the mouse look began, in window coordinates.
    pub fn look_anchor(&self) -> (i32, i32) {
        let (ax, ay) = LOOK_ANCHOR.get();
        let mut p = POINT { x: ax, y: ay };
        unsafe { ScreenToClient(self.main, &mut p) };
        (p.x, p.y)
    }

    /// Plays a complete .wav file held in memory (asynchronously). The buffer must stay alive until
    /// the next call or [`Window::stop_sound`].
    pub fn play_sound(&self, wav: &[u8]) {
        unsafe { PlaySoundW(wav.as_ptr(), 0, 0x4 | 0x1 | 0x2) }; // SND_MEMORY | SND_ASYNC | SND_NODEFAULT
    }

    /// Plays a .wav file from disk (asynchronously).
    pub fn play_sound_file(&self, path: &str) {
        let w = wide(path);
        unsafe { PlaySoundW(w.as_ptr() as *const u8, 0, 0x2_0000 | 0x1 | 0x2) }; // SND_FILENAME | SND_ASYNC | SND_NODEFAULT
    }

    pub fn stop_sound(&self) {
        unsafe { PlaySoundW(std::ptr::null(), 0, 0) };
    }

    // ---------------------------------------------------------------- dialogs
    /// Runs a dialog over a dimmed copy of the last frame until it is done.
    fn run_dialog(&self, d: &mut dyn Dialog) {
        let mut clip = SysClip;
        // buttons released by the user while the dialog opens must not leak into the 3D view
        self.view_capture.set([false; 2]);
        update(|i| {
            i.left_down = false;
            i.right_down = false;
        });
        let mut last = std::time::Instant::now();
        loop {
            if !self.pump() {
                update(|i| i.quit = true);
                return;
            }
            let events: Vec<Raw> = RAW.with_borrow_mut(std::mem::take);
            for ev in events {
                match ev {
                    Raw::Move(x, y) => d.event(&DlgEvent::Move((x, y)), &mut clip),
                    Raw::Down(b, x, y, dbl) => d.event(&DlgEvent::Down(b, (x, y), dbl), &mut clip),
                    Raw::Up(b, x, y) => d.event(&DlgEvent::Up(b, (x, y)), &mut clip),
                    Raw::Wheel(n, x, y) => d.event(&DlgEvent::Wheel(n, (x, y)), &mut clip),
                    Raw::Key(vk, _) => d.event(&DlgEvent::Key(vk, Self::mods()), &mut clip),
                    Raw::Char(c) => d.event(&DlgEvent::Char(c), &mut clip),
                    Raw::Resize(w, h) => self.ui.borrow_mut().relayout(w, h),
                    Raw::Close => {
                        update(|i| i.quit = true);
                        return;
                    }
                    Raw::Leave => {}
                }
            }
            if d.done() {
                break;
            }
            let (w, h) = self.client();
            let mut rend = self.rend.borrow_mut();
            let Some(r) = rend.as_mut() else { return };
            let ui = self.ui.borrow();
            let Some(fonts) = ui.fonts.as_ref() else { return };
            r.begin(w as i32, h as i32);
            let snap = r.snapshot_texture();
            r.image(snap, 0.0, 0.0, w, h, true, [0.34, 0.34, 0.38, 1.0]);
            let mut c = Ctx { r, f: fonts, s: SCALE.get() };
            let (dw, dh) = d.size(&c);
            let y = if dh >= h - 1.0 { 0.0 } else { ((h - dh) * 0.45).floor().max(8.0) }; // a full-window dialog fills the window
            d.draw(&mut c, [((w - dw) * 0.5).floor(), y, dw, dh]);
            r.end();
            unsafe { SwapBuffers(self.dc) };
            // keep the loop from spinning when vsync is unavailable
            let spent = last.elapsed();
            if spent < std::time::Duration::from_millis(4) {
                std::thread::sleep(std::time::Duration::from_millis(4) - spent);
            }
            last = std::time::Instant::now();
        }
        // swallow the events of the closing click / key so they do not reach the editor
        RAW.with_borrow_mut(|r| r.clear());
    }

    /// Turns vsync on or off (off lets the frame rate run free).
    pub fn set_vsync(&self, on: bool) {
        unsafe {
            let swap = wglGetProcAddress(b"wglSwapIntervalEXT\0".as_ptr());
            if !swap.is_null() {
                let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(swap);
                f(on as i32);
            }
        }
    }

    /// The start page. None when it was closed (cancelled, or the window was closed).
    pub fn hub_dialog(&self, page: crate::ui::hub::Page, recents: Vec<crate::ui::hub::RecentItem>, cancellable: bool) -> Option<crate::ui::hub::HubChoice> {
        let mut d = crate::ui::hub::Hub::new(page, recents, cancellable);
        self.run_dialog(&mut d);
        d.result
    }

    /// True once the user asked to close the window.
    pub fn quit_requested(&self) -> bool {
        INPUT.get().quit
    }

    /// The game GUI editor; returns the edited elements when applied.
    pub fn gui_dialog(&self, elems: Vec<crate::gui::GuiElem>, mode: crate::gui::Mode) -> Option<(Vec<crate::gui::GuiElem>, crate::gui::Mode)> {
        let mut d = crate::ui::gui_dialog::GuiDialog::new(elems, mode);
        self.run_dialog(&mut d);
        let mode = d.mode;
        d.result.map(|e| (e, mode))
    }

    /// A settings-style dialog with a custom confirm button: (numbers and switches, texts) or None if cancelled.
    pub fn form_dialog(&self, title: &str, ok: &str, specs: Vec<ItemSpec>) -> Option<(Vec<f32>, Vec<String>)> {
        let mut d = SettingsDialog::new(title, specs).with_ok(ok);
        self.run_dialog(&mut d);
        let texts = d.texts();
        d.result.map(|v| (v, texts))
    }

    /// Borderless full screen on / off (the window comes back to where it was).
    pub fn set_fullscreen(&self, on: bool) {
        unsafe {
            if on && SAVED_FRAME.get().is_none() {
                let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetWindowRect(self.main, &mut r);
                SAVED_FRAME.set(Some((GetWindowLongPtrW(self.main, -16), (r.left, r.top, r.right - r.left, r.bottom - r.top))));
                SetWindowLongPtrW(self.main, -16, 0x8000_0000u32 as isize | 0x1000_0000); // WS_POPUP | WS_VISIBLE
                SetWindowPos(self.main, 0, 0, 0, GetSystemMetrics(0), GetSystemMetrics(1), 0x20 | 0x40); // FRAMECHANGED | SHOWWINDOW
            } else if !on {
                if let Some((style, (x, y, w, h))) = SAVED_FRAME.take() {
                    SetWindowLongPtrW(self.main, -16, style);
                    SetWindowPos(self.main, 0, x, y, w, h, 0x20 | 0x40);
                }
            }
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        SAVED_FRAME.with(|f| f.get().is_some())
    }

    /// Shows the settings dialog; the values come back in item order (None if cancelled).
    pub fn settings_dialog(&self, specs: Vec<ItemSpec>) -> Option<Vec<f32>> {
        let mut d = SettingsDialog::new("Engine settings", specs);
        self.run_dialog(&mut d);
        d.result
    }

    /// Open / Save dialog. `filters` are (description, pattern) pairs like ("Models", "*.blend;*.obj").
    pub fn file_dialog(&self, save: bool, filters: &[(&str, &str)], default_ext: &str, initial_name: &str) -> Option<String> {
        let start = LAST_DIR.with_borrow(|d| d.clone()).filter(|d| d.is_dir()).or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("C:\\"));
        let mut d = FileDialog::new(save, filters, default_ext, initial_name, &start);
        self.run_dialog(&mut d);
        LAST_DIR.with_borrow_mut(|l| *l = Some(d.dir().to_path_buf()));
        d.result.map(|p| p.to_string_lossy().into_owned())
    }

    /// Color picker; colors are 0..1 sRGB. None if cancelled.
    pub fn choose_color(&self, init: [f32; 3]) -> Option<[f32; 3]> {
        let mut d = ColorDialog::new(init);
        self.run_dialog(&mut d);
        d.result
    }
}

/// The system clipboard.
pub struct SysClip;

impl crate::ui::widgets::Clipboard for SysClip {
    fn get(&mut self) -> String {
        clipboard_get()
    }

    fn set(&mut self, text: &str) {
        clipboard_set(text);
    }
}

/// Text on the clipboard (empty when there is none).
pub fn clipboard_get() -> String {
    unsafe {
        let mut out = String::new();
        if OpenClipboard(MAIN.get()) != 0 {
            let h = GetClipboardData(13); // CF_UNICODETEXT
            if h != 0 {
                let p = GlobalLock(h) as *const u16;
                if !p.is_null() {
                    let mut n = 0;
                    while *p.add(n) != 0 {
                        n += 1;
                    }
                    out = String::from_utf16_lossy(std::slice::from_raw_parts(p, n)).replace("\r\n", "\n");
                    GlobalUnlock(h);
                }
            }
            CloseClipboard();
        }
        out
    }
}

pub fn clipboard_set(text: &str) {
    unsafe {
        let data: Vec<u16> = text.replace('\n', "\r\n").encode_utf16().chain(Some(0)).collect();
        if OpenClipboard(MAIN.get()) != 0 {
            EmptyClipboard();
            let mem = GlobalAlloc(0x0002, data.len() * 2); // GMEM_MOVEABLE
            if mem != 0 {
                let p = GlobalLock(mem) as *mut u16;
                if !p.is_null() {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
                    GlobalUnlock(mem);
                    SetClipboardData(13, mem);
                }
            }
            CloseClipboard();
        }
    }
}

fn update(f: impl FnOnce(&mut Input)) {
    let mut i = INPUT.get();
    f(&mut i);
    INPUT.set(i);
}

/// System DPI scale (1.0 = 96 dpi).
pub fn dpi_scale() -> f32 {
    SCALE.get()
}

/// True while the given virtual key is physically held (works regardless of focus).
pub fn key_down(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

/// Resolves a GL entry point: WGL for 1.2+, opengl32.dll exports for 1.0/1.1.
pub fn get_proc(name: &str) -> *const c_void {
    debug_assert!(name.ends_with('\0'));
    unsafe {
        let p = wglGetProcAddress(name.as_ptr());
        let v = p as usize;
        if v > 3 && v != usize::MAX {
            return p;
        }
        let lib = LoadLibraryA(b"opengl32.dll\0".as_ptr());
        GetProcAddress(lib, name.as_ptr())
    }
}
