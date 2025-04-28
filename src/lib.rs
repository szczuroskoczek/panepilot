#![allow(unused_imports)] // Keep this for now if needed

use std::sync::{Arc, Mutex};
use std::thread;
use std::u32;

// Import ErrorStrategy specifically
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{
  CallContext, Env, Error as NapiError, JsFunction, JsObject, JsUndefined, NapiRaw, Result, Task,
}; // Added NapiRaw for JsFunction context
use napi_derive::napi;

// pull hotkey registration from the KeyboardAndMouse module:
use windows::Win32::UI::Input::KeyboardAndMouse::{
  HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey, VK_F24, VK_MENU,
};
// pull message-loop pieces and WM_HOTKEY from WindowsAndMessaging:
use windows::Win32::UI::WindowsAndMessaging::{
  DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT_FLAGS, MSG, PostThreadMessageW, TranslateMessage,
  WM_HOTKEY, WM_QUIT,
};
// Import necessary windows-rs types
use windows::core::Error as WinError;
use windows::core::Result as WinResult;

// Import web_view types
use web_view::{Content, Handle, WebView, builder}; // Keep Handle

use once_cell::sync::Lazy;
use std::ptr::null_mut;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
  CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
  WH_KEYBOARD_LL, WM_KEYUP, WM_SYSKEYUP,
};

use windows::core::PCWSTR;

#[napi]
pub enum Modifiers {
  Alt = 1,     // Corresponds to MOD_ALT (0x0001)
  Control = 2, // Corresponds to MOD_CONTROL (0x0002)
  Shift = 4,   // Corresponds to MOD_SHIFT (0x0004) - Corrected
  Win = 8,     // Corresponds to MOD_WIN (0x0008) - Corrected
}

// Helper to convert Modifiers enum to actual Win32 flags
fn modifiers_to_flags(modifier: Modifiers) -> u32 {
  match modifier {
    Modifiers::Alt => 0x0001,     // MOD_ALT
    Modifiers::Control => 0x0002, // MOD_CONTROL
    Modifiers::Shift => 0x0004,   // MOD_SHIFT
    Modifiers::Win => 0x0008,     // MOD_WIN
  }
}

/// background task that runs the Win32 message loop
struct HotkeyListener {
  hotkey_id: i32, // Use a specific ID for the hotkey
  mask: u32,      // Win32 modifier flags
  vk: u32,
  tsfn: ThreadsafeFunction<(), ErrorStrategy::CalleeHandled>,
  // Store the thread ID to post WM_QUIT later if needed (though NAPI handles task cancellation)
  // thread_id: u32, // Uncomment if manual thread termination is needed
}

impl Task for HotkeyListener {
  type Output = ();
  type JsValue = (); // Resolves to undefined in JS

  fn compute(&mut self) -> Result<Self::Output> {
    // Get the current thread ID if needed for WM_QUIT (optional)
    // self.thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };

    // Register the hotkey globally (hwnd = None)
    let modifiers = HOT_KEY_MODIFIERS(self.mask);
    // Use .is_ok() to check the Result<()> from RegisterHotKey
    let registration_result: WinResult<()> =
      unsafe { RegisterHotKey(None, self.hotkey_id, modifiers, self.vk) };

    if registration_result.is_err() {
      let error = WinError::from_win32(); // Get error info *after* failure
      eprintln!(
        "Failed to register hotkey (ID: {} Modifiers: {:?}, VK: {}): {:?}",
        self.hotkey_id, modifiers, self.vk, error
      );
      return Err(napi::Error::new(
        napi::Status::GenericFailure,
        format!("Failed to register hotkey: {}", error),
      ));
    }
    // println!("Hotkey registered successfully (ID: {})", self.hotkey_id);

    let mut msg = MSG::default();
    loop {
      // Blocking call
      // GetMessageW returns > 0 for messages, 0 for WM_QUIT, -1 for error.
      let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
      match result.0 {
        -1 => {
          let error = WinError::from_win32();
          eprintln!("Error in GetMessageW (ID: {}): {:?}", self.hotkey_id, error);
          break; // Exit loop on error
        }
        0 => {
          // Received WM_QUIT
          println!(
            "WM_QUIT received, exiting message loop (ID: {}).",
            self.hotkey_id
          );
          break; // Exit loop cleanly
        }
        _ => {
          // Check if it's our hotkey message
          // wParam for WM_HOTKEY is the hotkey ID (i32)
          if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == self.hotkey_id {
            // Call the JS callback via the threadsafe function
            let status = self
              .tsfn
              .call(Ok(()), ThreadsafeFunctionCallMode::NonBlocking);
            if status != napi::Status::Ok {
              eprintln!(
                "Failed to call JS callback (ID: {}): {:?}",
                self.hotkey_id, status
              );
              // Consider if the loop should break here depending on desired behavior
            }
          } else {
            // Only process other messages if necessary for this thread's function.
            // For a pure hotkey listener, this might not be needed unless
            // other windows/timers are created on this same thread.
            // It's generally safe to include them.
            unsafe {
              let _ = TranslateMessage(&msg);
              DispatchMessageW(&msg);
            }
          }
        }
      }
    }

    // --- Unregistration ---
    // Use .is_ok() to check the Result<()> from UnregisterHotKey
    let unregister_result: WinResult<()> = unsafe { UnregisterHotKey(None, self.hotkey_id) };
    if unregister_result.is_err() {
      let error = WinError::from_win32();
      eprintln!(
        "Failed to unregister hotkey (ID: {}): {:?}",
        self.hotkey_id, error
      );
      // Log error, maybe return an error if critical? Compute is about to finish anyway.
    } else {
      println!("Hotkey unregistered successfully (ID: {})", self.hotkey_id);
    }

    // --- Cleanup ---
    // No explicit abort needed. Relies on RAII: tsfn will be dropped when
    // the HotkeyListener instance is dropped after resolve/reject.

    Ok(())
  }

  fn resolve(&mut self, _env: Env, _output: Self::Output) -> Result<Self::JsValue> {
    // Called on the main thread if `compute` succeeds.
    Ok(()) // Resolves to undefined in JS
  }

  fn reject(&mut self, _env: Env, err: napi::Error) -> Result<Self::JsValue> {
    // Called on the main thread if `compute` returns an Err.
    eprintln!(
      "HotkeyListener task failed (ID: {}): {}",
      self.hotkey_id, err
    );
    // Attempt unregistration *just in case*. Safe if not registered.
    let _ = unsafe { UnregisterHotKey(None, self.hotkey_id) };
    // No explicit abort needed (RAII).
    Err(err) // Propagate the error so the JS Promise rejects
  }
}

// Simple counter for unique hotkey IDs (ensures different calls get different IDs)
static HOTKEY_ID_COUNTER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

#[napi]
pub fn register_hotkey(env: Env, modifier: Modifiers, vk: u32, callback: JsFunction) -> Result<()> {
  // Create a threadsafe function to call the JS callback from the listener thread.
  let tsfn: ThreadsafeFunction<(), ErrorStrategy::CalleeHandled> = callback
    .create_threadsafe_function(
      0,
      |ctx: napi::threadsafe_function::ThreadSafeCallContext<()>| {
        // Map the Rust () unit type to JS 'undefined'.
        // ctx.value is the () sent from the Rust side in tsfn.call(Ok(()), ...).
        // We return a Vec<JsValue> to be passed as arguments to the JS callback.
        Ok(vec![ctx.env.get_undefined()?]) // Send 'undefined' as the only argument
      },
    )?;

  // Generate a unique ID for this hotkey registration
  let id = HOTKEY_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
  // Get the correct Win32 modifier flags from the enum
  let modifier_flags = modifiers_to_flags(modifier);

  // Spawn the listener task on the libuv thread pool.
  env.spawn(HotkeyListener {
    hotkey_id: id,
    mask: modifier_flags,
    vk,
    tsfn, // Move the threadsafe function into the task
  })?; // This returns a Promise<void> in JS

  // println!(
  //   "Attempting to register hotkey (ID: {}, Modifiers: 0x{:X}, VK: 0x{:X}) and spawn listener task.",
  //   id, modifier_flags, vk
  // );
  Ok(()) // Return Ok(()) to indicate the spawning was successful (JS gets a Promise)
}

// --- WebView Section ---

type SharedHandle = Arc<Mutex<Option<Handle<()>>>>;

#[napi]
pub struct WebviewHandle {
  handle: SharedHandle,
}

#[napi]
impl WebviewHandle {
  #[napi]
  pub fn exit(&self) -> Result<()> {
    if let Some(handle) = self.handle.lock().unwrap().take() {
      let _ = handle.dispatch(|webview| {
        webview.exit();
        Ok(())
      });
    }
    Ok(())
  }

  #[napi]
  pub fn set_title(&self, title: String) -> Result<()> {
    if let Some(handle) = self.handle.lock().unwrap().clone() {
      let _ = handle.dispatch(move |webview| {
        let _ = webview.set_title(&title);
        Ok(())
      });
    }
    Ok(())
  }

  #[napi]
  pub fn set_visible(&self, visible: bool) -> Result<()> {
    if let Some(handle) = self.handle.lock().unwrap().clone() {
      let _ = handle.dispatch(move |webview| {
        let _ = webview.set_visible(visible);
        Ok(())
      });
    }
    Ok(())
  }
}

#[napi]
pub fn open_webview(title: String, width: i32, height: i32) -> Result<WebviewHandle> {
  let handle_store: SharedHandle = Arc::new(Mutex::new(None));
  let thread_store = handle_store.clone();

  thread::spawn(move || {
    let webview = builder()
      .title(&title)
      .content(Content::Html("<h1>Hello world!</h1>"))
      .size(width, height)
      .resizable(false)
      .frameless(true)
      .debug(false)
      .user_data(())
      .invoke_handler(|_webview, _arg| Ok(()))
      .visible(false)
      .build()
      .unwrap();

    let handle = webview.handle();
    *thread_store.lock().unwrap() = Some(handle.clone());

    webview.run().unwrap();
  });

  Ok(WebviewHandle {
    handle: handle_store,
  })
}

#[derive(Copy, Clone)]
struct SafeHhook(HHOOK);
unsafe impl Send for SafeHhook {}
unsafe impl Sync for SafeHhook {}

static HOOK_HANDLE: Lazy<Mutex<Option<SafeHhook>>> = Lazy::new(|| Mutex::new(None));
static CALLBACK: Lazy<Mutex<Option<ThreadsafeFunction<(), ErrorStrategy::CalleeHandled>>>> =
  Lazy::new(|| Mutex::new(None));
static HOOK_THREAD_ID: Lazy<Mutex<Option<u32>>> = Lazy::new(|| Mutex::new(None));

extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
  unsafe {
    if code == HC_ACTION as i32 && wparam.0 as u32 == 257 {
      let kb = *(lparam.0 as *const KBDLLHOOKSTRUCT);
      if kb.vkCode == 164 {
        // fire callback once
        if let Some(tsfn) = CALLBACK.lock().unwrap().take() {
          let _ = tsfn.call(Ok(()), ThreadsafeFunctionCallMode::NonBlocking);
        }

        // unhook
        if let Some(SafeHhook(h)) = HOOK_HANDLE.lock().unwrap().take() {
          let _ = UnhookWindowsHookEx(h);
        }
        // signal thread to exit
        if let Some(tid) = HOOK_THREAD_ID.lock().unwrap().take() {
          let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
        }
      }
    }
    CallNextHookEx(None, code, wparam, lparam)
  }
}

#[napi]
pub fn register_alt_release(_env: Env, callback: JsFunction) -> Result<()> {
  // prevent double registration
  if HOOK_HANDLE.lock().unwrap().is_some() {
    return Err(NapiError::from_reason(
      "Hook already registered".to_string(),
    ));
  }
  if HOOK_THREAD_ID.lock().unwrap().is_some() {
    return Err(NapiError::from_reason(
      "Hook thread already running".to_string(),
    ));
  }

  let tsfn = callback.create_threadsafe_function(0, |ctx| Ok(vec![ctx.env.get_undefined()?]))?;
  *CALLBACK.lock().unwrap() = Some(tsfn);

  thread::spawn(move || unsafe {
    let tid = GetCurrentThreadId();
    *HOOK_THREAD_ID.lock().unwrap() = Some(tid);
    let hmod = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
    let hook = SetWindowsHookExW(
      WH_KEYBOARD_LL,
      Some(keyboard_proc),
      Some(HINSTANCE(hmod.0)),
      0,
    )
    .unwrap_or_else(|e| panic!("SetWindowsHookExW failed: {:?}", e));
    *HOOK_HANDLE.lock().unwrap() = Some(SafeHhook(hook));

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).into() {
      let _ = TranslateMessage(&msg);
      DispatchMessageW(&msg);
    }
  });

  Ok(())
}
