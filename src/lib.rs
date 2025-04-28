use std::u32;

// Import ErrorStrategy specifically
use napi::threadsafe_function::{ErrorStrategy, ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, JsFunction, Result, Task};
use napi_derive::napi;

// pull hotkey registration from the KeyboardAndMouse module:
use windows::Win32::UI::Input::KeyboardAndMouse::{
  HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey,
};
// pull message-loop pieces and WM_HOTKEY from WindowsAndMessaging:
use windows::Win32::UI::WindowsAndMessaging::{
  DispatchMessageW, GetMessageW, MSG, TranslateMessage, WM_HOTKEY,
};
// HWND is in Foundation
// use windows::Win32::Foundation::HWND;

#[napi]
pub enum Modifiers {
  Alt = 1,     // HOT_KEY_MODIFIERS::MOD_ALT.0 as isize,
  Control = 2, //HOT_KEY_MODIFIERS::MOD_CONTROL.0 as isize,
  Shift = 3,   // HOT_KEY_MODIFIERS::MOD_SHIFT.0 as isize,
  Win = 4,     //HOT_KEY_MODIFIERS::MOD_WIN.0 as isize,
}

/// background task that runs the Win32 message loop
struct HotkeyListener {
  hotkey_id: i32, // Use a specific ID for the hotkey
  mask: u32,
  vk: u32,
  // Use the imported ErrorStrategy
  tsfn: ThreadsafeFunction<(), ErrorStrategy::CalleeHandled>,
}

impl Task for HotkeyListener {
  type Output = ();
  type JsValue = ();

  fn compute(&mut self) -> Result<Self::Output> {
    // Register the hotkey globally (hwnd = None)
    // Pass None for the HWND parameter for global hotkeys.
    // Wrap the mask value in the HOT_KEY_MODIFIERS struct.
    let modifiers = HOT_KEY_MODIFIERS(self.mask);
    let success = unsafe { RegisterHotKey(None, self.hotkey_id, modifiers, self.vk) }.is_ok();

    if !success {
      // You might want to log this failure or return a proper Rust error
      // e.g., using windows::core::Error::from_win32()
      eprintln!(
        "Failed to register hotkey (ID: {} Modifiers: {:?}, VK: {})",
        self.hotkey_id, modifiers, self.vk
      );
      // Convert windows error to NAPI error if possible, or just return Ok() to stop the task
      // For simplicity, we just stop here. A robust app might signal failure back to JS.
      return Ok(());
    }
    println!("Hotkey registered successfully (ID: {})", self.hotkey_id); // Added for feedback

    let mut msg = MSG::default();
    // standard Win32 message loop; GetMessageW blocks until an event
    // Loop while GetMessageW returns a value > 0.
    // 0 indicates WM_QUIT, -1 indicates an error.
    loop {
      let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
      match result.0 {
        -1 => {
          // Handle error, perhaps log it
          eprintln!(
            "Error in GetMessageW: {:?}",
            windows::core::Error::from_win32()
          );
          break;
        }
        0 => {
          // Received WM_QUIT or the message queue was destroyed
          println!("GetMessageW returned 0, exiting loop.");
          break;
        }
        _ => {
          // Check if it's our hotkey message
          if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == self.hotkey_id {
            println!("Hotkey activated (ID: {})", self.hotkey_id); // Added for feedback
            // fire the JS callback
            // Use NonBlocking to avoid deadlocks if the JS callback takes time or calls back into Rust
            let status = self
              .tsfn
              .call(Ok(()), ThreadsafeFunctionCallMode::NonBlocking);
            if status != napi::Status::Ok {
              eprintln!("Failed to call JS callback: {:?}", status);
            }
          }
          // Standard message processing
          unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
          }
        }
      }
    }

    // Unregister the hotkey when the message loop exits
    // This is important to clean up resources
    let unregister_success = unsafe { UnregisterHotKey(None, self.hotkey_id) }.is_ok();
    if !unregister_success {
      eprintln!("Failed to unregister hotkey (ID: {})", self.hotkey_id);
    } else {
      println!("Hotkey unregistered successfully (ID: {})", self.hotkey_id); // Added for feedback
    }

    Ok(())
  }

  fn resolve(&mut self, _env: Env, _out: ()) -> Result<Self::JsValue> {
    // This is called on the main thread after `compute` finishes successfully.
    // We don't need to return anything specific to JS here.
    Ok(())
  }

  fn reject(&mut self, _env: Env, err: napi::Error) -> Result<Self::JsValue> {
    // This is called on the main thread if `compute` returns an Err.
    eprintln!("HotkeyListener task failed: {}", err);
    // We still need to attempt unregistration if registration might have succeeded before the error
    // Note: This runs on the main thread, unregistration might ideally happen on the worker thread
    //       before it exits if possible, but cleanup here is better than none.
    let _ = unsafe { UnregisterHotKey(None, self.hotkey_id) };
    Ok(()) // Resolve with undefined in JS even on error
  }
}

// Simple counter for unique hotkey IDs
static HOTKEY_ID_COUNTER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

#[napi]
pub fn register_hotkey(env: Env, modifier: Modifiers, vk: u32, callback: JsFunction) -> Result<()> {
  // create a threadsafe function. Use the imported ErrorStrategy::Fatal.
  // Fatal will terminate the node process if the callback throws an unhandled exception.
  // Consider using ErrorStrategy::CalleeHandled if you want JS to handle errors.
  let tsfn: ThreadsafeFunction<(), ErrorStrategy::CalleeHandled> = callback
    .create_threadsafe_function(0, |_ctx| {
      // This transforms the Rust () result into a JS value (undefined in this case)
      // The vec![] indicates no arguments are passed to the JS callback.
      // Ok(vec![])
      Ok::<Vec<napi::JsUnknown>, napi::Error>(vec![])
    })?;

  // Generate a unique ID for this hotkey registration
  let id = HOTKEY_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

  // Spawn the listener task on the libuv thread pool.
  // The `modifier` enum variant (e.g., Modifiers::Alt) comes from JS.
  // NAPI-RS converts it to its underlying numeric value (u32 in this case).
  env.spawn(HotkeyListener {
    hotkey_id: id,
    mask: modifier as u32, // Cast the enum variant to its u32 value
    vk,
    tsfn,
  })?;

  println!("Spawning HotkeyListener task (ID: {})", id); // Added for feedback
  Ok(())
}
