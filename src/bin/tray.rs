//! palit-led-tray: windowless system-tray app that runs the temperature mode.
//! Raw Shell_NotifyIcon (no GUI framework). Right-click tray icon -> Exit.

#![windows_subsystem = "windows"]

use palit_led::config::{self, TempCfg};
use palit_led::led;
use palit_led::nvapi::NvApi;

use std::mem::size_of;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, KillTimer, LoadIconW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    TrackPopupMenu, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, IDI_APPLICATION,
    MF_STRING, MSG, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
const MENU_EXIT: usize = 1;

struct State {
    nv: NvApi,
    gpus: Vec<usize>,
    cfg: TempCfg,
    last: Vec<(u8, u8, u8)>,
    nid: NOTIFYICONDATAW,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
    let w = wide(text);
    let n = w.len().min(nid.szTip.len());
    nid.szTip[..n].copy_from_slice(&w[..n]);
}

unsafe fn update(state: &mut State) {
    let mut tips: Vec<String> = Vec::with_capacity(state.gpus.len());
    let t = &state.cfg;
    for (i, &g) in state.gpus.iter().enumerate() {
        let temp = state.nv.gpu_temp(g).unwrap_or(0);
        let c = led::temp_to_color(temp, t.green_max, t.yellow_max, t.orange_max, t.red_full, t.smooth);
        if c != state.last[i] {
            let _ = led::set_color(&state.nv, g, c.0, c.1, c.2, 100);
            state.last[i] = c;
        }
        tips.push(format!("GPU{g} {temp}C"));
    }
    set_tip(&mut state.nid, &format!("palit-led  {}", tips.join("  ")));
    state.nid.uFlags = NIF_TIP;
    Shell_NotifyIconW(NIM_MODIFY, &state.nid);
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    AppendMenuW(menu, MF_STRING, MENU_EXIT, wide("Exit").as_ptr());
    let mut p = POINT { x: 0, y: 0 };
    GetCursorPos(&mut p);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_RETURNCMD,
        p.x,
        p.y,
        0,
        hwnd,
        null_mut(),
    );
    DestroyMenu(menu);
    if cmd as usize == MENU_EXIT {
        DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = lp as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            0
        }
        WM_TIMER => {
            let s = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            if !s.is_null() {
                update(&mut *s);
            }
            0
        }
        WM_TRAY => {
            if (lp as u32) & 0xFFFF == WM_RBUTTONUP {
                show_menu(hwnd);
            }
            0
        }
        WM_COMMAND => {
            if (wp & 0xFFFF) as usize == MENU_EXIT {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            let s = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            if !s.is_null() {
                Shell_NotifyIconW(NIM_DELETE, &(*s).nid);
                drop(Box::from_raw(s));
            }
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    let cfg = config::load();
    let nv = match NvApi::new() {
        Ok(n) => n,
        Err(_) => return,
    };
    let count = nv.gpu_count();
    if count == 0 {
        return;
    }
    let gpus = cfg.gpus.resolve(count);
    let gpus = if gpus.is_empty() { vec![0] } else { gpus };
    let last = vec![(1u8, 1u8, 1u8); gpus.len()];

    unsafe {
        let hinst = GetModuleHandleW(null_mut());
        let class = wide("PalitLedTray");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null_mut(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);

        let mut state = Box::new(State { nv, gpus, cfg: cfg.temp, last, nid: std::mem::zeroed() });
        let state_ptr: *mut State = &mut *state;

        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("palit-led").as_ptr(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            null_mut(),
            null_mut(),
            hinst,
            state_ptr as *const _,
        );
        if hwnd.is_null() {
            return;
        }

        // tray icon
        state.nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        state.nid.hWnd = hwnd;
        state.nid.uID = 1;
        state.nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        state.nid.uCallbackMessage = WM_TRAY;
        state.nid.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
        set_tip(&mut state.nid, "palit-led temp");
        Shell_NotifyIconW(NIM_ADD, &state.nid);

        // hand ownership to the window; keep alive until WM_DESTROY
        std::mem::forget(state);

        update(&mut *state_ptr);
        let interval = (*state_ptr).cfg.interval_ms.max(200) as u32;
        SetTimer(hwnd, TIMER_ID, interval, None);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
